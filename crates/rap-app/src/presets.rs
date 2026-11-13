//! Ready-made scenarios to start from.

use rap_domain::material_id::STEEL_4340;
use rap_domain::{
    ApbcProjectile, ApfsdsProjectile, Armor, ArmorLayer, MaterialId, Projectile, Scenario,
};
use rap_io::ScenarioFile;

fn steel_plate(thickness: f64) -> Armor {
    Armor {
        layers: vec![ArmorLayer {
            material: MaterialId::new(STEEL_4340),
            thickness,
            gap_before: 0.0,
        }],
        obliquity: 0.0,
    }
}

/// A generic 76 mm capped shot striking a 60 mm steel plate at 700 m/s.
#[must_use]
pub fn apbc_against_steel() -> ScenarioFile {
    ScenarioFile::new(Scenario {
        projectile: Projectile::Apbc(ApbcProjectile::generic_76mm()),
        armor: steel_plate(0.06),
        impact_speed: 700.0,
    })
}

/// A generic 120 mm-class long rod striking 400 mm of steel at 1600 m/s.
#[must_use]
pub fn apfsds_against_steel() -> ScenarioFile {
    ScenarioFile::new(Scenario {
        projectile: Projectile::Apfsds(ApfsdsProjectile::generic_120mm()),
        armor: steel_plate(0.4),
        impact_speed: 1600.0,
    })
}
