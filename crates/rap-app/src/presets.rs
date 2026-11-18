//! Ready-made scenarios to start from.

use rap_domain::material_id::{CAST_ARMOR_STEEL, STEEL_4340};
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
        face_profile: Vec::new(),
    }
}

/// A generic 76 mm capped shot striking a 60 mm steel plate at 700 m/s.
#[must_use]
pub fn apbc_against_steel() -> ScenarioFile {
    ScenarioFile::new(Scenario {
        projectile: Projectile::Apbc(ApbcProjectile::generic_76mm()),
        armor: steel_plate(0.06).into(),
        impact_speed: 700.0,
    })
}

/// A generic 120 mm-class long rod striking 400 mm of steel at 1600 m/s.
#[must_use]
pub fn apfsds_against_steel() -> ScenarioFile {
    ScenarioFile::new(Scenario {
        projectile: Projectile::Apfsds(ApfsdsProjectile::generic_120mm()),
        armor: steel_plate(0.4).into(),
        impact_speed: 1600.0,
    })
}

/// Obliquity of the turret cheek in [`aphe_against_cast_armor`]: a face
/// leaning back 30° and turned 30° away from the shot, in radians.
const COMPOUND_OBLIQUITY: f64 = 0.722_734_247_813_415_6;

/// A 152 mm shell with a bursting charge striking 245 mm of cast steel at 810 m/s
/// and a compound angle of 30° by 30°: the cheek of a heavy tank's cast
/// turret, about 325 mm along the line of sight.
#[must_use]
pub fn aphe_against_cast_armor() -> ScenarioFile {
    let mut armor = steel_plate(0.245);
    armor.obliquity = COMPOUND_OBLIQUITY;
    armor.layers[0].material = MaterialId::new(CAST_ARMOR_STEEL);
    let mut file = ScenarioFile::new(Scenario {
        projectile: Projectile::Apbc(ApbcProjectile::aphe_152mm()),
        armor: armor.into(),
        impact_speed: 810.0,
    });
    // A thick plate needs less width, in shell radii, than the default.
    file.settings.armor_extent_radii = 5.0;
    file
}
