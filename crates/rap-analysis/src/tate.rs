//! Alekseevskii–Tate model of long-rod penetration.
//!
//! The rod is a rigid tail feeding a thin eroding zone at the crater bottom.
//! Along the axis the modified Bernoulli equation balances rod and target:
//!
//! ```text
//! ½·ρ_p·(v − u)² + Y_p = ½·ρ_t·u² + R_t
//! ```
//!
//! where `v` is the tail speed, `u` the penetration speed, `Y_p` the rod
//! strength and `R_t` the target resistance. The tail is decelerated by the
//! rod strength, `ρ_p·ℓ·dv/dt = −Y_p`, and shortens at `dℓ/dt = −(v − u)`.
//!
//! Three regimes follow from the balance:
//!
//! * eroding penetration, described above;
//! * if `R_t > Y_p` and the rod is slow, the target does not yield (`u = 0`)
//!   and the rod erodes against its surface;
//! * if `Y_p > R_t` and the rod is slow, the rod does not yield (`u = v`)
//!   and penetrates as a rigid body, `ρ_p·ℓ·dv/dt = −(R_t + ½·ρ_t·v²)`.

/// Inputs of the Alekseevskii–Tate model, in SI units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TateRod {
    /// Rod density `ρ_p`, in kg/m³.
    pub rod_density: f64,
    /// Rod strength `Y_p`, in Pa.
    pub rod_strength: f64,
    /// Initial rod length, in m.
    pub rod_length: f64,
    /// Target density `ρ_t`, in kg/m³.
    pub target_density: f64,
    /// Target resistance `R_t`, in Pa.
    pub target_resistance: f64,
    /// Impact speed, in m/s.
    pub impact_speed: f64,
}

/// Result of integrating the Alekseevskii–Tate model until the rod stops or
/// is consumed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TatePenetration {
    /// Final penetration depth into a semi-infinite target, in m.
    pub depth: f64,
    /// Length of rod left, in m.
    pub residual_length: f64,
    /// Time from impact to the end of penetration, in s.
    pub duration: f64,
}

/// Number of integration steps per rod length travelled at impact speed.
const STEPS_PER_LENGTH: f64 = 20_000.0;
/// The rod counts as consumed or stopped below these fractions of its
/// initial length and speed.
const NEGLIGIBLE: f64 = 1.0e-6;

impl TateRod {
    /// Penetration speed `u` for a tail speed `v`, from the modified
    /// Bernoulli equation, limited to the physical range `[0, v]`.
    #[must_use]
    pub fn penetration_speed(&self, tail_speed: f64) -> f64 {
        let (rod, target) = (self.rod_density, self.target_density);
        let excess = self.target_resistance - self.rod_strength;
        let speed = if (rod - target).abs() < 1.0e-9 * rod {
            tail_speed / 2.0 - excess / (rod * tail_speed)
        } else {
            let discriminant = rod * target * tail_speed.powi(2) + 2.0 * excess * (rod - target);
            (rod * tail_speed - discriminant.max(0.0).sqrt()) / (rod - target)
        };
        speed.clamp(0.0, tail_speed)
    }

    /// Integrates the model from impact until the rod stops or is consumed.
    #[must_use]
    pub fn penetrate(&self) -> TatePenetration {
        let time_step = self.rod_length / self.impact_speed / STEPS_PER_LENGTH;
        let (mut speed, mut length) = (self.impact_speed, self.rod_length);
        let (mut depth, mut time) = (0.0, 0.0);
        while speed > NEGLIGIBLE * self.impact_speed && length > NEGLIGIBLE * self.rod_length {
            let penetration_speed = self.penetration_speed(speed);
            let rigid = penetration_speed >= speed;
            let decelerating_stress = if rigid {
                self.target_resistance + 0.5 * self.target_density * speed.powi(2)
            } else {
                self.rod_strength
            };
            if decelerating_stress <= 0.0 && rigid {
                // Nothing resists a rigid rod; the depth would be unbounded.
                break;
            }
            depth += time_step * penetration_speed;
            length -= time_step * (speed - penetration_speed);
            speed -= time_step * decelerating_stress
                / (self.rod_density * length.max(f64::MIN_POSITIVE));
            time += time_step;
        }
        TatePenetration {
            depth,
            residual_length: length.max(0.0),
            duration: time,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::TateRod;

    const TUNGSTEN_INTO_STEEL: TateRod = TateRod {
        rod_density: 17_000.0,
        rod_strength: 1.9e9,
        rod_length: 0.1,
        target_density: 7830.0,
        target_resistance: 4.6e9,
        impact_speed: 1500.0,
    };

    #[test]
    fn strengthless_materials_give_the_hydrodynamic_limit() {
        let rod = TateRod {
            rod_strength: 0.0,
            target_resistance: 0.0,
            ..TUNGSTEN_INTO_STEEL
        };
        let result = rod.penetrate();
        let expected = rod.rod_length * (rod.rod_density / rod.target_density).sqrt();
        assert!(
            (result.depth / expected - 1.0).abs() < 1e-3,
            "{}",
            result.depth
        );
        assert!(result.residual_length < 1e-5 * rod.rod_length);
    }

    #[test]
    fn penetration_speed_satisfies_the_bernoulli_balance() {
        let rod = TUNGSTEN_INTO_STEEL;
        let tail = 1500.0;
        let penetration = rod.penetration_speed(tail);
        let rod_side = 0.5 * rod.rod_density * (tail - penetration).powi(2) + rod.rod_strength;
        let target_side = 0.5 * rod.target_density * penetration.powi(2) + rod.target_resistance;
        assert!((rod_side / target_side - 1.0).abs() < 1e-12);
        assert!((penetration - 743.7).abs() < 0.5, "{penetration}");
    }

    #[test]
    fn equal_densities_use_the_linear_balance() {
        let rod = TateRod {
            rod_density: 7830.0,
            ..TUNGSTEN_INTO_STEEL
        };
        let penetration = rod.penetration_speed(2000.0);
        let rod_side = 0.5 * 7830.0 * (2000.0 - penetration).powi(2) + rod.rod_strength;
        let target_side = 0.5 * 7830.0 * penetration.powi(2) + rod.target_resistance;
        assert!((rod_side / target_side - 1.0).abs() < 1e-12);
    }

    #[test]
    fn slow_rod_erodes_without_penetrating_a_stronger_target() {
        // Below sqrt(2·(R_t − Y_p)/ρ_p) = 564 m/s the target does not yield.
        let rod = TateRod {
            impact_speed: 500.0,
            ..TUNGSTEN_INTO_STEEL
        };
        let result = rod.penetrate();
        assert!(result.depth.abs() < f64::EPSILON);
        assert!(result.residual_length < rod.rod_length);
    }

    #[test]
    fn slow_strong_rod_penetrates_as_a_rigid_body() {
        let rod = TateRod {
            rod_strength: 6.0e9,
            target_resistance: 1.0e9,
            impact_speed: 300.0,
            ..TUNGSTEN_INTO_STEEL
        };
        let result = rod.penetrate();
        assert!((result.residual_length / rod.rod_length - 1.0).abs() < 1e-9);
        // ρ_p·L·v·dv/dx = −(R_t + ½·ρ_t·v²) integrates to
        // x = ρ_p·L/ρ_t · ln(1 + ρ_t·v₀²/(2·R_t)).
        let expected = rod.rod_density * rod.rod_length / rod.target_density
            * (1.0 + rod.target_density * 300.0_f64.powi(2) / (2.0 * rod.target_resistance)).ln();
        assert!(
            (result.depth / expected - 1.0).abs() < 1e-3,
            "{} vs {expected}",
            result.depth
        );
    }

    #[test]
    fn depth_grows_with_impact_speed_towards_the_hydrodynamic_limit() {
        let depth = |impact_speed| {
            TateRod {
                impact_speed,
                ..TUNGSTEN_INTO_STEEL
            }
            .penetrate()
            .depth
        };
        let limit = 0.1 * (17_000.0_f64 / 7830.0).sqrt();
        let depths = [1000.0, 1500.0, 2000.0, 3000.0].map(depth);
        assert!(depths.is_sorted());
        assert!(depths[0] > 0.0 && depths[3] < 1.15 * limit, "{depths:?}");
        assert!(depths[3] > 0.85 * limit, "{depths:?}");
    }
}
