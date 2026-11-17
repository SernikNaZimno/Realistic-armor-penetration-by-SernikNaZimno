//! Single-precision MPM solvers running in wgpu compute shaders.
//!
//! The scheme is the one of the CPU reference solvers, ported to WGSL:
//! `flat.wgsl` for two-dimensional models ([`GpuSolver`]), `solid.wgsl` for
//! three-dimensional ones ([`GpuSolver3`]), and `common.wgsl` for the
//! material models and the time step they share. The whole state lives on
//! the GPU; it is read back only when a run or step returns, so `run_until`
//! is the efficient way to advance. Arithmetic is `f32`, so results agree
//! with the CPU solvers to single-precision accuracy rather than bit for
//! bit.

mod engine;
mod layout;
mod solid;

use std::borrow::Cow;

use rap_solver::{Model, Particle, Solver, SolverError};

use crate::engine::{Engine, Program, gpu_error, workgroups};
use crate::layout::{
    Clock, PARTICLE_WORDS, decode_particle, encode_materials, encode_params, encode_particles,
    fastest_signal,
};
pub use crate::solid::GpuSolver3;

/// Invocations per workgroup of the `grid` pass along each axis.
const GRID_GROUP: u32 = 8;
/// Words per node and body: velocity and time-centred velocity.
const NODE_WORDS: u32 = 4;

/// The shader of the two-dimensional solver.
fn source() -> String {
    [include_str!("common.wgsl"), include_str!("flat.wgsl")].concat()
}

/// `model` as the device sees it.
fn program(model: &Model) -> Result<Program, SolverError> {
    let count = |value: usize| u32::try_from(value).map_err(gpu_error);
    let [nodes_x, nodes_y] = model.grid.nodes();
    let (nodes_x, nodes_y) = (count(nodes_x)?, count(nodes_y)?);
    Ok(Program {
        label: "flat.wgsl",
        source: source(),
        params: encode_params(model),
        materials: encode_materials(&model.materials)?,
        particles: encode_particles(model),
        clock: Clock::starting_with(fastest_signal(model)),
        node_count: count(model.grid.node_count())?,
        body_count: count(model.body_count())?,
        node_words: NODE_WORDS,
        rows: nodes_y,
        particle_count: count(model.seeds.len())?,
        grid_workgroups: [
            workgroups(nodes_x, GRID_GROUP),
            workgroups(nodes_y, GRID_GROUP),
            1,
        ],
    })
}

/// Explicit MLS-MPM solver for two-dimensional models, running on the GPU.
#[derive(Debug)]
pub struct GpuSolver {
    engine: Engine,
    particles: Vec<Particle>,
    departed: Vec<Particle>,
}

impl GpuSolver {
    /// Creates a solver for `model` with every particle in its material's
    /// reference state.
    ///
    /// # Errors
    /// Returns [`SolverError::Model`] if the model is inconsistent,
    /// [`SolverError::NoGpu`] if no suitable graphics adapter exists and
    /// [`SolverError::Gpu`] if the model does not fit the adapter's limits
    /// or uses a material model the GPU solver does not implement.
    pub fn new(model: &Model) -> Result<Self, SolverError> {
        model.validate()?;
        let mut solver = Self {
            engine: Engine::new(&program(model)?)?,
            particles: Vec::new(),
            departed: Vec::new(),
        };
        solver.read_particles()?;
        Ok(solver)
    }

    /// Name of the graphics adapter the solver runs on.
    #[must_use]
    pub fn adapter_name(&self) -> &str {
        self.engine.adapter_name()
    }

    /// Refreshes the CPU copy of the particles from the GPU.
    fn read_particles(&mut self) -> Result<(), SolverError> {
        let words = self.engine.read_particles()?;
        self.particles.clear();
        self.departed.clear();
        for chunk in words.chunks_exact(PARTICLE_WORDS) {
            let (particle, status) = decode_particle(chunk);
            match status {
                layout::ACTIVE => self.particles.push(particle),
                layout::DEPARTED => self.departed.push(particle),
                _ => {}
            }
        }
        Ok(())
    }

    /// Runs the simulation on the GPU until its clock reaches `end_time`,
    /// taking at most `max_steps` steps.
    fn advance(&mut self, end_time: f64, max_steps: Option<u32>) -> Result<u32, SolverError> {
        let outcome = self.engine.advance(end_time, max_steps);
        // Also after a failure, so that the state it failed in can be seen.
        self.read_particles()?;
        outcome
    }
}

impl Solver for GpuSolver {
    fn time(&self) -> f64 {
        self.engine.time()
    }

    fn step(&mut self, max_time_step: f64) -> Result<f64, SolverError> {
        let before = self.time();
        self.advance(before + max_time_step, Some(1))?;
        Ok(self.time() - before)
    }

    fn particles(&self) -> Cow<'_, [Particle]> {
        Cow::Borrowed(&self.particles)
    }

    fn departed(&self) -> Cow<'_, [Particle]> {
        Cow::Borrowed(&self.departed)
    }

    fn run_until(&mut self, end_time: f64) -> Result<usize, SolverError> {
        if end_time <= self.time() {
            return Ok(0);
        }
        let steps = self.advance(end_time, None)?;
        Ok(usize::try_from(steps).unwrap_or(usize::MAX))
    }
}

#[cfg(test)]
mod tests {
    /// Parses and validates a shader the way the device would, without a
    /// device.
    pub(crate) fn validate(source: &str) {
        let module = match wgpu::naga::front::wgsl::parse_str(source) {
            Ok(module) => module,
            Err(error) => panic!("{}", error.emit_to_string(source)),
        };
        let mut validator = wgpu::naga::valid::Validator::new(
            wgpu::naga::valid::ValidationFlags::all(),
            wgpu::naga::valid::Capabilities::empty(),
        );
        if let Err(error) = validator.validate(&module) {
            panic!("{}", error.emit_to_string(source));
        }
    }

    #[test]
    fn two_dimensional_shader_is_valid() {
        validate(&super::source());
    }
}
