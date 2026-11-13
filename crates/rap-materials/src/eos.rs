//! Equations of state.

use rap_domain::validation::{self, InvalidParameter};
use serde::{Deserialize, Serialize};

use crate::EquationOfState;

/// Mie–Grüneisen equation of state referenced to a linear shock Hugoniot,
/// `U_s = c0 + s·u_p`, with `Γ·ρ` held constant.
///
/// With `μ = ρ/ρ0 − 1`, in compression (`μ > 0`):
///
/// ```text
/// p = ρ0·c0²·μ·[1 + (1 − Γ0/2)·μ] / [1 − (s − 1)·μ]² + Γ0·ρ0·e
/// ```
///
/// and in expansion (`μ ≤ 0`): `p = ρ0·c0²·μ + Γ0·ρ0·e`.
///
/// The compression branch is singular at `μ = 1/(s − 1)`; states that close
/// to the limit are outside the validity of a linear Hugoniot.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MieGruneisen {
    /// Density at the reference state, `ρ0`, in kg/m³.
    pub reference_density: f64,
    /// Bulk sound speed at the reference state, `c0`, in m/s.
    pub bulk_sound_speed: f64,
    /// Slope `s` of the shock-velocity versus particle-velocity line.
    pub hugoniot_slope: f64,
    /// Grüneisen parameter at the reference state, `Γ0`.
    pub gruneisen_gamma: f64,
}

impl MieGruneisen {
    /// Checks the parameters.
    ///
    /// # Errors
    /// Returns [`InvalidParameter`] naming the first invalid parameter.
    pub fn validate(&self) -> Result<(), InvalidParameter> {
        validation::positive("eos.reference_density", self.reference_density)?;
        validation::positive("eos.bulk_sound_speed", self.bulk_sound_speed)?;
        validation::non_negative("eos.hugoniot_slope", self.hugoniot_slope)?;
        validation::non_negative("eos.gruneisen_gamma", self.gruneisen_gamma)
    }

    /// Energy-independent part of the pressure and its derivative with
    /// respect to `μ`.
    fn cold_pressure(&self, mu: f64) -> (f64, f64) {
        let bulk_modulus = self.reference_density * self.bulk_sound_speed.powi(2);
        if mu <= 0.0 {
            return (bulk_modulus * mu, bulk_modulus);
        }
        let a = self.hugoniot_slope - 1.0;
        let b = 1.0 - self.gruneisen_gamma / 2.0;
        let denominator = 1.0 - a * mu;
        let numerator = mu * (1.0 + b * mu);
        let pressure = bulk_modulus * numerator / denominator.powi(2);
        let derivative = bulk_modulus * ((1.0 + 2.0 * b * mu) * denominator + 2.0 * a * numerator)
            / denominator.powi(3);
        (pressure, derivative)
    }

    fn compression(&self, density: f64) -> f64 {
        density / self.reference_density - 1.0
    }
}

impl EquationOfState for MieGruneisen {
    fn reference_density(&self) -> f64 {
        self.reference_density
    }

    fn pressure(&self, density: f64, specific_energy: f64) -> f64 {
        let (cold, _) = self.cold_pressure(self.compression(density));
        cold + self.gruneisen_gamma * self.reference_density * specific_energy
    }

    fn sound_speed(&self, density: f64, specific_energy: f64) -> f64 {
        // c² = (∂p/∂ρ)_e + (p/ρ²)·(∂p/∂e)_ρ
        let (cold, cold_derivative) = self.cold_pressure(self.compression(density));
        let energy_derivative = self.gruneisen_gamma * self.reference_density;
        let pressure = cold + energy_derivative * specific_energy;
        let squared = cold_derivative / self.reference_density
            + pressure * energy_derivative / density.powi(2);
        squared.max(0.0).sqrt()
    }
}

/// The built-in equations of state.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
#[serde(tag = "model", rename_all = "snake_case")]
pub enum Eos {
    /// See [`MieGruneisen`].
    MieGruneisen(MieGruneisen),
}

impl Eos {
    /// Checks the parameters of the wrapped model.
    ///
    /// # Errors
    /// Returns [`InvalidParameter`] naming the first invalid parameter.
    pub fn validate(&self) -> Result<(), InvalidParameter> {
        match self {
            Self::MieGruneisen(model) => model.validate(),
        }
    }
}

impl EquationOfState for Eos {
    fn reference_density(&self) -> f64 {
        match self {
            Self::MieGruneisen(model) => model.reference_density(),
        }
    }

    fn pressure(&self, density: f64, specific_energy: f64) -> f64 {
        match self {
            Self::MieGruneisen(model) => model.pressure(density, specific_energy),
        }
    }

    fn sound_speed(&self, density: f64, specific_energy: f64) -> f64 {
        match self {
            Self::MieGruneisen(model) => model.sound_speed(density, specific_energy),
        }
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::MieGruneisen;
    use crate::EquationOfState;

    const STEEL: MieGruneisen = MieGruneisen {
        reference_density: 7830.0,
        bulk_sound_speed: 4569.0,
        hugoniot_slope: 1.49,
        gruneisen_gamma: 2.17,
    };

    fn relative_error(actual: f64, expected: f64) -> f64 {
        ((actual - expected) / expected).abs()
    }

    #[test]
    fn reference_state_has_zero_pressure_and_reference_sound_speed() {
        assert!(STEEL.pressure(STEEL.reference_density, 0.0).abs() < 1e-6);
        let speed = STEEL.sound_speed(STEEL.reference_density, 0.0);
        assert!(relative_error(speed, STEEL.bulk_sound_speed) < 1e-12);
    }

    #[test]
    fn pressure_matches_the_hugoniot_it_is_built_on() {
        // Rankine–Hugoniot jump conditions for U_s = c0 + s·u_p.
        for particle_velocity in [100.0, 500.0, 1000.0, 1500.0] {
            let shock_velocity = STEEL.bulk_sound_speed + STEEL.hugoniot_slope * particle_velocity;
            let density =
                STEEL.reference_density * shock_velocity / (shock_velocity - particle_velocity);
            let hugoniot_pressure = STEEL.reference_density * shock_velocity * particle_velocity;
            let hugoniot_energy = particle_velocity.powi(2) / 2.0;

            let pressure = STEEL.pressure(density, hugoniot_energy);
            assert!(
                relative_error(pressure, hugoniot_pressure) < 1e-10,
                "u_p = {particle_velocity}: {pressure} vs {hugoniot_pressure}"
            );
        }
    }

    #[test]
    fn expansion_is_linear_in_compression() {
        let density = 0.98 * STEEL.reference_density;
        let expected = STEEL.reference_density * STEEL.bulk_sound_speed.powi(2) * -0.02;
        assert!(relative_error(STEEL.pressure(density, 0.0), expected) < 1e-10);
    }

    #[test]
    fn sound_speed_matches_isentropic_finite_difference() {
        for (density, energy) in [
            (7600.0, 0.0),
            (7830.0, 5.0e4),
            (8600.0, 1.0e5),
            (9500.0, 4.0e5),
        ] {
            let pressure = STEEL.pressure(density, energy);
            let step = 1.0e-3;
            // Along an isentrope de = (p/ρ²)·dρ.
            let energy_step = pressure / (density * density) * step;
            let derivative = (STEEL.pressure(density + step, energy + energy_step)
                - STEEL.pressure(density - step, energy - energy_step))
                / (2.0 * step);
            let speed = STEEL.sound_speed(density, energy);
            assert!(
                relative_error(speed, derivative.sqrt()) < 1e-6,
                "ρ = {density}, e = {energy}: {speed} vs {}",
                derivative.sqrt()
            );
        }
    }

    #[test]
    fn invalid_parameters_are_rejected() {
        assert_eq!(STEEL.validate(), Ok(()));
        let broken = MieGruneisen {
            bulk_sound_speed: 0.0,
            ..STEEL
        };
        assert_eq!(broken.validate().unwrap_err().field, "eos.bulk_sound_speed");
    }

    proptest! {
        #[test]
        fn pressure_increases_with_density(compression in -0.2..0.3_f64, step in 1e-4..1e-2_f64) {
            let lower = STEEL.reference_density * (1.0 + compression);
            let upper = STEEL.reference_density * (1.0 + compression + step);
            prop_assert!(STEEL.pressure(upper, 0.0) > STEEL.pressure(lower, 0.0));
        }

        #[test]
        fn pressure_increases_with_energy(compression in -0.2..0.3_f64, energy in 0.0..1e6_f64) {
            let density = STEEL.reference_density * (1.0 + compression);
            prop_assert!(STEEL.pressure(density, energy + 1.0) > STEEL.pressure(density, energy));
        }
    }
}
