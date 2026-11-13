//! A running simulation of one scenario.

use std::borrow::Cow;

use rap_analysis::{ImpactSummary, ShotLine, summarize};
use rap_domain::{Scenario, SimulationSettings};
use rap_materials::Material;
use rap_solver::{Particle, Solver};
use rap_solver_cpu::MpmSolver;

use crate::AppError;
use crate::assemble::{Layout, assemble};

/// One scenario being simulated.
#[derive(Debug, Clone)]
pub struct Simulation {
    solver: MpmSolver,
    layout: Layout,
    shot: ShotLine,
}

impl Simulation {
    /// Prepares the simulation of `scenario`; `custom_materials` are
    /// searched before the built-in library.
    ///
    /// # Errors
    /// Returns [`AppError`] if the scenario cannot be turned into a model.
    pub fn new(
        scenario: &Scenario,
        settings: &SimulationSettings,
        custom_materials: &[Material],
    ) -> Result<Self, AppError> {
        let assembly = assemble(scenario, settings, custom_materials)?;
        let layout = assembly.layout;
        let solver = MpmSolver::new(assembly.model)?;
        let initial_projectile_mass = solver
            .particles()
            .iter()
            .filter(|particle| particle.body == layout.projectile_body)
            .map(|particle| particle.mass)
            .sum();
        let shot = ShotLine {
            armor_face: layout.armor_face,
            armor_back: layout.armor_back,
            projectile_body: layout.projectile_body,
            armor_body: layout.armor_body,
            initial_projectile_mass,
            impact_speed: scenario.impact_speed,
        };
        Ok(Self {
            solver,
            layout,
            shot,
        })
    }

    /// Where the projectile and armor are in the model.
    #[must_use]
    pub fn layout(&self) -> &Layout {
        &self.layout
    }

    /// Simulation time reached so far, in s.
    #[must_use]
    pub fn time(&self) -> f64 {
        self.solver.time()
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
        Ok(self.solver.run_until(time)?)
    }

    /// Current state of every particle still being simulated.
    #[must_use]
    pub fn particles(&self) -> Cow<'_, [Particle]> {
        self.solver.particles()
    }

    /// Particles that flew out of the simulated region, in the state they
    /// left in.
    #[must_use]
    pub fn departed(&self) -> Cow<'_, [Particle]> {
        self.solver.departed()
    }

    /// Outcome of the impact so far.
    #[must_use]
    pub fn summary(&self) -> ImpactSummary {
        let mut particles = self.solver.particles().into_owned();
        particles.extend_from_slice(&self.solver.departed());
        summarize(&particles, &self.shot, self.time())
    }
}
