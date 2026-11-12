//! Parametric projectile definitions.

use std::f64::consts::FRAC_PI_2;

use crate::material_id::{MaterialId, STEEL_4340, TUNGSTEN_HEAVY_ALLOY};
use crate::validation::{self, InvalidParameter};

/// Shape of the front of a body of revolution.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NoseShape {
    /// Flat face, no nose section.
    Flat,
    /// Half sphere with the body radius.
    Hemisphere,
    /// Cone with the given half-angle at the tip, in radians.
    Cone {
        /// Angle between the axis and the cone surface, in `(0, π/2)`.
        half_angle: f64,
    },
    /// Tangent ogive.
    Ogive {
        /// Ogive radius divided by the body diameter; `0.5` is a hemisphere.
        caliber_radius_head: f64,
    },
}

impl NoseShape {
    /// Checks the shape parameters.
    ///
    /// # Errors
    /// Returns [`InvalidParameter`] if a parameter is outside its valid range.
    pub fn validate(&self) -> Result<(), InvalidParameter> {
        match *self {
            Self::Flat | Self::Hemisphere => Ok(()),
            Self::Cone { half_angle } => {
                let valid = half_angle > 0.0 && half_angle < FRAC_PI_2;
                if valid {
                    Ok(())
                } else {
                    Err(InvalidParameter {
                        field: "nose.half_angle",
                        value: half_angle,
                        requirement: "strictly between 0 and π/2 rad",
                    })
                }
            }
            Self::Ogive {
                caliber_radius_head,
            } => validation::within(
                "nose.caliber_radius_head",
                caliber_radius_head,
                &(0.5..=f64::MAX),
                "at least 0.5",
            ),
        }
    }
}

/// Thin hollow windshield fitted over the nose of a full-caliber shot.
#[derive(Debug, Clone, PartialEq)]
pub struct BallisticCap {
    /// Length of the cap ahead of the body shoulder, in metres.
    pub length: f64,
    /// Wall thickness, in metres.
    pub wall_thickness: f64,
    /// Material of the cap.
    pub material: MaterialId,
}

/// Armor-piercing ballistic-capped shot: a full-caliber solid body with an
/// optional ballistic cap.
#[derive(Debug, Clone, PartialEq)]
pub struct ApbcProjectile {
    /// Body diameter, in metres.
    pub caliber: f64,
    /// Length of the cylindrical body behind the nose, in metres.
    pub body_length: f64,
    /// Shape of the penetrating nose under the cap.
    pub nose: NoseShape,
    /// Ballistic cap, if fitted.
    pub cap: Option<BallisticCap>,
    /// Material of the body.
    pub material: MaterialId,
}

impl ApbcProjectile {
    /// A generic 76 mm steel shot with a ballistic cap.
    #[must_use]
    pub fn generic_76mm() -> Self {
        Self {
            caliber: 0.076,
            body_length: 0.20,
            nose: NoseShape::Ogive {
                caliber_radius_head: 1.0,
            },
            cap: Some(BallisticCap {
                length: 0.09,
                wall_thickness: 0.002,
                material: MaterialId::new(STEEL_4340),
            }),
            material: MaterialId::new(STEEL_4340),
        }
    }

    /// Checks all dimensions.
    ///
    /// # Errors
    /// Returns [`InvalidParameter`] naming the first invalid dimension.
    pub fn validate(&self) -> Result<(), InvalidParameter> {
        validation::positive("caliber", self.caliber)?;
        validation::positive("body_length", self.body_length)?;
        self.nose.validate()?;
        if let Some(cap) = &self.cap {
            validation::positive("cap.length", cap.length)?;
            validation::within(
                "cap.wall_thickness",
                cap.wall_thickness,
                &(f64::MIN_POSITIVE..=self.caliber / 2.0),
                "positive and at most half the caliber",
            )?;
        }
        Ok(())
    }
}

/// Armor-piercing fin-stabilized discarding-sabot penetrator: a long rod.
/// The sabot is discarded before impact and is not modelled.
#[derive(Debug, Clone, PartialEq)]
pub struct ApfsdsProjectile {
    /// Rod diameter, in metres.
    pub rod_diameter: f64,
    /// Length of the cylindrical rod behind the nose, in metres.
    pub rod_length: f64,
    /// Shape of the rod tip.
    pub nose: NoseShape,
    /// Material of the rod.
    pub material: MaterialId,
}

impl ApfsdsProjectile {
    /// A generic 120 mm-class tungsten-alloy long rod.
    #[must_use]
    pub fn generic_120mm() -> Self {
        Self {
            rod_diameter: 0.025,
            rod_length: 0.60,
            nose: NoseShape::Cone {
                half_angle: 15.0_f64.to_radians(),
            },
            material: MaterialId::new(TUNGSTEN_HEAVY_ALLOY),
        }
    }

    /// Checks all dimensions.
    ///
    /// # Errors
    /// Returns [`InvalidParameter`] naming the first invalid dimension.
    pub fn validate(&self) -> Result<(), InvalidParameter> {
        validation::positive("rod_diameter", self.rod_diameter)?;
        validation::positive("rod_length", self.rod_length)?;
        self.nose.validate()
    }
}

/// Any supported projectile.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Projectile {
    /// Full-caliber capped shot.
    Apbc(ApbcProjectile),
    /// Long-rod penetrator.
    Apfsds(ApfsdsProjectile),
}

impl Projectile {
    /// Largest diameter of the penetrating body, in metres.
    #[must_use]
    pub fn diameter(&self) -> f64 {
        match self {
            Self::Apbc(shot) => shot.caliber,
            Self::Apfsds(rod) => rod.rod_diameter,
        }
    }

    /// Checks all dimensions.
    ///
    /// # Errors
    /// Returns [`InvalidParameter`] naming the first invalid dimension.
    pub fn validate(&self) -> Result<(), InvalidParameter> {
        match self {
            Self::Apbc(shot) => shot.validate(),
            Self::Apfsds(rod) => rod.validate(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ApbcProjectile, ApfsdsProjectile, NoseShape, Projectile};

    #[test]
    fn presets_are_valid() {
        assert_eq!(
            Projectile::Apbc(ApbcProjectile::generic_76mm()).validate(),
            Ok(())
        );
        assert_eq!(
            Projectile::Apfsds(ApfsdsProjectile::generic_120mm()).validate(),
            Ok(())
        );
    }

    #[test]
    fn nose_parameters_are_range_checked() {
        assert!(NoseShape::Cone { half_angle: 0.0 }.validate().is_err());
        assert!(NoseShape::Cone { half_angle: 2.0 }.validate().is_err());
        assert!(
            NoseShape::Cone {
                half_angle: f64::NAN
            }
            .validate()
            .is_err()
        );
        assert!(
            NoseShape::Ogive {
                caliber_radius_head: 0.4
            }
            .validate()
            .is_err()
        );
        assert!(
            NoseShape::Ogive {
                caliber_radius_head: 0.5
            }
            .validate()
            .is_ok()
        );
    }

    #[test]
    fn cap_wall_cannot_exceed_body_radius() {
        let mut shot = ApbcProjectile::generic_76mm();
        shot.cap.as_mut().expect("preset has a cap").wall_thickness = 0.05;
        let error = shot.validate().unwrap_err();
        assert_eq!(error.field, "cap.wall_thickness");
    }

    #[test]
    fn non_positive_dimensions_are_rejected() {
        let mut rod = ApfsdsProjectile::generic_120mm();
        rod.rod_length = 0.0;
        assert_eq!(rod.validate().unwrap_err().field, "rod_length");
    }
}
