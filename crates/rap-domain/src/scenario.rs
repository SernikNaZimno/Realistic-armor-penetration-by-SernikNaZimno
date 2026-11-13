//! A complete description of one impact to simulate.

use serde::{Deserialize, Serialize};

use crate::armor::Armor;
use crate::projectile::Projectile;
use crate::validation::{self, DomainError};

/// One projectile striking one armor array.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scenario {
    /// The striking projectile.
    pub projectile: Projectile,
    /// The target.
    pub armor: Armor,
    /// Projectile speed at impact, in metres per second.
    pub impact_speed: f64,
}

impl Scenario {
    /// Checks the projectile, the armor and the impact speed.
    ///
    /// # Errors
    /// Returns the first [`DomainError`] found.
    pub fn validate(&self) -> Result<(), DomainError> {
        self.projectile.validate()?;
        self.armor.validate()?;
        validation::positive("impact_speed", self.impact_speed)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::Scenario;
    use crate::armor::{Armor, ArmorLayer};
    use crate::material_id::{MaterialId, STEEL_4340};
    use crate::projectile::{ApfsdsProjectile, Projectile};

    fn scenario(impact_speed: f64) -> Scenario {
        Scenario {
            projectile: Projectile::Apfsds(ApfsdsProjectile::generic_120mm()),
            armor: Armor {
                layers: vec![ArmorLayer {
                    material: MaterialId::new(STEEL_4340),
                    thickness: 0.3,
                    gap_before: 0.0,
                }],
                obliquity: 0.0,
            },
            impact_speed,
        }
    }

    #[test]
    fn valid_scenario_passes() {
        assert_eq!(scenario(1600.0).validate(), Ok(()));
    }

    #[test]
    fn impact_speed_must_be_positive() {
        assert!(scenario(0.0).validate().is_err());
    }
}
