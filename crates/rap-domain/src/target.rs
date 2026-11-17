//! What the projectile strikes: an array of plates or a solid of any shape.

use serde::{Deserialize, Serialize};

use crate::armor::{Armor, ArmorLayer, ProfilePoint};
use crate::material_id::MaterialId;
use crate::validation::{self, DomainError, InvalidParameter};

/// Length of one unit of a mesh file in metres when the scenario does not
/// say: STL files are most often written in millimetres.
const DEFAULT_METRES_PER_UNIT: f64 = 1.0e-3;

fn default_metres_per_unit() -> f64 {
    DEFAULT_METRES_PER_UNIT
}

/// Armor whose whole shape is the closed surface in a mesh file, made of
/// one material.
///
/// The shot is placed in the coordinates of the file: the projectile flies
/// along `direction` on the line through `aim_point` and first meets the
/// solid where that line enters it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SolidArmor {
    /// Path of the STL file; a relative path starts at the folder of the
    /// scenario file.
    pub file: String,
    /// Length of one unit of the file, in metres.
    #[serde(default = "default_metres_per_unit")]
    pub metres_per_unit: f64,
    /// Material of the solid.
    pub material: MaterialId,
    /// A point on the shot line, in the coordinates and units of the file.
    pub aim_point: [f64; 3],
    /// Direction in which the projectile flies, in the coordinates of the
    /// file; its length does not matter.
    pub direction: [f64; 3],
    /// How far behind the point of impact the solid is modelled, along the
    /// shot line, in metres; `None` for all of it. Material further back is
    /// left out, which keeps a hollow shape from being modelled through to
    /// its far wall.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_depth: Option<f64>,
}

impl SolidArmor {
    /// A solid read from `file`, given in millimetres, struck along the
    /// `x` axis of the file on the line through its origin.
    #[must_use]
    pub fn new(file: impl Into<String>, material: MaterialId) -> Self {
        Self {
            file: file.into(),
            metres_per_unit: DEFAULT_METRES_PER_UNIT,
            material,
            aim_point: [0.0; 3],
            direction: [1.0, 0.0, 0.0],
            max_depth: None,
        }
    }

    /// Checks the file name, the scale and the placement of the shot.
    ///
    /// # Errors
    /// Returns [`DomainError::NoMeshFile`] if no file is named, or
    /// [`DomainError::InvalidParameter`] naming the first invalid value.
    pub fn validate(&self) -> Result<(), DomainError> {
        if self.file.trim().is_empty() {
            return Err(DomainError::NoMeshFile);
        }
        validation::positive("mesh.metres_per_unit", self.metres_per_unit)?;
        for coordinate in self.aim_point {
            validation::finite("mesh.aim_point", coordinate)?;
        }
        for component in self.direction {
            validation::finite("mesh.direction", component)?;
        }
        let length = self
            .direction
            .iter()
            .map(|component| component * component)
            .sum::<f64>()
            .sqrt();
        // The square of a tiny component underflows, the square of a huge
        // one overflows; neither gives a direction.
        if !(length.is_finite() && length > 0.0) {
            return Err(InvalidParameter {
                field: "mesh.direction",
                value: length,
                requirement: "a vector of finite, non-zero length",
            }
            .into());
        }
        if let Some(depth) = self.max_depth {
            validation::positive("mesh.max_depth", depth)?;
        }
        Ok(())
    }
}

/// What the projectile strikes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "TargetFields", into = "TargetFields")]
#[non_exhaustive]
pub enum Target {
    /// An array of plates.
    Plates(Armor),
    /// A solid of any shape, read from a mesh file.
    Solid(SolidArmor),
}

impl Target {
    /// Checks whichever kind of armor this is.
    ///
    /// # Errors
    /// Returns the first [`DomainError`] found.
    pub fn validate(&self) -> Result<(), DomainError> {
        match self {
            Self::Plates(armor) => armor.validate(),
            Self::Solid(solid) => solid.validate(),
        }
    }

    /// The array of plates, if that is what this is.
    #[must_use]
    pub fn plates(&self) -> Option<&Armor> {
        match self {
            Self::Plates(armor) => Some(armor),
            Self::Solid(_) => None,
        }
    }

    /// The array of plates, for changing it, if that is what this is.
    #[must_use]
    pub fn plates_mut(&mut self) -> Option<&mut Armor> {
        match self {
            Self::Plates(armor) => Some(armor),
            Self::Solid(_) => None,
        }
    }

    /// The solid, if that is what this is.
    #[must_use]
    pub fn solid(&self) -> Option<&SolidArmor> {
        match self {
            Self::Plates(_) => None,
            Self::Solid(solid) => Some(solid),
        }
    }

    /// Angle between the projectile path and the plate normal, in radians;
    /// zero for a solid, whose surface need not have one normal.
    #[must_use]
    pub fn obliquity(&self) -> f64 {
        self.plates().map_or(0.0, |armor| armor.obliquity)
    }
}

impl From<Armor> for Target {
    fn from(armor: Armor) -> Self {
        Self::Plates(armor)
    }
}

impl From<SolidArmor> for Target {
    fn from(solid: SolidArmor) -> Self {
        Self::Solid(solid)
    }
}

/// Serialized form of [`Target`]: the fields of a plate array, or `mesh`.
///
/// Both kinds share one set of fields so that a misspelt or misplaced name
/// is reported by name.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TargetFields {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    layers: Option<Vec<ArmorLayer>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    obliquity_deg: Option<f64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    face_profile: Vec<ProfilePoint>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    mesh: Option<SolidArmor>,
}

impl TryFrom<TargetFields> for Target {
    type Error = &'static str;

    fn try_from(fields: TargetFields) -> Result<Self, Self::Error> {
        match (fields.mesh, fields.layers, fields.obliquity_deg) {
            (Some(solid), None, None) if fields.face_profile.is_empty() => Ok(Self::Solid(solid)),
            (Some(_), ..) => {
                Err("armor given by `mesh` takes no `layers`, `obliquity_deg` or `face_profile`")
            }
            (None, Some(layers), Some(obliquity_deg)) => Ok(Self::Plates(Armor {
                layers,
                obliquity: obliquity_deg.to_radians(),
                face_profile: fields.face_profile,
            })),
            (None, ..) => Err("armor needs either `layers` and `obliquity_deg`, or `mesh`"),
        }
    }
}

impl From<Target> for TargetFields {
    fn from(target: Target) -> Self {
        match target {
            Target::Plates(armor) => Self {
                layers: Some(armor.layers),
                obliquity_deg: Some(armor.obliquity.to_degrees()),
                face_profile: armor.face_profile,
                mesh: None,
            },
            Target::Solid(solid) => Self {
                layers: None,
                obliquity_deg: None,
                face_profile: Vec::new(),
                mesh: Some(solid),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{SolidArmor, Target};
    use crate::armor::Armor;
    use crate::material_id::{MaterialId, STEEL_4340};
    use crate::validation::DomainError;

    fn solid() -> SolidArmor {
        SolidArmor::new("turret.stl", MaterialId::new(STEEL_4340))
    }

    #[test]
    fn default_solid_is_valid() {
        assert_eq!(solid().validate(), Ok(()));
        assert_eq!(Target::from(solid()).validate(), Ok(()));
    }

    #[test]
    fn solid_needs_a_file() {
        let mut solid = solid();
        solid.file = "  ".to_owned();
        assert_eq!(solid.validate(), Err(DomainError::NoMeshFile));
    }

    #[test]
    fn solid_needs_a_direction_a_scale_and_a_positive_depth() {
        let field = |solid: &SolidArmor| match solid.validate() {
            Err(DomainError::InvalidParameter(error)) => error.field,
            other => panic!("expected an invalid parameter, got {other:?}"),
        };
        let mut bad = solid();
        bad.direction = [0.0; 3];
        assert_eq!(field(&bad), "mesh.direction");
        bad.direction = [0.0, f64::NAN, 1.0];
        assert_eq!(field(&bad), "mesh.direction");
        let mut bad = solid();
        bad.metres_per_unit = 0.0;
        assert_eq!(field(&bad), "mesh.metres_per_unit");
        let mut bad = solid();
        bad.aim_point[2] = f64::INFINITY;
        assert_eq!(field(&bad), "mesh.aim_point");
        let mut bad = solid();
        bad.max_depth = Some(0.0);
        assert_eq!(field(&bad), "mesh.max_depth");
    }

    #[test]
    fn only_plates_have_an_obliquity() {
        let plates = Target::from(Armor::flat(Vec::new(), 0.5));
        assert!((plates.obliquity() - 0.5).abs() < f64::EPSILON);
        assert!(plates.plates().is_some() && plates.solid().is_none());
        let solid = Target::from(solid());
        assert!(solid.obliquity().abs() < f64::EPSILON);
        assert!(solid.plates().is_none() && solid.solid().is_some());
    }
}
