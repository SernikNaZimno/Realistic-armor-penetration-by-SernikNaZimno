//! Layered, optionally shaped armor definition.

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

/// One point of the profile that shapes the armor face.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfilePoint {
    /// Distance from the shot line, measured along the plate, in metres.
    pub lateral: f64,
    /// Displacement of the armor face along the plate normal at that
    /// distance, in metres; positive values move the face away from the
    /// projectile.
    pub offset: f64,
}

/// An array of plates, listed from the struck face inwards.
///
/// Without a face profile the plates are flat. With one, the face is
/// displaced along the plate normal by a piecewise-linear function of the
/// distance from the shot line, the same on both sides of it, and every
/// layer follows that shape.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Armor {
    /// Plates in the order the projectile meets them.
    pub layers: Vec<ArmorLayer>,
    /// Angle between the projectile path and the plate normal, in radians,
    /// in `[0, π/2)`. Zero is a perpendicular hit.
    #[serde(rename = "obliquity_deg", with = "crate::degrees")]
    pub obliquity: f64,
    /// Shape of the armor face, as points ordered by increasing distance
    /// from the shot line. Empty for flat armor.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub face_profile: Vec<ProfilePoint>,
}

impl Armor {
    /// Flat armor made of `layers`, struck at `obliquity` radians.
    #[must_use]
    pub fn flat(layers: Vec<ArmorLayer>, obliquity: f64) -> Self {
        Self {
            layers,
            obliquity,
            face_profile: Vec::new(),
        }
    }

    /// Checks the layers, the obliquity and the face profile.
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
        let mut previous = None;
        for point in &self.face_profile {
            validation::non_negative("face_profile.lateral", point.lateral)?;
            validation::finite("face_profile.offset", point.offset)?;
            if previous.is_some_and(|lateral| point.lateral <= lateral) {
                return Err(InvalidParameter {
                    field: "face_profile.lateral",
                    value: point.lateral,
                    requirement: "strictly increasing from one point to the next",
                }
                .into());
            }
            previous = Some(point.lateral);
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

    /// Displacement of the face given by the profile itself at `lateral`
    /// from the shot line: linear between points, constant outside them.
    fn profile_offset(&self, lateral: f64) -> f64 {
        let lateral = lateral.abs();
        let (Some(first), Some(last)) = (self.face_profile.first(), self.face_profile.last())
        else {
            return 0.0;
        };
        if lateral <= first.lateral {
            return first.offset;
        }
        if lateral >= last.lateral {
            return last.offset;
        }
        self.face_profile
            .windows(2)
            .find(|pair| lateral <= pair[1].lateral)
            .map_or(last.offset, |pair| {
                let fraction = (lateral - pair[0].lateral) / (pair[1].lateral - pair[0].lateral);
                pair[0].offset + fraction * (pair[1].offset - pair[0].offset)
            })
    }

    /// Displacement of the armor face along the plate normal at `lateral`
    /// from the shot line, relative to the face on the shot line itself, in
    /// metres. Positive values are further from the projectile.
    #[must_use]
    pub fn face_offset(&self, lateral: f64) -> f64 {
        self.profile_offset(lateral) - self.profile_offset(0.0)
    }

    /// Smallest and largest [`Self::face_offset`] over the whole profile,
    /// in metres; `(0, 0)` for flat armor.
    #[must_use]
    pub fn face_offset_range(&self) -> (f64, f64) {
        let on_shot_line = self.profile_offset(0.0);
        self.face_profile
            .iter()
            .map(|point| point.offset - on_shot_line)
            .fold((0.0, 0.0), |(lowest, highest): (f64, f64), offset| {
                (lowest.min(offset), highest.max(offset))
            })
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
    use super::{Armor, ArmorLayer, ProfilePoint};
    use crate::material_id::{MaterialId, STEEL_4340};
    use crate::validation::DomainError;

    fn plate(thickness: f64, gap_before: f64) -> ArmorLayer {
        ArmorLayer {
            material: MaterialId::new(STEEL_4340),
            thickness,
            gap_before,
        }
    }

    fn point(lateral: f64, offset: f64) -> ProfilePoint {
        ProfilePoint { lateral, offset }
    }

    #[test]
    fn empty_armor_is_rejected() {
        assert_eq!(
            Armor::flat(Vec::new(), 0.0).validate(),
            Err(DomainError::EmptyArmor)
        );
    }

    #[test]
    fn obliquity_must_be_below_grazing() {
        let mut armor = Armor::flat(vec![plate(0.1, 0.0)], 1.0);
        assert_eq!(armor.validate(), Ok(()));
        for bad in [-0.1, std::f64::consts::FRAC_PI_2, f64::NAN] {
            armor.obliquity = bad;
            assert!(armor.validate().is_err(), "{bad} should be rejected");
        }
    }

    #[test]
    fn line_of_sight_doubles_at_sixty_degrees() {
        let armor = Armor::flat(
            vec![plate(0.03, 0.0), plate(0.07, 0.05)],
            60.0_f64.to_radians(),
        );
        assert!((armor.total_thickness() - 0.10).abs() < 1e-12);
        assert!((armor.line_of_sight_thickness() - 0.20).abs() < 1e-12);
    }

    #[test]
    fn negative_gap_is_rejected() {
        let armor = Armor::flat(vec![plate(0.1, -0.01)], 0.0);
        assert!(
            matches!(armor.validate(), Err(DomainError::InvalidParameter(e)) if e.field == "layer.gap_before")
        );
    }

    #[test]
    fn flat_armor_has_no_face_offset() {
        let armor = Armor::flat(vec![plate(0.1, 0.0)], 0.0);
        assert!(armor.face_offset(0.3).abs() < f64::EPSILON);
        assert_eq!(armor.face_offset_range(), (0.0, 0.0));
    }

    #[test]
    fn face_offset_interpolates_and_is_relative_to_the_shot_line() {
        let mut armor = Armor::flat(vec![plate(0.1, 0.0)], 0.0);
        armor.face_profile = vec![point(0.0, 0.01), point(0.1, 0.03), point(0.2, -0.02)];
        assert_eq!(armor.validate(), Ok(()));
        assert!(armor.face_offset(0.0).abs() < 1e-15);
        assert!((armor.face_offset(0.05) - 0.01).abs() < 1e-15);
        assert!((armor.face_offset(0.1) - 0.02).abs() < 1e-15);
        assert!((armor.face_offset(0.15) - -0.005).abs() < 1e-15);
        // Constant beyond the last point and the same on both sides.
        assert!((armor.face_offset(0.5) - -0.03).abs() < 1e-15);
        assert!((armor.face_offset(-0.05) - 0.01).abs() < 1e-15);
        let (lowest, highest) = armor.face_offset_range();
        assert!((lowest - -0.03).abs() < 1e-15 && (highest - 0.02).abs() < 1e-15);
    }

    #[test]
    fn profile_starting_off_the_shot_line_is_flat_up_to_its_first_point() {
        let mut armor = Armor::flat(vec![plate(0.1, 0.0)], 0.0);
        armor.face_profile = vec![point(0.05, 0.0), point(0.15, 0.04)];
        assert!(armor.face_offset(0.03).abs() < 1e-15);
        assert!((armor.face_offset(0.10) - 0.02).abs() < 1e-15);
    }

    #[test]
    fn profile_points_must_be_ordered() {
        let mut armor = Armor::flat(vec![plate(0.1, 0.0)], 0.0);
        armor.face_profile = vec![point(0.1, 0.0), point(0.1, 0.02)];
        assert!(
            matches!(armor.validate(), Err(DomainError::InvalidParameter(e)) if e.field == "face_profile.lateral")
        );
        armor.face_profile = vec![point(-0.1, 0.0)];
        assert!(armor.validate().is_err());
    }
}
