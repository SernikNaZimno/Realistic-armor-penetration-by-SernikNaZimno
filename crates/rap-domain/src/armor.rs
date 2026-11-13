//! Layered armor definition.

use std::f64::consts::FRAC_PI_2;

use serde::{Deserialize, Serialize};

use crate::material_id::MaterialId;
use crate::validation::{self, DomainError, InvalidParameter};

/// One plate of an armor array.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArmorLayer {
    /// Material of the plate.
    pub material: MaterialId,
    /// Plate thickness along its normal, in metres.
    pub thickness: f64,
    /// Empty space in front of this plate, along the normal, in metres.
    /// Non-zero values model spaced armor.
    #[serde(default)]
    pub gap_before: f64,
}

/// An array of parallel plates, listed from the struck face inwards.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Armor {
    /// Plates in the order the projectile meets them.
    pub layers: Vec<ArmorLayer>,
    /// Angle between the projectile path and the plate normal, in radians,
    /// in `[0, π/2)`. Zero is a perpendicular hit.
    #[serde(rename = "obliquity_deg", with = "crate::degrees")]
    pub obliquity: f64,
}

impl Armor {
    /// Checks the layers and the obliquity.
    ///
    /// # Errors
    /// Returns [`DomainError::EmptyArmor`] if there is no layer, or
    /// [`DomainError::InvalidParameter`] naming the first invalid value.
    pub fn validate(&self) -> Result<(), DomainError> {
        if self.layers.is_empty() {
            return Err(DomainError::EmptyArmor);
        }
        for layer in &self.layers {
            validation::positive("layer.thickness", layer.thickness)?;
            validation::non_negative("layer.gap_before", layer.gap_before)?;
        }
        if self.obliquity >= 0.0 && self.obliquity < FRAC_PI_2 {
            Ok(())
        } else {
            Err(InvalidParameter {
                field: "obliquity",
                value: self.obliquity,
                requirement: "at least 0 and below π/2 rad",
            }
            .into())
        }
    }

    /// Sum of plate thicknesses along the normal, excluding gaps, in metres.
    #[must_use]
    pub fn total_thickness(&self) -> f64 {
        self.layers.iter().map(|layer| layer.thickness).sum()
    }

    /// Plate material met along the projectile path, excluding gaps, in metres.
    #[must_use]
    pub fn line_of_sight_thickness(&self) -> f64 {
        self.total_thickness() / self.obliquity.cos()
    }
}

#[cfg(test)]
mod tests {
    use super::{Armor, ArmorLayer};
    use crate::material_id::{MaterialId, STEEL_4340};
    use crate::validation::DomainError;

    fn plate(thickness: f64, gap_before: f64) -> ArmorLayer {
        ArmorLayer {
            material: MaterialId::new(STEEL_4340),
            thickness,
            gap_before,
        }
    }

    #[test]
    fn empty_armor_is_rejected() {
        let armor = Armor {
            layers: Vec::new(),
            obliquity: 0.0,
        };
        assert_eq!(armor.validate(), Err(DomainError::EmptyArmor));
    }

    #[test]
    fn obliquity_must_be_below_grazing() {
        let mut armor = Armor {
            layers: vec![plate(0.1, 0.0)],
            obliquity: 1.0,
        };
        assert_eq!(armor.validate(), Ok(()));
        for bad in [-0.1, std::f64::consts::FRAC_PI_2, f64::NAN] {
            armor.obliquity = bad;
            assert!(armor.validate().is_err(), "{bad} should be rejected");
        }
    }

    #[test]
    fn line_of_sight_doubles_at_sixty_degrees() {
        let armor = Armor {
            layers: vec![plate(0.03, 0.0), plate(0.07, 0.05)],
            obliquity: 60.0_f64.to_radians(),
        };
        assert!((armor.total_thickness() - 0.10).abs() < 1e-12);
        assert!((armor.line_of_sight_thickness() - 0.20).abs() < 1e-12);
    }

    #[test]
    fn negative_gap_is_rejected() {
        let armor = Armor {
            layers: vec![plate(0.1, -0.01)],
            obliquity: 0.0,
        };
        assert!(
            matches!(armor.validate(), Err(DomainError::InvalidParameter(e)) if e.field == "layer.gap_before")
        );
    }
}
