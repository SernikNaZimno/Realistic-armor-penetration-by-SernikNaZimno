//! Built-in materials.
//!
//! Parameters are taken from commonly cited open literature (Johnson & Cook,
//! 1983 and 1985, for strength and fracture; linear-Hugoniot fits for the
//! equations of state). They are starting points for qualitative studies and
//! must be checked against a primary source before any quantitative use.

use rap_domain::MaterialId;
use rap_domain::material_id::{STEEL_4340, TUNGSTEN_HEAVY_ALLOY};

use crate::eos::{Eos, MieGruneisen};
use crate::johnson_cook::{JohnsonCookDamage, JohnsonCookStrength, RateTemperatureReference};
use crate::material::{Damage, Material, Strength};

const ROOM_TEMPERATURE: f64 = 294.0;
const TAYLOR_QUINNEY: f64 = 0.9;

/// 4340 steel, also usable as a surrogate for rolled homogeneous armor.
#[must_use]
pub fn steel_4340() -> Material {
    let reference = RateTemperatureReference {
        strain_rate: 1.0,
        temperature: ROOM_TEMPERATURE,
        melting_temperature: 1793.0,
    };
    Material {
        id: MaterialId::new(STEEL_4340),
        name: "4340 steel".to_owned(),
        eos: Eos::MieGruneisen(MieGruneisen {
            reference_density: 7830.0,
            bulk_sound_speed: 4569.0,
            hugoniot_slope: 1.49,
            gruneisen_gamma: 2.17,
        }),
        strength: Strength::JohnsonCook(JohnsonCookStrength {
            shear_modulus: 77.0e9,
            yield_stress: 792.0e6,
            hardening_modulus: 510.0e6,
            hardening_exponent: 0.26,
            rate_coefficient: 0.014,
            thermal_exponent: 1.03,
            reference,
        }),
        damage: Damage::JohnsonCook(JohnsonCookDamage {
            d1: 0.05,
            d2: 3.44,
            d3: -2.12,
            d4: 0.002,
            d5: 0.61,
            reference,
        }),
        specific_heat: 477.0,
        taylor_quinney: TAYLOR_QUINNEY,
    }
}

/// Tungsten heavy alloy (about 90 % W with Ni and Fe) for long-rod penetrators.
#[must_use]
pub fn tungsten_heavy_alloy() -> Material {
    let reference = RateTemperatureReference {
        strain_rate: 1.0,
        temperature: ROOM_TEMPERATURE,
        melting_temperature: 1723.0,
    };
    Material {
        id: MaterialId::new(TUNGSTEN_HEAVY_ALLOY),
        name: "Tungsten heavy alloy".to_owned(),
        eos: Eos::MieGruneisen(MieGruneisen {
            reference_density: 17_000.0,
            bulk_sound_speed: 4029.0,
            hugoniot_slope: 1.237,
            gruneisen_gamma: 1.54,
        }),
        strength: Strength::JohnsonCook(JohnsonCookStrength {
            shear_modulus: 145.0e9,
            yield_stress: 1506.0e6,
            hardening_modulus: 177.0e6,
            hardening_exponent: 0.12,
            rate_coefficient: 0.016,
            thermal_exponent: 1.0,
            reference,
        }),
        damage: Damage::JohnsonCook(JohnsonCookDamage {
            d1: 0.0,
            d2: 0.33,
            d3: -1.5,
            d4: 0.0,
            d5: 0.0,
            reference,
        }),
        specific_heat: 134.0,
        taylor_quinney: TAYLOR_QUINNEY,
    }
}

/// All built-in materials.
#[must_use]
pub fn builtin() -> Vec<Material> {
    vec![steel_4340(), tungsten_heavy_alloy()]
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::builtin;
    use crate::{EquationOfState, PlasticState, StrengthModel};

    #[test]
    fn builtin_materials_are_valid_and_uniquely_identified() {
        let materials = builtin();
        let ids: HashSet<_> = materials
            .iter()
            .map(|material| material.id.clone())
            .collect();
        assert_eq!(ids.len(), materials.len());
        for material in &materials {
            assert_eq!(material.validate(), Ok(()), "{}", material.id);
        }
    }

    #[test]
    fn builtin_materials_are_at_rest_in_their_reference_state() {
        for material in builtin() {
            let density = material.eos.reference_density();
            assert!(
                material.eos.pressure(density, 0.0).abs() < 1e-3,
                "{}",
                material.id
            );
            let state = PlasticState {
                strain: 0.0,
                strain_rate: 1.0,
                temperature: 294.0,
            };
            assert!(
                material.strength.flow_stress(state) > 0.0,
                "{}",
                material.id
            );
        }
    }
}
