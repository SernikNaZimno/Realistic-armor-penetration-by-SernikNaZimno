//! Built-in materials.
//!
//! Parameters are taken from commonly cited open literature (Johnson & Cook,
//! 1983 and 1985, for strength and fracture; linear-Hugoniot fits for the
//! equations of state; Johnson–Holmquist constants for alumina as commonly
//! cited for JH-2; spall strengths of the metals are round values inside the ranges
//! reported for these alloys). They are starting points for qualitative studies and
//! must be checked against a primary source before any quantitative use.

use rap_domain::MaterialId;
use rap_domain::material_id::{ALUMINA, SHELL_STEEL, STEEL_4340, TNT_INERT, TUNGSTEN_HEAVY_ALLOY};

use crate::eos::{Eos, MieGruneisen, PolynomialEos};
use crate::johnson_cook::{JohnsonCookDamage, JohnsonCookStrength, RateTemperatureReference};
use crate::johnson_holmquist::{JohnsonHolmquistDamage, JohnsonHolmquistStrength};
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
        spall_strength: 3.5e9,
        specific_heat: 477.0,
        taylor_quinney: TAYLOR_QUINNEY,
    }
}

/// Hardened steel of a full-caliber armor-piercing shell: more than twice
/// as strong as armor steel and far less ductile, so that a shell stays
/// nearly rigid while it penetrates and breaks rather than bends when it is
/// overloaded.
///
/// The strength corresponds to a hardness of roughly 55 HRC and the
/// fracture strains are estimates for a hard, brittle steel; none of these
/// values comes from a primary source.
#[must_use]
pub fn shell_steel() -> Material {
    let reference = RateTemperatureReference {
        strain_rate: 1.0,
        temperature: ROOM_TEMPERATURE,
        melting_temperature: 1793.0,
    };
    Material {
        id: MaterialId::new(SHELL_STEEL),
        name: "Hardened shell steel".to_owned(),
        eos: Eos::MieGruneisen(MieGruneisen {
            reference_density: 7830.0,
            bulk_sound_speed: 4569.0,
            hugoniot_slope: 1.49,
            gruneisen_gamma: 2.17,
        }),
        strength: Strength::JohnsonCook(JohnsonCookStrength {
            shear_modulus: 77.0e9,
            yield_stress: 1900.0e6,
            hardening_modulus: 510.0e6,
            hardening_exponent: 0.26,
            rate_coefficient: 0.014,
            thermal_exponent: 1.03,
            reference,
        }),
        damage: Damage::JohnsonCook(JohnsonCookDamage {
            d1: 0.02,
            d2: 0.1,
            d3: -2.12,
            d4: 0.002,
            d5: 0.61,
            reference,
        }),
        spall_strength: 3.5e9,
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
        spall_strength: 2.0e9,
        specific_heat: 134.0,
        taylor_quinney: TAYLOR_QUINNEY,
    }
}

/// Alumina ceramic, 99.5 % pure, with the Johnson–Holmquist JH-2 model.
///
/// The equivalent stress at the Hugoniot elastic limit follows from the
/// longitudinal limit of 2.79e9 Pa and its pressure of 1.46e9 Pa:
/// `σ_HEL = 1.5·(HEL − P_HEL)`.
#[must_use]
pub fn alumina() -> Material {
    let (hel_pressure, tensile_strength) = (1.46e9, 0.2e9);
    Material {
        id: MaterialId::new(ALUMINA),
        name: "Alumina 99.5 %".to_owned(),
        eos: Eos::Polynomial(PolynomialEos {
            reference_density: 3700.0,
            k1: 130.95e9,
            k2: 0.0,
            k3: 0.0,
        }),
        strength: Strength::JohnsonHolmquist(JohnsonHolmquistStrength {
            shear_modulus: 90.16e9,
            hel_stress: 1.5 * (2.79e9 - hel_pressure),
            hel_pressure,
            tensile_strength,
            intact_coefficient: 0.93,
            intact_exponent: 0.6,
            fractured_coefficient: 0.31,
            fractured_exponent: 0.6,
            fractured_limit: 1.0,
            rate_coefficient: 0.0,
            reference_strain_rate: 1.0,
        }),
        damage: Damage::JohnsonHolmquist(JohnsonHolmquistDamage {
            d1: 0.005,
            d2: 1.0,
            hel_pressure,
            tensile_strength,
        }),
        spall_strength: tensile_strength,
        specific_heat: 880.0,
        taylor_quinney: TAYLOR_QUINNEY,
    }
}

/// Cast TNT as an inert solid: a stand-in for the bursting charge of a
/// shell. It has the density and roughly the stiffness of the explosive and
/// almost no strength; it does not react.
///
/// The Hugoniot is a round-number fit for unreacted TNT and the strength
/// values are estimates; neither comes from a primary source.
#[must_use]
pub fn tnt_inert() -> Material {
    let reference = RateTemperatureReference {
        strain_rate: 1.0,
        temperature: ROOM_TEMPERATURE,
        melting_temperature: 354.0,
    };
    Material {
        id: MaterialId::new(TNT_INERT),
        name: "TNT, inert".to_owned(),
        eos: Eos::MieGruneisen(MieGruneisen {
            reference_density: 1630.0,
            bulk_sound_speed: 2400.0,
            hugoniot_slope: 2.0,
            gruneisen_gamma: 1.0,
        }),
        strength: Strength::JohnsonCook(JohnsonCookStrength {
            shear_modulus: 3.0e9,
            yield_stress: 40.0e6,
            hardening_modulus: 0.0,
            hardening_exponent: 1.0,
            rate_coefficient: 0.0,
            thermal_exponent: 1.0,
            reference,
        }),
        damage: Damage::JohnsonCook(JohnsonCookDamage {
            d1: 0.05,
            d2: 0.0,
            d3: 0.0,
            d4: 0.0,
            d5: 0.0,
            reference,
        }),
        spall_strength: 20.0e6,
        specific_heat: 1130.0,
        taylor_quinney: TAYLOR_QUINNEY,
    }
}

/// All built-in materials.
#[must_use]
pub fn builtin() -> Vec<Material> {
    vec![
        steel_4340(),
        tungsten_heavy_alloy(),
        alumina(),
        shell_steel(),
        tnt_inert(),
    ]
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
                pressure: 0.0,
                damage: 0.0,
            };
            assert!(
                material.strength.flow_stress(state) > 0.0,
                "{}",
                material.id
            );
        }
    }
}
