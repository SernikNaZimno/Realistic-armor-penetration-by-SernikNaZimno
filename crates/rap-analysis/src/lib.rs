//! Post-processing: residual velocity, penetration depth, crater profile, energy balance.

use glam::DVec2;
use rap_solver::Particle;

/// Conserved quantities summed over a set of particles, per unit depth.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Totals {
    /// Mass, in kg/m.
    pub mass: f64,
    /// Linear momentum, in kg·m/s per metre.
    pub momentum: DVec2,
    /// Kinetic energy, in J/m.
    pub kinetic_energy: f64,
    /// Internal energy above the reference state, in J/m.
    pub internal_energy: f64,
}

impl Totals {
    /// Sums the conserved quantities of `particles`.
    #[must_use]
    pub fn of(particles: &[Particle]) -> Self {
        let zero = Self {
            mass: 0.0,
            momentum: DVec2::ZERO,
            kinetic_energy: 0.0,
            internal_energy: 0.0,
        };
        particles.iter().fold(zero, |sum, particle| Self {
            mass: sum.mass + particle.mass,
            momentum: sum.momentum + particle.mass * particle.velocity,
            kinetic_energy: sum.kinetic_energy
                + 0.5 * particle.mass * particle.velocity.length_squared(),
            internal_energy: sum.internal_energy + particle.mass * particle.specific_energy,
        })
    }

    /// Kinetic plus internal energy, in J/m.
    #[must_use]
    pub fn total_energy(&self) -> f64 {
        self.kinetic_energy + self.internal_energy
    }
}

/// Lower and upper corners of the box containing all particle centres, or
/// `None` if there are no particles.
#[must_use]
pub fn bounding_box(particles: &[Particle]) -> Option<(DVec2, DVec2)> {
    particles
        .iter()
        .map(|particle| particle.position)
        .fold(None, |bounds, position| {
            let (lower, upper) = bounds.unwrap_or((position, position));
            Some((lower.min(position), upper.max(position)))
        })
}
