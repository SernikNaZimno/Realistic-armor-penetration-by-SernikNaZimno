//! Johnson–Holmquist model (JH-2) for brittle materials such as ceramics.
//!
//! Strength depends on pressure and falls from an intact to a fractured
//! curve as damage accumulates. In terms of the pressure and tensile
//! strength normalised by the pressure at the Hugoniot elastic limit,
//! `P* = P/P_HEL` and `T* = T/P_HEL`, and with stresses normalised by the
//! equivalent stress at that limit, `σ_HEL`:
//!
//! ```text
//! intact:     σ_i* = A·(P* + T*)ᴺ · (1 + C·ln ε̇*)
//! fractured:  σ_f* = B·(P*)ᴹ · (1 + C·ln ε̇*)   (at most SFMAX)
//! current:    σ*   = σ_i* − D·(σ_i* − σ_f*)
//! ```
//!
//! Damage grows with plastic strain against a fracture strain that rises
//! with pressure, `ε_f = D1·(P* + T*)^D2`. The bulking pressure of the
//! original model, which turns part of the energy released by fracture
//! into pressure, is not included.

use rap_domain::validation::{self, InvalidParameter};
use serde::{Deserialize, Serialize};

use crate::{DamageModel, PlasticState, StrengthModel};

/// Smallest fracture strain reported, so that damage increments stay finite
/// at and beyond the tensile limit.
const MIN_FAILURE_STRAIN: f64 = 1.0e-6;

/// JH-2 strength.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JohnsonHolmquistStrength {
    /// Elastic shear modulus, in Pa.
    pub shear_modulus: f64,
    /// Equivalent stress at the Hugoniot elastic limit, `σ_HEL`, in Pa.
    pub hel_stress: f64,
    /// Pressure at the Hugoniot elastic limit, `P_HEL`, in Pa.
    pub hel_pressure: f64,
    /// Maximum hydrostatic tension the intact material carries, `T`, in Pa.
    pub tensile_strength: f64,
    /// Intact strength coefficient `A`.
    pub intact_coefficient: f64,
    /// Intact strength exponent `N`.
    pub intact_exponent: f64,
    /// Fractured strength coefficient `B`.
    pub fractured_coefficient: f64,
    /// Fractured strength exponent `M`.
    pub fractured_exponent: f64,
    /// Largest normalised fractured strength, `SFMAX`.
    pub fractured_limit: f64,
    /// Strain-rate coefficient `C`.
    pub rate_coefficient: f64,
    /// Reference strain rate `ε̇0`, in 1/s.
    pub reference_strain_rate: f64,
}

impl JohnsonHolmquistStrength {
    /// Checks the parameters.
    ///
    /// # Errors
    /// Returns [`InvalidParameter`] naming the first invalid parameter.
    pub fn validate(&self) -> Result<(), InvalidParameter> {
        validation::positive("strength.shear_modulus", self.shear_modulus)?;
        validation::positive("strength.hel_stress", self.hel_stress)?;
        validation::positive("strength.hel_pressure", self.hel_pressure)?;
        validation::non_negative("strength.tensile_strength", self.tensile_strength)?;
        validation::positive("strength.intact_coefficient", self.intact_coefficient)?;
        validation::non_negative("strength.intact_exponent", self.intact_exponent)?;
        validation::non_negative("strength.fractured_coefficient", self.fractured_coefficient)?;
        validation::non_negative("strength.fractured_exponent", self.fractured_exponent)?;
        validation::positive("strength.fractured_limit", self.fractured_limit)?;
        validation::non_negative("strength.rate_coefficient", self.rate_coefficient)?;
        validation::positive("strength.reference_strain_rate", self.reference_strain_rate)
    }

    /// Normalised strength of intact (`0`) and fully fractured (`1`)
    /// material at `pressure`, without the strain-rate factor.
    fn normalised_strengths(&self, pressure: f64) -> (f64, f64) {
        let pressure = pressure / self.hel_pressure;
        let tension = self.tensile_strength / self.hel_pressure;
        let intact =
            self.intact_coefficient * (pressure + tension).max(0.0).powf(self.intact_exponent);
        let fractured = (self.fractured_coefficient
            * pressure.max(0.0).powf(self.fractured_exponent))
        .min(self.fractured_limit);
        (intact, fractured)
    }
}

impl StrengthModel for JohnsonHolmquistStrength {
    fn shear_modulus(&self) -> f64 {
        self.shear_modulus
    }

    fn flow_stress(&self, state: PlasticState) -> f64 {
        let (intact, fractured) = self.normalised_strengths(state.pressure);
        let damage = state.damage.clamp(0.0, 1.0);
        let rate = 1.0
            + self.rate_coefficient
                * (state.strain_rate / self.reference_strain_rate)
                    .max(1.0)
                    .ln();
        // Fracture can only weaken the material.
        let current = intact - damage * (intact - fractured.min(intact));
        (current * rate * self.hel_stress).max(0.0)
    }

    fn keeps_strength_when_failed(&self) -> bool {
        true
    }
}

/// JH-2 damage.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JohnsonHolmquistDamage {
    /// Damage coefficient `D1`.
    pub d1: f64,
    /// Damage exponent `D2`.
    pub d2: f64,
    /// Pressure at the Hugoniot elastic limit, `P_HEL`, in Pa.
    pub hel_pressure: f64,
    /// Maximum hydrostatic tension the intact material carries, `T`, in Pa.
    pub tensile_strength: f64,
}

impl JohnsonHolmquistDamage {
    /// Checks the parameters.
    ///
    /// # Errors
    /// Returns [`InvalidParameter`] naming the first invalid parameter.
    pub fn validate(&self) -> Result<(), InvalidParameter> {
        validation::positive("damage.d1", self.d1)?;
        validation::non_negative("damage.d2", self.d2)?;
        validation::positive("damage.hel_pressure", self.hel_pressure)?;
        validation::non_negative("damage.tensile_strength", self.tensile_strength)
    }
}

impl DamageModel for JohnsonHolmquistDamage {
    /// The fracture strain depends on the pressure in `state`; the
    /// triaxiality is not used.
    fn failure_strain(&self, _triaxiality: f64, state: PlasticState) -> f64 {
        let confinement = (state.pressure + self.tensile_strength) / self.hel_pressure;
        (self.d1 * confinement.max(0.0).powf(self.d2)).max(MIN_FAILURE_STRAIN)
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::{JohnsonHolmquistDamage, JohnsonHolmquistStrength};
    use crate::{DamageModel, PlasticState, StrengthModel};

    /// Alumina (99.5 %), constants as commonly cited for JH-2.
    const STRENGTH: JohnsonHolmquistStrength = JohnsonHolmquistStrength {
        shear_modulus: 90.16e9,
        hel_stress: 1.995e9,
        hel_pressure: 1.46e9,
        tensile_strength: 0.2e9,
        intact_coefficient: 0.93,
        intact_exponent: 0.6,
        fractured_coefficient: 0.31,
        fractured_exponent: 0.6,
        fractured_limit: 1.0,
        rate_coefficient: 0.0,
        reference_strain_rate: 1.0,
    };
    const DAMAGE: JohnsonHolmquistDamage = JohnsonHolmquistDamage {
        d1: 0.005,
        d2: 1.0,
        hel_pressure: 1.46e9,
        tensile_strength: 0.2e9,
    };

    fn state(pressure: f64, damage: f64) -> PlasticState {
        PlasticState {
            strain: 0.0,
            strain_rate: 1.0,
            temperature: 294.0,
            pressure,
            damage,
        }
    }

    #[test]
    fn intact_strength_at_the_elastic_limit_is_the_limit_stress() {
        // The constants are consistent: A·(1 + T*)ᴺ ≈ 1 at P = P_HEL.
        let stress = STRENGTH.flow_stress(state(1.46e9, 0.0));
        assert!((stress / 1.995e9 - 1.0).abs() < 0.01, "{stress}");
    }

    #[test]
    fn strength_vanishes_at_the_tensile_limit_and_in_fractured_material_without_pressure() {
        assert!(STRENGTH.flow_stress(state(-0.2e9, 0.0)).abs() < 1.0);
        assert!(STRENGTH.flow_stress(state(-1.0e9, 0.0)).abs() < 1.0);
        assert!(STRENGTH.flow_stress(state(0.0, 1.0)).abs() < 1.0);
        // Intact material at zero pressure still has strength.
        assert!(STRENGTH.flow_stress(state(0.0, 0.0)) > 0.5e9);
    }

    #[test]
    fn fractured_strength_follows_its_own_curve_up_to_the_limit() {
        // B·(P*)ᴹ at P* = 2: 0.31·2^0.6 = 0.4699.
        let stress = STRENGTH.flow_stress(state(2.92e9, 1.0));
        assert!((stress / 1.995e9 - 0.4699).abs() < 1e-3, "{stress}");
        let capped = JohnsonHolmquistStrength {
            fractured_limit: 0.2,
            ..STRENGTH
        };
        assert!((capped.flow_stress(state(2.92e9, 1.0)) / 1.995e9 - 0.2).abs() < 1e-9);
        // Half-damaged material lies half-way between the curves.
        let intact = STRENGTH.flow_stress(state(2.92e9, 0.0));
        let half = STRENGTH.flow_stress(state(2.92e9, 0.5));
        assert!((half - f64::midpoint(intact, stress)).abs() < 1.0);
    }

    #[test]
    fn fracture_strain_grows_with_pressure_from_zero_at_the_tensile_limit() {
        // D1·(P* + T*)^D2 at P = P_HEL: 0.005·(1 + 0.137) = 0.005685.
        let strain = DAMAGE.failure_strain(0.0, state(1.46e9, 0.0));
        assert!((strain - 0.005_685).abs() < 1e-6, "{strain}");
        assert!(DAMAGE.failure_strain(0.0, state(-0.2e9, 0.0)) <= 1.0e-6);
        assert!(DAMAGE.damage_increment(0.001, 0.0, state(1.46e9, 0.0)) > 0.17);
    }

    #[test]
    fn parameters_are_validated() {
        assert_eq!(STRENGTH.validate(), Ok(()));
        assert_eq!(DAMAGE.validate(), Ok(()));
        let broken = JohnsonHolmquistStrength {
            hel_pressure: 0.0,
            ..STRENGTH
        };
        assert_eq!(
            broken.validate().unwrap_err().field,
            "strength.hel_pressure"
        );
        assert!(STRENGTH.keeps_strength_when_failed());
    }

    proptest! {
        #[test]
        fn strength_rises_with_pressure_and_falls_with_damage(
            pressure in -0.5e9..10.0e9_f64,
            extra in 0.0..2.0e9_f64,
            damage in 0.0..1.0_f64,
            more in 0.0..1.0_f64,
        ) {
            let base = STRENGTH.flow_stress(state(pressure, damage));
            prop_assert!(base >= 0.0);
            prop_assert!(STRENGTH.flow_stress(state(pressure + extra, damage)) >= base - 1.0);
            let worse = (damage + more).min(1.0);
            prop_assert!(STRENGTH.flow_stress(state(pressure, worse)) <= base + 1.0);
        }
    }
}
