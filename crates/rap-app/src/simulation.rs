//! A running simulation of one scenario.

use std::borrow::Cow;

use glam::{DMat2, DVec2};
use rap_analysis::{ImpactSummary, ShotLine, Tracer, summarize_tracers};
use rap_domain::{Backend, Scenario, SimulationSettings, Target};
use rap_geometry::TriangleMesh;
use rap_materials::Material;
use rap_solver::{Deviator, Particle, Particle3, Solver, Solver3};
use rap_solver_cpu::{MpmSolver, MpmSolver3};
use rap_solver_gpu::GpuSolver;

use crate::AppError;
use crate::assemble::mesh::assemble_mesh;
use crate::assemble::{Layout, ModelKind, assemble, assemble_solid};

/// The solver behind a simulation.
enum Engine {
    /// A two-dimensional model.
    Flat(Box<dyn Solver + Send>),
    /// A three-dimensional model.
    Solid(Box<dyn Solver3 + Send>),
}

/// One scenario being simulated.
pub struct Simulation {
    engine: Engine,
    solver_name: String,
    layout: Layout,
    shot: ShotLine,
}

/// The particle of the two-dimensional picture that shows a particle of a
/// solid model lying in its symmetry plane.
fn in_plane(particle: &Particle3) -> Particle {
    let gradient = particle.velocity_gradient;
    Particle {
        position: particle.position.truncate(),
        velocity: particle.velocity.truncate(),
        velocity_gradient: DMat2::from_cols(
            DVec2::new(gradient.x_axis.x, gradient.x_axis.y),
            DVec2::new(gradient.y_axis.x, gradient.y_axis.y),
        ),
        mass: particle.mass,
        volume: particle.volume,
        body: particle.body,
        material: particle.material,
        deviator: Deviator {
            xx: particle.deviator.x_axis.x,
            yy: particle.deviator.y_axis.y,
            xy: particle.deviator.y_axis.x,
        },
        pressure: particle.pressure,
        viscous_pressure: particle.viscous_pressure,
        specific_energy: particle.specific_energy,
        plastic_strain: particle.plastic_strain,
        plastic_strain_rate: particle.plastic_strain_rate,
        temperature: particle.temperature,
        damage: particle.damage,
        sound_speed: particle.sound_speed,
    }
}

impl Simulation {
    /// Prepares the simulation of `scenario` on the solver chosen in
    /// `settings`; `custom_materials` are searched before the built-in
    /// library.
    ///
    /// # Errors
    /// Returns [`AppError`] if the scenario cannot be turned into a model
    /// or the chosen solver cannot be started. A missing GPU is an error;
    /// the CPU solver is never substituted silently. Armor given by a mesh
    /// is an error here; use [`Self::with_mesh`].
    pub fn new(
        scenario: &Scenario,
        settings: &SimulationSettings,
        custom_materials: &[Material],
    ) -> Result<Self, AppError> {
        Self::with_mesh(scenario, settings, custom_materials, None)
    }

    /// Prepares the simulation of `scenario` like [`Self::new`]; `mesh` is
    /// the solid its armor refers to, if the armor is given by one. See
    /// [`crate::load_mesh`].
    ///
    /// # Errors
    /// Returns [`AppError`] if the scenario cannot be turned into a model
    /// or the chosen solver cannot be started, and
    /// [`AppError::MeshRequired`] if the armor needs a mesh and there is
    /// none.
    pub fn with_mesh(
        scenario: &Scenario,
        settings: &SimulationSettings,
        custom_materials: &[Material],
        mesh: Option<&TriangleMesh>,
    ) -> Result<Self, AppError> {
        let (engine, solver_name, layout) = match ModelKind::of(scenario, settings) {
            ModelKind::Solid => {
                if settings.backend == Backend::Gpu {
                    return Err(AppError::SolidOnGpu);
                }
                let assembly = match (&scenario.armor, mesh) {
                    (Target::Plates(_), _) => assemble_solid(scenario, settings, custom_materials)?,
                    (_, Some(mesh)) => assemble_mesh(scenario, settings, custom_materials, mesh)?,
                    (_, None) => return Err(AppError::MeshRequired),
                };
                let solver = MpmSolver3::new(assembly.model)?;
                (
                    Engine::Solid(Box::new(solver)),
                    "CPU".to_owned(),
                    assembly.layout,
                )
            }
            ModelKind::Axisymmetric | ModelKind::PlaneStrain => {
                let assembly = assemble(scenario, settings, custom_materials)?;
                let (solver, name): (Box<dyn Solver + Send>, String) = match settings.backend {
                    Backend::Cpu => (Box::new(MpmSolver::new(assembly.model)?), "CPU".to_owned()),
                    Backend::Gpu => {
                        let solver = GpuSolver::new(&assembly.model)?;
                        let name = format!("GPU ({})", solver.adapter_name());
                        (Box::new(solver), name)
                    }
                };
                (Engine::Flat(solver), name, assembly.layout)
            }
        };
        let mut simulation = Self {
            engine,
            solver_name,
            layout,
            shot: ShotLine {
                armor_face: layout.armor_face,
                armor_back: layout.armor_back,
                projectile_body: layout.projectile_body,
                armor_body: layout.armor_body,
                initial_projectile_mass: 0.0,
                impact_speed: scenario.impact_speed,
            },
        };
        simulation.shot.initial_projectile_mass = simulation
            .tracers()
            .iter()
            .filter(|tracer| tracer.body == layout.projectile_body)
            .map(|tracer| tracer.mass)
            .sum();
        Ok(simulation)
    }

    /// Where the projectile and armor are in the model.
    #[must_use]
    pub fn layout(&self) -> &Layout {
        &self.layout
    }

    /// Description of the solver the simulation runs on.
    #[must_use]
    pub fn solver_name(&self) -> &str {
        &self.solver_name
    }

    /// Number of particles still being simulated.
    #[must_use]
    pub fn particle_count(&self) -> usize {
        match &self.engine {
            Engine::Flat(solver) => solver.particles().len(),
            Engine::Solid(solver) => solver.particles().len(),
        }
    }

    /// Simulation time reached so far, in s.
    #[must_use]
    pub fn time(&self) -> f64 {
        match &self.engine {
            Engine::Flat(solver) => solver.time(),
            Engine::Solid(solver) => solver.time(),
        }
    }

    /// Whether the time set in the layout has been reached.
    #[must_use]
    pub fn is_finished(&self) -> bool {
        self.time() >= self.layout.end_time
    }

    /// Advances the simulation to `time` and returns the number of steps.
    ///
    /// # Errors
    /// Returns [`AppError::Solver`] if the solver state has become invalid.
    pub fn advance_to(&mut self, time: f64) -> Result<usize, AppError> {
        Ok(match &mut self.engine {
            Engine::Flat(solver) => solver.run_until(time)?,
            Engine::Solid(solver) => solver.run_until(time)?,
        })
    }

    /// The particles of a solid model that lie just above the plane its
    /// picture shows, as a two-dimensional picture.
    fn cross_section(&self, particles: &[Particle3]) -> Vec<Particle> {
        let plane = self.layout.symmetry_plane;
        let slice = plane..plane + self.layout.cell_size / 2.0;
        particles
            .iter()
            .filter(|particle| slice.contains(&particle.position.z))
            .map(in_plane)
            .collect()
    }

    /// Two-dimensional picture of the particles still being simulated: the
    /// model itself, or for a solid model its cross-section in the
    /// symmetry plane.
    #[must_use]
    pub fn particles(&self) -> Cow<'_, [Particle]> {
        match &self.engine {
            Engine::Flat(solver) => solver.particles(),
            Engine::Solid(solver) => Cow::Owned(self.cross_section(&solver.particles())),
        }
    }

    /// Two-dimensional picture of the particles that flew out of the
    /// simulated region, in the state they left in.
    #[must_use]
    pub fn departed(&self) -> Cow<'_, [Particle]> {
        match &self.engine {
            Engine::Flat(solver) => solver.departed(),
            Engine::Solid(solver) => Cow::Owned(self.cross_section(&solver.departed())),
        }
    }

    /// All particles of a solid model still being simulated; `None` for a
    /// two-dimensional model.
    #[must_use]
    pub fn solid_particles(&self) -> Option<Cow<'_, [Particle3]>> {
        match &self.engine {
            Engine::Flat(_) => None,
            Engine::Solid(solver) => Some(solver.particles()),
        }
    }

    /// Every particle, simulated or departed, reduced to what the impact
    /// summary needs.
    fn tracers(&self) -> Vec<Tracer> {
        match &self.engine {
            Engine::Flat(solver) => solver
                .particles()
                .iter()
                .chain(solver.departed().iter())
                .map(Tracer::from)
                .collect(),
            Engine::Solid(solver) => solver
                .particles()
                .iter()
                .chain(solver.departed().iter())
                .map(Tracer::from)
                .collect(),
        }
    }

    /// Outcome of the impact so far.
    #[must_use]
    pub fn summary(&self) -> ImpactSummary {
        summarize_tracers(&self.tracers(), &self.shot, self.time())
    }
}
