//! The three-dimensional solver and the binary layout it shares with
//! `solid.wgsl`.

use std::borrow::Cow;

use glam::{DMat3, DVec3};
use rap_materials::EquationOfState;
use rap_solver::{Model3, Particle3, Solver3, SolverError};

use crate::engine::{Engine, Program, gpu_error, workgroups};
use crate::layout::{
    self, Clock, bits, encode_materials, index, real, reference_wave_speed, wall_code,
};

/// Words per `Particle` in the shader.
const PARTICLE_WORDS: usize = 36;
/// Words per node and body: velocity and time-centred velocity, each
/// padded to four words.
const NODE_WORDS: u32 = 8;
/// Invocations per workgroup of the `grid` pass along each axis.
const GRID_GROUP: u32 = 4;

/// The shader of the three-dimensional solver.
fn source() -> String {
    [include_str!("common.wgsl"), include_str!("solid.wgsl")].concat()
}

/// A vector followed by the scalar that shares its four words.
fn with(vector: DVec3, scalar: u32) -> [u32; 4] {
    [bits(vector.x), bits(vector.y), bits(vector.z), scalar]
}

/// The shader's `Params`.
fn encode_params(model: &Model3) -> Vec<u32> {
    let [nodes_x, nodes_y, nodes_z] = model.grid.nodes().map(index);
    let (lower, upper) = model.grid.interior();
    let config = &model.config;
    let mut words = vec![nodes_x, nodes_y, nodes_z, index(model.body_count())];
    words.extend(with(lower, index(model.seeds.len())));
    words.extend(with(upper, bits(model.grid.cell_size)));
    words.extend(
        [
            config.cfl,
            config.linear_viscosity,
            config.quadratic_viscosity,
            config.friction,
        ]
        .map(bits),
    );
    for walls in [model.walls.low, model.walls.high] {
        words.extend(walls.map(wall_code));
        words.push(0);
    }
    words
}

/// The shader's `array<Particle>`, every particle in its reference state.
fn encode_particles(model: &Model3) -> Vec<u32> {
    let mut words = Vec::with_capacity(model.seeds.len() * PARTICLE_WORDS);
    for seed in &model.seeds {
        let material = &model.materials[seed.material];
        let mass = material.eos.reference_density() * seed.volume;
        words.extend(with(seed.position, bits(mass)));
        words.extend(with(seed.velocity, bits(seed.volume)));
        // Velocity gradient with pressure, viscous pressure and specific
        // energy; deviatoric stress with plastic strain and its rate.
        words.extend([0; 20]);
        words.extend([
            bits(model.initial_temperature),
            // Damage.
            0,
            bits(reference_wave_speed(material)),
            index(seed.body),
            index(seed.material),
            layout::ACTIVE,
            0,
            0,
        ]);
    }
    words
}

fn vector(words: &[u32]) -> DVec3 {
    DVec3::new(real(words[0]), real(words[1]), real(words[2]))
}

/// Decodes one shader `Particle` and its status.
fn decode_particle(words: &[u32]) -> (Particle3, u32) {
    let normal = vector(&words[20..]);
    let shear = vector(&words[24..]);
    let particle = Particle3 {
        position: vector(words),
        velocity: vector(&words[4..]),
        velocity_gradient: DMat3::from_cols(
            vector(&words[8..]),
            vector(&words[12..]),
            vector(&words[16..]),
        ),
        mass: real(words[3]),
        volume: real(words[7]),
        body: words[31] as usize,
        material: words[32] as usize,
        deviator: DMat3::from_cols(
            DVec3::new(normal.x, shear.x, shear.y),
            DVec3::new(shear.x, normal.y, shear.z),
            DVec3::new(shear.y, shear.z, normal.z),
        ),
        pressure: real(words[11]),
        viscous_pressure: real(words[15]),
        specific_energy: real(words[19]),
        plastic_strain: real(words[23]),
        plastic_strain_rate: real(words[27]),
        temperature: real(words[28]),
        damage: real(words[29]),
        sound_speed: real(words[30]),
    };
    (particle, words[33])
}

/// `model` as the device sees it.
fn program(model: &Model3) -> Result<Program, SolverError> {
    let count = |value: usize| u32::try_from(value).map_err(gpu_error);
    let [nodes_x, nodes_y, nodes_z] = model.grid.nodes();
    let nodes = [count(nodes_x)?, count(nodes_y)?, count(nodes_z)?];
    let fastest = model
        .seeds
        .iter()
        .map(|seed| reference_wave_speed(&model.materials[seed.material]) + seed.velocity.length())
        .fold(0.0, f64::max);
    Ok(Program {
        label: "solid.wgsl",
        source: source(),
        params: encode_params(model),
        materials: encode_materials(&model.materials, model.config.detonation)?,
        particles: encode_particles(model),
        clock: Clock::starting_with(fastest),
        node_count: count(model.grid.node_count())?,
        body_count: count(model.body_count())?,
        node_words: NODE_WORDS,
        rows: nodes[1] * nodes[2],
        particle_count: count(model.seeds.len())?,
        grid_workgroups: nodes.map(|nodes| workgroups(nodes, GRID_GROUP)),
    })
}

/// Explicit MLS-MPM solver for three-dimensional models, running on the
/// GPU.
#[derive(Debug)]
pub struct GpuSolver3 {
    engine: Engine,
    particles: Vec<Particle3>,
    departed: Vec<Particle3>,
}

impl GpuSolver3 {
    /// Creates a solver for `model` with every particle in its material's
    /// reference state.
    ///
    /// # Errors
    /// Returns [`SolverError::Model`] if the model is inconsistent,
    /// [`SolverError::NoGpu`] if no suitable graphics adapter exists and
    /// [`SolverError::Gpu`] if the model does not fit the adapter's limits
    /// or uses a material model the GPU solver does not implement.
    pub fn new(model: &Model3) -> Result<Self, SolverError> {
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

impl Solver3 for GpuSolver3 {
    fn time(&self) -> f64 {
        self.engine.time()
    }

    fn step(&mut self, max_time_step: f64) -> Result<f64, SolverError> {
        let before = self.time();
        self.advance(before + max_time_step, Some(1))?;
        Ok(self.time() - before)
    }

    fn particles(&self) -> Cow<'_, [Particle3]> {
        Cow::Borrowed(&self.particles)
    }

    fn departed(&self) -> Cow<'_, [Particle3]> {
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
    use glam::DVec3;
    use rap_materials::library;
    use rap_solver::{Grid3, Model3, ParticleSeed3, SolverConfig, Wall, Walls3};

    use super::{PARTICLE_WORDS, decode_particle, encode_params, encode_particles, source};
    use crate::layout::ACTIVE;

    fn model() -> Model3 {
        Model3 {
            grid: Grid3 {
                cells: [10, 8, 6],
                cell_size: 1.0e-3,
            },
            walls: Walls3 {
                low: [Wall::Outflow, Wall::Frictionless, Wall::Sliding],
                high: [Wall::Outflow, Wall::Sticky, Wall::Outflow],
            },
            config: SolverConfig::default(),
            materials: vec![library::tungsten_heavy_alloy(), library::steel_4340()],
            seeds: vec![
                ParticleSeed3 {
                    position: DVec3::new(3.0e-3, 4.0e-3, 3.0e-3),
                    velocity: DVec3::new(1500.0, 0.0, -20.0),
                    volume: 1.25e-10,
                    material: 0,
                    body: 0,
                },
                ParticleSeed3 {
                    position: DVec3::new(6.0e-3, 3.0e-3, 3.5e-3),
                    velocity: DVec3::ZERO,
                    volume: 2.5e-10,
                    material: 1,
                    body: 1,
                },
            ],
            initial_temperature: 294.0,
        }
    }

    #[test]
    fn shader_is_valid() {
        crate::tests::validate(&source());
    }

    #[test]
    fn params_carry_grid_and_walls() {
        let words = encode_params(&model());
        // `Params` is 96 bytes in WGSL.
        assert_eq!(words.len() * 4, 96);
        // Nodes, bodies; the particle count shares a row with the corner.
        assert_eq!(&words[..4], &[11, 9, 7, 2]);
        assert_eq!(words[7], 2);
        assert!((f32::from_bits(words[4]) - 2.0e-3).abs() < 1e-9);
        assert!((f32::from_bits(words[10]) - 4.0e-3).abs() < 1e-9);
        assert!((f32::from_bits(words[11]) - 1.0e-3).abs() < 1e-9);
        // Walls at the low and at the high sides.
        assert_eq!(&words[16..19], &[3, 0, 1]);
        assert_eq!(&words[20..23], &[3, 2, 3]);
    }

    #[test]
    fn particles_survive_encoding_to_single_precision() {
        let model = model();
        let words = encode_particles(&model);
        // `Particle` is 144 bytes in WGSL.
        assert_eq!(words.len(), 2 * PARTICLE_WORDS);
        assert_eq!(PARTICLE_WORDS * 4, 144);
        let (first, status) = decode_particle(&words);
        assert_eq!(status, ACTIVE);
        assert!((first.velocity - DVec3::new(1500.0, 0.0, -20.0)).length() < 1e-3);
        let (second, status) = decode_particle(&words[PARTICLE_WORDS..]);
        assert_eq!(status, ACTIVE);
        assert_eq!((second.body, second.material), (1, 1));
        assert!((second.position - DVec3::new(6.0e-3, 3.0e-3, 3.5e-3)).length() < 1e-9);
        assert!((second.mass / (7830.0 * 2.5e-10) - 1.0).abs() < 1e-6);
        assert!((second.temperature - 294.0).abs() < 1e-4);
        assert!(second.sound_speed > 5000.0);
    }

    #[test]
    fn stress_components_land_where_the_shader_keeps_them() {
        let mut words = encode_particles(&model());
        // (xx, yy, zz) then (xy, xz, yz), with scalars in between.
        for (offset, value) in [20, 21, 22, 24, 25, 26].into_iter().zip(1_u8..) {
            words[offset] = f32::from(value).to_bits();
        }
        words[23] = 0.25_f32.to_bits();
        let (particle, _) = decode_particle(&words);
        let deviator = particle.deviator;
        assert_eq!(
            (deviator.x_axis.x, deviator.y_axis.y, deviator.z_axis.z),
            (1.0, 2.0, 3.0)
        );
        assert_eq!((deviator.y_axis.x, deviator.x_axis.y), (4.0, 4.0));
        assert_eq!((deviator.z_axis.x, deviator.x_axis.z), (5.0, 5.0));
        assert_eq!((deviator.z_axis.y, deviator.y_axis.z), (6.0, 6.0));
        assert!((particle.plastic_strain - 0.25).abs() < f64::EPSILON);
    }
}
