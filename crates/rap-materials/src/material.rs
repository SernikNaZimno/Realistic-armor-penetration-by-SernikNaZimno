//! A complete material: equation of state, strength, damage and thermal data.

use rap_domain::MaterialId;
use rap_domain::validation::{self, InvalidParameter};
use serde::{Deserialize, Serialize};

use crate::eos::Eos;
use crate::johnson_cook::{JohnsonCookDamage, JohnsonCookStrength};
use crate::{DamageModel, PlasticState, StrengthModel};

/// The built-in strength models.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
#[serde(tag = "model", rename_all = "snake_case")]
pub enum Strength {
    /// See [`JohnsonCookStrength`].
    JohnsonCook(JohnsonCookStrength),
}

impl Strength {
    /// Checks the parameters of the wrapped model.
    ///
    /// # Errors
    /// Returns [`InvalidParameter`] naming the first invalid parameter.
    pub fn validate(&self) -> Result<(), InvalidParameter> {
        match self {
            Self::JohnsonCook(model) => model.validate(),
        }
    }
}

impl StrengthModel for Strength {
    fn shear_modulus(&self) -> f64 {
        match self {
            Self::JohnsonCook(model) => model.shear_modulus(),
        }
    }

    fn flow_stress(&self, state: PlasticState) -> f64 {
        match self {
            Self::JohnsonCook(model) => model.flow_stress(state),
        }
    }
}

/// The built-in damage models.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
#[serde(tag = "model", rename_all = "snake_case")]
pub enum Damage {
    /// See [`JohnsonCookDamage`].
    JohnsonCook(JohnsonCookDamage),
}

impl Damage {
    /// Checks the parameters of the wrapped model.
    ///
    /// # Errors
    /// Returns [`InvalidParameter`] naming the first invalid parameter.
    pub fn validate(&self) -> Result<(), InvalidParameter> {
        match self {
            Self::JohnsonCook(model) => model.validate(),
        }
    }
}

impl DamageModel for Damage {
    fn failure_strain(&self, triaxiality: f64, state: PlasticState) -> f64 {
        match self {
            Self::JohnsonCook(model) => model.failure_strain(triaxiality, state),
        }
    }
}

/// Everything a solver needs to know about one material.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Material {
    /// Key under which the material is referenced by projectiles and armor.
    pub id: MaterialId,
    /// Name shown to the user.
    pub name: String,
    /// Pressure response.
    pub eos: Eos,
    /// Resistance to plastic flow.
    pub strength: Strength,
    /// Ductile failure criterion.
    pub damage: Damage,
    /// Tensile pressure at which the material fails by spallation, in Pa.
    pub spall_strength: f64,
    /// Specific heat capacity, in J/(kg·K).
    pub specific_heat: f64,
    /// Fraction of plastic work converted to heat (Taylor–Quinney
    /// coefficient), in `[0, 1]`.
    pub taylor_quinney: f64,
}

impl Material {
    /// Checks every model and the thermal data.
    ///
    /// # Errors
    /// Returns [`InvalidParameter`] naming the first invalid parameter.
    pub fn validate(&self) -> Result<(), InvalidParameter> {
        self.eos.validate()?;
        self.strength.validate()?;
        self.damage.validate()?;
        validation::positive("spall_strength", self.spall_strength)?;
        validation::positive("specific_heat", self.specific_heat)?;
        validation::within(
            "taylor_quinney",
            self.taylor_quinney,
            &(0.0..=1.0),
            "between 0 and 1",
        )
    }

    /// Adiabatic temperature rise, in K, caused by plastic work done on a
    /// unit mass, in J/kg.
    #[must_use]
    pub fn adiabatic_heating(&self, specific_plastic_work: f64) -> f64 {
        self.taylor_quinney * specific_plastic_work / self.specific_heat
    }
}

#[cfg(test)]
mod tests {
    use crate::library;

    #[test]
    fn adiabatic_heating_follows_specific_heat() {
        let steel = library::steel_4340();
        // 0.9 · 53 000 J/kg / 477 J/(kg·K) = 100 K
        assert!((steel.adiabatic_heating(53_000.0) - 100.0).abs() < 1e-9);
    }

    #[test]
    fn thermal_data_is_range_checked() {
        let mut steel = library::steel_4340();
        steel.taylor_quinney = 1.2;
        assert_eq!(steel.validate().unwrap_err().field, "taylor_quinney");
    }
}
