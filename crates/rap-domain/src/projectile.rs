//! Parametric projectile definitions.

use std::f64::consts::FRAC_PI_2;

use serde::{Deserialize, Serialize};

use crate::material_id::{MaterialId, SHELL_STEEL, STEEL_4340, TNT_INERT, TUNGSTEN_HEAVY_ALLOY};
use crate::validation::{self, InvalidParameter};

/// Shape of the front of a body of revolution.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "shape", rename_all = "snake_case")]
pub enum NoseShape {
    /// Flat face, no nose section.
    Flat,
    /// Half sphere with the body radius.
    Hemisphere,
    /// Cone with the given half-angle at the tip, in radians.
    Cone {
        /// Angle between the axis and the cone surface, in `(0, π/2)`.
        #[serde(rename = "half_angle_deg", with = "crate::degrees")]
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BallisticCap {
    /// Length of the cap ahead of the body shoulder, in metres; at least
    /// half the caliber.
    pub length: f64,
    /// Wall thickness, in metres.
    pub wall_thickness: f64,
    /// Material of the cap.
    pub material: MaterialId,
}

/// Bursting charge in a cavity bored into the base of a full-caliber shot.
///
/// The cavity is a cylinder on the axis, closed at the rear by a base plug
/// of the body material. The charge is modelled as an inert solid bonded
/// to the shell: it weakens the shell and changes its mass, but it does
/// not detonate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Filler {
    /// Diameter of the cavity, in metres; smaller than the caliber.
    pub diameter: f64,
    /// Length of the cavity along the axis, in metres.
    pub length: f64,
    /// Thickness of the base plug behind the cavity, in metres. The cavity
    /// and the plug together must fit in the cylindrical body.
    pub base_plug: f64,
    /// Material of the charge.
    pub material: MaterialId,
}

/// Armor-piercing full-caliber shot: a body with an optional ballistic cap
/// and an optional bursting charge (APHE).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    /// Bursting charge, if the shot has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filler: Option<Filler>,
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
            filler: None,
        }
    }

    /// A 152 mm sharp-nosed steel shell of about 48.8 kg with a small
    /// bursting charge, after the Soviet BR-540. Only the caliber and the
    /// mass follow the original; the proportions are estimated.
    #[must_use]
    pub fn aphe_152mm() -> Self {
        Self {
            caliber: 0.152,
            body_length: 0.26,
            nose: NoseShape::Ogive {
                caliber_radius_head: 1.5,
            },
            cap: None,
            material: MaterialId::new(SHELL_STEEL),
            filler: Some(Filler {
                diameter: 0.05,
                length: 0.12,
                base_plug: 0.03,
                material: MaterialId::new(TNT_INERT),
            }),
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
            validation::within(
                "cap.length",
                cap.length,
                &(self.caliber / 2.0..=f64::MAX),
                "at least half the caliber",
            )?;
            validation::within(
                "cap.wall_thickness",
                cap.wall_thickness,
                &(f64::MIN_POSITIVE..=self.caliber / 2.0),
                "positive and at most half the caliber",
            )?;
        }
        if let Some(filler) = &self.filler {
            if !(filler.diameter > 0.0 && filler.diameter < self.caliber) {
                return Err(InvalidParameter {
                    field: "filler.diameter",
                    value: filler.diameter,
                    requirement: "positive and smaller than the caliber",
                });
            }
            validation::positive("filler.length", filler.length)?;
            validation::non_negative("filler.base_plug", filler.base_plug)?;
            validation::within(
                "filler.length",
                filler.length,
                &(f64::MIN_POSITIVE..=self.body_length - filler.base_plug),
                "short enough for the cavity and the base plug to fit in the body",
            )?;
        }
        Ok(())
    }
}

/// Armor-piercing fin-stabilized discarding-sabot penetrator: a long rod.
/// The sabot is discarded before impact and is not modelled.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Projectile {
    /// Full-caliber capped shot.
    Apbc(ApbcProjectile),
    /// Long-rod penetrator.
    Apfsds(ApfsdsProjectile),
}

impl Projectile {
    /// Radius of the penetrating body, in metres.
    #[must_use]
    pub fn body_radius(&self) -> f64 {
        self.diameter() / 2.0
    }

    /// Length of the cylindrical part behind the nose, in metres.
    #[must_use]
    pub fn body_length(&self) -> f64 {
        match self {
            Self::Apbc(shot) => shot.body_length,
            Self::Apfsds(rod) => rod.rod_length,
        }
    }

    /// Shape of the penetrating nose.
    #[must_use]
    pub fn nose(&self) -> NoseShape {
        match self {
            Self::Apbc(shot) => shot.nose,
            Self::Apfsds(rod) => rod.nose,
        }
    }

    /// Material of the penetrating body.
    #[must_use]
    pub fn material(&self) -> &MaterialId {
        match self {
            Self::Apbc(shot) => &shot.material,
            Self::Apfsds(rod) => &rod.material,
        }
    }

    /// Ballistic cap, if the projectile has one.
    #[must_use]
    pub fn cap(&self) -> Option<&BallisticCap> {
        match self {
            Self::Apbc(shot) => shot.cap.as_ref(),
            Self::Apfsds(_) => None,
        }
    }

    /// Bursting charge, if the projectile has one.
    #[must_use]
    pub fn filler(&self) -> Option<&Filler> {
        match self {
            Self::Apbc(shot) => shot.filler.as_ref(),
            Self::Apfsds(_) => None,
        }
    }

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
    fn shell_with_a_charge_is_valid_and_its_cavity_must_fit() {
        let shell = ApbcProjectile::aphe_152mm();
        assert_eq!(shell.validate(), Ok(()));
        assert!(Projectile::Apbc(shell.clone()).filler().is_some());

        let field = |change: &dyn Fn(&mut super::Filler)| {
            let mut shell = shell.clone();
            change(shell.filler.as_mut().expect("the preset has a charge"));
            shell.validate().unwrap_err().field
        };
        assert_eq!(field(&|filler| filler.diameter = 0.152), "filler.diameter");
        assert_eq!(field(&|filler| filler.diameter = 0.0), "filler.diameter");
        assert_eq!(field(&|filler| filler.length = 0.0), "filler.length");
        assert_eq!(
            field(&|filler| filler.base_plug = -0.01),
            "filler.base_plug"
        );
        // 0.12 m of cavity and 0.15 m of plug do not fit in a 0.26 m body.
        assert_eq!(field(&|filler| filler.base_plug = 0.15), "filler.length");
    }

    #[test]
    fn non_positive_dimensions_are_rejected() {
        let mut rod = ApfsdsProjectile::generic_120mm();
        rod.rod_length = 0.0;
        assert_eq!(rod.validate().unwrap_err().field, "rod_length");
    }
}
