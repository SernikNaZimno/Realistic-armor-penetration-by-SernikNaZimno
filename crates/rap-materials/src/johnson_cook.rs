//! Johnson–Cook strength and fracture models.

use rap_domain::validation::{self, InvalidParameter};
use serde::{Deserialize, Serialize};

use crate::{DamageModel, PlasticState, StrengthModel};

/// Smallest failure strain a damage model reports, so that damage
/// increments stay finite under strong tension.
const MIN_FAILURE_STRAIN: f64 = 1.0e-6;

/// Reference conditions that make strain rate and temperature dimensionless.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RateTemperatureReference {
    /// Reference strain rate `ε̇0`, in 1/s.
    pub strain_rate: f64,
    /// Temperature at which the parameters were measured, in K.
    pub temperature: f64,
    /// Melting temperature, in K.
    pub melting_temperature: f64,
}

impl RateTemperatureReference {
    /// Checks that the rates and temperatures are physically ordered.
    ///
    /// # Errors
    /// Returns [`InvalidParameter`] naming the first invalid parameter.
    pub fn validate(&self) -> Result<(), InvalidParameter> {
        validation::positive("reference.strain_rate", self.strain_rate)?;
        validation::positive("reference.temperature", self.temperature)?;
        if self.melting_temperature.is_finite() && self.melting_temperature > self.temperature {
            Ok(())
        } else {
            Err(InvalidParameter {
                field: "reference.melting_temperature",
                value: self.melting_temperature,
                requirement: "finite and above the reference temperature",
            })
        }
    }

    /// `ln(ε̇/ε̇0)`, clamped at zero below the reference rate.
    fn log_rate(&self, strain_rate: f64) -> f64 {
        (strain_rate / self.strain_rate).max(1.0).ln()
    }

    /// `T* = (T − T_ref)/(T_melt − T_ref)`, clamped to `[0, 1]`.
    fn homologous_temperature(&self, temperature: f64) -> f64 {
        ((temperature - self.temperature) / (self.melting_temperature - self.temperature))
            .clamp(0.0, 1.0)
    }
}

/// Johnson–Cook flow stress:
///
/// ```text
/// σ = (A + B·εⁿ) · (1 + C·ln ε̇*) · (1 − T*ᵐ)
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JohnsonCookStrength {
    /// Elastic shear modulus, in Pa.
    pub shear_modulus: f64,
    /// Initial yield stress `A`, in Pa.
    pub yield_stress: f64,
    /// Strain-hardening modulus `B`, in Pa.
    pub hardening_modulus: f64,
    /// Strain-hardening exponent `n`.
    pub hardening_exponent: f64,
    /// Strain-rate coefficient `C`.
    pub rate_coefficient: f64,
    /// Thermal-softening exponent `m`.
    pub thermal_exponent: f64,
    /// Reference strain rate and temperatures.
    pub reference: RateTemperatureReference,
}

impl JohnsonCookStrength {
    /// Checks the parameters.
    ///
    /// # Errors
    /// Returns [`InvalidParameter`] naming the first invalid parameter.
    pub fn validate(&self) -> Result<(), InvalidParameter> {
        validation::positive("strength.shear_modulus", self.shear_modulus)?;
        validation::positive("strength.yield_stress", self.yield_stress)?;
        validation::non_negative("strength.hardening_modulus", self.hardening_modulus)?;
        validation::non_negative("strength.hardening_exponent", self.hardening_exponent)?;
        validation::non_negative("strength.rate_coefficient", self.rate_coefficient)?;
        validation::positive("strength.thermal_exponent", self.thermal_exponent)?;
        self.reference.validate()
    }
}

impl StrengthModel for JohnsonCookStrength {
    fn shear_modulus(&self) -> f64 {
        self.shear_modulus
    }

    fn flow_stress(&self, state: PlasticState) -> f64 {
        let hardening = self.yield_stress
            + self.hardening_modulus * state.strain.max(0.0).powf(self.hardening_exponent);
        let rate = 1.0 + self.rate_coefficient * self.reference.log_rate(state.strain_rate);
        let softening = 1.0
            - self
                .reference
                .homologous_temperature(state.temperature)
                .powf(self.thermal_exponent);
        hardening * rate * softening
    }
}

/// Johnson–Cook fracture strain:
///
/// ```text
/// ε_f = [D1 + D2·exp(D3·σ*)] · (1 + D4·ln ε̇*) · (1 + D5·T*)
/// ```
///
/// where `σ*` is the stress triaxiality.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JohnsonCookDamage {
    /// Constant `D1`.
    pub d1: f64,
    /// Constant `D2`.
    pub d2: f64,
    /// Constant `D3`; negative for metals, whose ductility drops in tension.
    pub d3: f64,
    /// Strain-rate constant `D4`.
    pub d4: f64,
    /// Temperature constant `D5`.
    pub d5: f64,
    /// Reference strain rate and temperatures.
    pub reference: RateTemperatureReference,
}

impl JohnsonCookDamage {
    /// Checks the parameters.
    ///
    /// # Errors
    /// Returns [`InvalidParameter`] naming the first invalid parameter.
    pub fn validate(&self) -> Result<(), InvalidParameter> {
        validation::finite("damage.d1", self.d1)?;
        validation::finite("damage.d2", self.d2)?;
        validation::finite("damage.d3", self.d3)?;
        validation::finite("damage.d4", self.d4)?;
        validation::finite("damage.d5", self.d5)?;
        self.reference.validate()
    }
}

impl DamageModel for JohnsonCookDamage {
    fn failure_strain(&self, triaxiality: f64, state: PlasticState) -> f64 {
        let stress = self.d1 + self.d2 * (self.d3 * triaxiality).exp();
        let rate = 1.0 + self.d4 * self.reference.log_rate(state.strain_rate);
        let temperature = 1.0 + self.d5 * self.reference.homologous_temperature(state.temperature);
        (stress * rate * temperature).max(MIN_FAILURE_STRAIN)
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::{JohnsonCookDamage, JohnsonCookStrength, RateTemperatureReference};
    use crate::{DamageModel, PlasticState, StrengthModel};

    const REFERENCE: RateTemperatureReference = RateTemperatureReference {
        strain_rate: 1.0,
        temperature: 294.0,
        melting_temperature: 1793.0,
    };

    /// 4340 steel, Johnson & Cook (1983, 1985).
    const STRENGTH: JohnsonCookStrength = JohnsonCookStrength {
        shear_modulus: 77.0e9,
        yield_stress: 792.0e6,
        hardening_modulus: 510.0e6,
        hardening_exponent: 0.26,
        rate_coefficient: 0.014,
        thermal_exponent: 1.03,
        reference: REFERENCE,
    };

    const DAMAGE: JohnsonCookDamage = JohnsonCookDamage {
        d1: 0.05,
        d2: 3.44,
        d3: -2.12,
        d4: 0.002,
        d5: 0.61,
        reference: REFERENCE,
    };

    fn state(strain: f64, strain_rate: f64, temperature: f64) -> PlasticState {
        PlasticState {
            strain,
            strain_rate,
            temperature,
        }
    }

    #[test]
    fn flow_stress_at_reference_conditions_is_the_yield_stress() {
        let stress = STRENGTH.flow_stress(state(0.0, 1.0, 294.0));
        assert!((stress - 792.0e6).abs() < 1.0);
    }

    #[test]
    fn flow_stress_matches_hand_calculation() {
        // 792 + 510·0.1^0.26 = 1072.27 MPa
        let hardened = STRENGTH.flow_stress(state(0.1, 1.0, 294.0));
        assert!((hardened - 1072.27e6).abs() < 0.05e6, "{hardened}");
        // rate factor at 1000 1/s: 1 + 0.014·ln(1000) = 1.096709
        let fast = STRENGTH.flow_stress(state(0.0, 1.0e3, 294.0));
        assert!((fast / 792.0e6 - 1.096_709).abs() < 1e-5, "{fast}");
    }

    #[test]
    fn material_has_no_strength_at_melting() {
        assert!(STRENGTH.flow_stress(state(0.5, 1.0e4, 1793.0)).abs() < 1e-3);
        assert!(STRENGTH.flow_stress(state(0.5, 1.0e4, 2500.0)).abs() < 1e-3);
    }

    #[test]
    fn rates_below_reference_do_not_soften() {
        let slow = STRENGTH.flow_stress(state(0.2, 1.0e-3, 294.0));
        let reference = STRENGTH.flow_stress(state(0.2, 1.0, 294.0));
        assert!((slow - reference).abs() < 1.0);
    }

    #[test]
    fn failure_strain_matches_hand_calculation() {
        // 0.05 + 3.44·exp(−2.12/3) = 1.7469
        let strain = DAMAGE.failure_strain(1.0 / 3.0, state(0.0, 1.0, 294.0));
        assert!((strain - 1.7469).abs() < 1e-3, "{strain}");
    }

    #[test]
    fn damage_increment_is_strain_over_failure_strain() {
        let plastic = state(0.0, 1.0, 294.0);
        let failure = DAMAGE.failure_strain(0.0, plastic);
        let increment = DAMAGE.damage_increment(0.01, 0.0, plastic);
        assert!((increment * failure - 0.01).abs() < 1e-12);
    }

    #[test]
    fn invalid_reference_is_rejected() {
        assert_eq!(STRENGTH.validate(), Ok(()));
        assert_eq!(DAMAGE.validate(), Ok(()));
        let reference = RateTemperatureReference {
            melting_temperature: 294.0,
            ..REFERENCE
        };
        let broken = JohnsonCookStrength {
            reference,
            ..STRENGTH
        };
        assert_eq!(
            broken.validate().unwrap_err().field,
            "reference.melting_temperature"
        );
    }

    proptest! {
        #[test]
        fn flow_stress_never_drops_with_strain_or_rate(
            strain in 0.0..2.0_f64,
            rate in 1e-2..1e6_f64,
            temperature in 250.0..1790.0_f64,
            factor in 1.0..3.0_f64,
        ) {
            let base = STRENGTH.flow_stress(state(strain, rate, temperature));
            prop_assert!(STRENGTH.flow_stress(state(strain * factor, rate, temperature)) >= base);
            prop_assert!(STRENGTH.flow_stress(state(strain, rate * factor, temperature)) >= base);
        }

        #[test]
        fn flow_stress_never_rises_with_temperature(
            strain in 0.0..2.0_f64,
            rate in 1e-2..1e6_f64,
            temperature in 250.0..2000.0_f64,
            heating in 0.0..500.0_f64,
        ) {
            let cold = STRENGTH.flow_stress(state(strain, rate, temperature));
            let hot = STRENGTH.flow_stress(state(strain, rate, temperature + heating));
            prop_assert!(hot <= cold);
            prop_assert!(hot >= 0.0);
        }

        #[test]
        fn failure_strain_is_positive_and_drops_in_tension(
            triaxiality in -3.0..3.0_f64,
            rate in 1e-2..1e6_f64,
            temperature in 250.0..2000.0_f64,
        ) {
            let plastic = state(0.0, rate, temperature);
            let strain = DAMAGE.failure_strain(triaxiality, plastic);
            prop_assert!(strain > 0.0);
            prop_assert!(DAMAGE.failure_strain(triaxiality + 0.5, plastic) <= strain);
        }
    }
}
