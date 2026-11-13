//! Solver abstraction shared by all numerical back ends.
//!
//! A [`Model`] describes a problem; a [`Solver`] advances it in time and
//! exposes the state as [`Particle`]s. Back ends (CPU, GPU) implement
//! [`Solver`] and are otherwise interchangeable.
//!
//! The current formulation is two-dimensional plane strain: all fields are
//! per unit depth and nothing varies along `z`.

pub mod model;
pub mod particle;

use std::borrow::Cow;

use thiserror::Error;

pub use model::{Grid, Model, ModelError, ParticleSeed, SolverConfig, WALL_CELLS, Wall, Walls};
pub use particle::{Deviator, Particle};

/// Reasons a simulation cannot start or continue.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum SolverError {
    /// The model handed to the solver is inconsistent.
    #[error("invalid model: {0}")]
    Model(#[from] ModelError),
    /// A particle moved out of the computational grid.
    #[error("particle {particle} left the grid at t = {time:.3e} s")]
    ParticleLeftGrid {
        /// Index of the particle.
        particle: usize,
        /// Simulation time at which it was detected.
        time: f64,
    },
    /// A particle acquired a non-finite position, usually a sign of
    /// numerical instability.
    #[error("particle {particle} has a non-finite state at t = {time:.3e} s")]
    NonFiniteState {
        /// Index of the particle.
        particle: usize,
        /// Simulation time at which it was detected.
        time: f64,
    },
}

/// A numerical back end that advances a [`Model`] in time.
pub trait Solver {
    /// Simulation time reached so far, in seconds.
    fn time(&self) -> f64;

    /// Advances by one stable time step, shortened to `max_time_step` if
    /// necessary, and returns the step taken, in seconds.
    ///
    /// # Errors
    /// Returns [`SolverError`] if the state has become invalid.
    fn step(&mut self, max_time_step: f64) -> Result<f64, SolverError>;

    /// Current state of every particle. Back ends that keep their state
    /// elsewhere (for example on a GPU) return an owned copy.
    fn particles(&self) -> Cow<'_, [Particle]>;

    /// Steps until `end_time` is reached and returns the number of steps.
    ///
    /// # Errors
    /// Returns [`SolverError`] if the state has become invalid.
    fn run_until(&mut self, end_time: f64) -> Result<usize, SolverError> {
        let mut steps = 0;
        while self.time() < end_time {
            self.step(end_time - self.time())?;
            steps += 1;
        }
        Ok(steps)
    }
}
