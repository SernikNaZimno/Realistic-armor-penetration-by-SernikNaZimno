//! Outcome of an impact measured along the shot line.

use rap_solver::{Particle, Particle3};
use serde::Serialize;

/// Fraction of the projectile mass that must emerge behind the armor for
/// the armor to count as perforated.
const PERFORATION_MASS_FRACTION: f64 = 0.01;
/// Fraction of the impact speed that material behind the armor must keep
/// for the armor to count as perforated. A penetrator that only bulges the
/// rear face ends up behind the back plane too, but at rest.
const PERFORATION_SPEED_FRACTION: f64 = 0.05;

/// Where the shot line crosses the armor and which bodies take part. The
/// shot line runs along `x`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShotLine {
    /// `x` at which the shot line enters the armor, in m.
    pub armor_face: f64,
    /// `x` at which the shot line leaves the armor, in m.
    pub armor_back: f64,
    /// Body index of the penetrator.
    pub projectile_body: usize,
    /// Body index of the armor.
    pub armor_body: usize,
    /// Mass of the penetrator before impact, in the units of the particles.
    pub initial_projectile_mass: f64,
    /// Speed of the penetrator at impact, in m/s.
    pub impact_speed: f64,
}

/// What the impact summary needs to know about one particle, whatever the
/// dimension of its model.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tracer {
    /// Position along the shot line, in m.
    pub along: f64,
    /// Velocity along the shot line, in m/s.
    pub speed: f64,
    /// Mass, in the units of the model.
    pub mass: f64,
    /// Body the particle belongs to.
    pub body: usize,
    /// Whether the material point has failed.
    pub failed: bool,
}

impl From<&Particle> for Tracer {
    fn from(particle: &Particle) -> Self {
        Self {
            along: particle.position.x,
            speed: particle.velocity.x,
            mass: particle.mass,
            body: particle.body,
            failed: particle.is_failed(),
        }
    }
}

impl From<&Particle3> for Tracer {
    fn from(particle: &Particle3) -> Self {
        Self {
            along: particle.position.x,
            speed: particle.velocity.x,
            mass: particle.mass,
            body: particle.body,
            failed: particle.is_failed(),
        }
    }
}

/// State of an impact at one instant.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct ImpactSummary {
    /// Simulation time, in s.
    pub time: f64,
    /// Distance the penetrator has advanced past the armor face along the
    /// shot line, limited to the armor path length, in m.
    pub penetration_depth: f64,
    /// Length of the shot line inside the armor, gaps included, in m.
    pub armor_path_length: f64,
    /// Whether a meaningful part of the penetrator has passed the armor and
    /// is still moving on.
    pub perforated: bool,
    /// Mean speed along the shot line of the penetrator material behind the
    /// armor, in m/s; zero if there is none.
    pub residual_speed: f64,
    /// Fraction of the penetrator mass behind the armor.
    pub residual_mass_fraction: f64,
    /// Fraction of the penetrator mass that has failed.
    pub projectile_failed_fraction: f64,
    /// Fraction of the armor mass that has failed.
    pub armor_failed_fraction: f64,
}

/// Failed fraction of the mass of a body.
fn failed_fraction(tracers: &[Tracer], body: usize) -> f64 {
    let (failed, total) = tracers.iter().filter(|tracer| tracer.body == body).fold(
        (0.0, 0.0),
        |(failed, total), tracer| {
            let failed_mass = if tracer.failed { tracer.mass } else { 0.0 };
            (failed + failed_mass, total + tracer.mass)
        },
    );
    if total > 0.0 { failed / total } else { 0.0 }
}

/// Measures the state of an impact from `tracers` at `time`.
#[must_use]
pub fn summarize_tracers(tracers: &[Tracer], shot: &ShotLine, time: f64) -> ImpactSummary {
    let armor_path_length = shot.armor_back - shot.armor_face;
    let projectile = || {
        tracers
            .iter()
            .filter(|tracer| tracer.body == shot.projectile_body)
    };

    let deepest = projectile()
        .map(|tracer| tracer.along)
        .fold(f64::MIN, f64::max);
    let (residual_mass, residual_momentum) = projectile()
        .filter(|tracer| tracer.along > shot.armor_back)
        .fold((0.0, 0.0), |(mass, momentum), tracer| {
            (mass + tracer.mass, momentum + tracer.mass * tracer.speed)
        });
    let residual_mass_fraction = residual_mass / shot.initial_projectile_mass;
    let residual_speed = if residual_mass > 0.0 {
        residual_momentum / residual_mass
    } else {
        0.0
    };

    ImpactSummary {
        time,
        penetration_depth: (deepest - shot.armor_face).clamp(0.0, armor_path_length),
        armor_path_length,
        perforated: residual_mass_fraction > PERFORATION_MASS_FRACTION
            && residual_speed > PERFORATION_SPEED_FRACTION * shot.impact_speed,
        residual_speed,
        residual_mass_fraction,
        projectile_failed_fraction: failed_fraction(tracers, shot.projectile_body),
        armor_failed_fraction: failed_fraction(tracers, shot.armor_body),
    }
}

/// Measures the state of an impact from the particles of a two-dimensional
/// model at `time`.
#[must_use]
pub fn summarize(particles: &[Particle], shot: &ShotLine, time: f64) -> ImpactSummary {
    let tracers: Vec<Tracer> = particles.iter().map(Tracer::from).collect();
    summarize_tracers(&tracers, shot, time)
}

#[cfg(test)]
mod tests {
    use super::{ShotLine, Tracer, summarize_tracers};

    const SHOT: ShotLine = ShotLine {
        armor_face: 0.10,
        armor_back: 0.15,
        projectile_body: 0,
        armor_body: 1,
        initial_projectile_mass: 4.0,
        impact_speed: 1000.0,
    };

    fn tracer(along: f64, speed: f64, body: usize, failed: bool) -> Tracer {
        Tracer {
            along,
            speed,
            mass: 1.0,
            body,
            failed,
        }
    }

    #[test]
    fn projectile_short_of_the_armor_has_not_penetrated() {
        let tracers = [tracer(0.05, 1000.0, 0, false), tracer(0.12, 0.0, 1, false)];
        let summary = summarize_tracers(&tracers, &SHOT, 1.0e-6);
        assert!(summary.penetration_depth.abs() < f64::EPSILON);
        assert!(!summary.perforated);
        assert!(summary.residual_speed.abs() < f64::EPSILON);
        assert!((summary.armor_path_length - 0.05).abs() < 1e-12);
    }

    #[test]
    fn depth_is_measured_from_the_face_to_the_deepest_projectile_point() {
        let tracers = [
            tracer(0.08, 600.0, 0, false),
            tracer(0.13, 300.0, 0, true),
            tracer(0.14, 0.0, 1, true),
            tracer(0.11, 0.0, 1, false),
        ];
        let summary = summarize_tracers(&tracers, &SHOT, 5.0e-5);
        assert!((summary.penetration_depth - 0.03).abs() < 1e-12);
        assert!(!summary.perforated);
        assert!((summary.projectile_failed_fraction - 0.5).abs() < 1e-12);
        assert!((summary.armor_failed_fraction - 0.5).abs() < 1e-12);
    }

    #[test]
    fn material_behind_the_armor_counts_as_residual() {
        let tracers = [
            tracer(0.12, 100.0, 0, true),
            tracer(0.16, 700.0, 0, false),
            tracer(0.18, 900.0, 0, false),
            tracer(0.16, 400.0, 1, true),
        ];
        let summary = summarize_tracers(&tracers, &SHOT, 1.0e-4);
        assert!(summary.perforated);
        assert!((summary.residual_speed - 800.0).abs() < 1e-9);
        assert!((summary.residual_mass_fraction - 0.5).abs() < 1e-12);
        // The depth never exceeds the armor path.
        assert!((summary.penetration_depth - 0.05).abs() < 1e-12);
    }

    #[test]
    fn projectile_at_rest_in_a_rear_bulge_has_not_perforated() {
        let tracers = [
            tracer(0.14, 0.0, 0, false),
            tracer(0.155, 10.0, 0, false),
            tracer(0.16, 5.0, 1, false),
        ];
        let summary = summarize_tracers(&tracers, &SHOT, 3.0e-4);
        assert!((summary.residual_mass_fraction - 0.25).abs() < 1e-12);
        assert!(!summary.perforated);
    }
}
