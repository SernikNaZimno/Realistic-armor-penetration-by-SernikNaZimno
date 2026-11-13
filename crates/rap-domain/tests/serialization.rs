//! JSON form of the domain types.

use rap_domain::material_id::STEEL_4340;
use rap_domain::{
    ApbcProjectile, ApfsdsProjectile, Armor, ArmorLayer, MaterialId, NoseShape, Projectile,
    Scenario,
};

fn scenario(projectile: Projectile) -> Scenario {
    Scenario {
        projectile,
        armor: Armor {
            layers: vec![
                ArmorLayer {
                    material: MaterialId::new(STEEL_4340),
                    thickness: 0.03,
                    gap_before: 0.0,
                },
                ArmorLayer {
                    material: MaterialId::new(STEEL_4340),
                    thickness: 0.08,
                    gap_before: 0.05,
                },
            ],
            obliquity: 60.0_f64.to_radians(),
        },
        impact_speed: 1600.0,
    }
}

/// Angles pass through degrees, so they come back only to rounding error.
fn assert_same(actual: &Scenario, expected: &Scenario) {
    assert!((actual.armor.obliquity - expected.armor.obliquity).abs() < 1e-12);
    let mut actual = actual.clone();
    actual.armor.obliquity = expected.armor.obliquity;
    if let (Projectile::Apfsds(rod), Projectile::Apfsds(reference)) =
        (&mut actual.projectile, &expected.projectile)
    {
        if let (
            NoseShape::Cone { half_angle },
            NoseShape::Cone {
                half_angle: reference,
            },
        ) = (&mut rod.nose, reference.nose)
        {
            assert!((*half_angle - reference).abs() < 1e-12);
            *half_angle = reference;
        }
    }
    assert_eq!(&actual, expected);
}

#[test]
fn scenarios_survive_a_round_trip() {
    for projectile in [
        Projectile::Apbc(ApbcProjectile::generic_76mm()),
        Projectile::Apfsds(ApfsdsProjectile::generic_120mm()),
    ] {
        let original = scenario(projectile);
        let json = serde_json::to_string_pretty(&original).expect("scenarios serialize");
        let restored: Scenario = serde_json::from_str(&json).expect("scenarios deserialize");
        assert_same(&restored, &original);
    }
}

#[test]
fn angles_are_written_in_degrees_and_types_are_tagged() {
    let json = serde_json::to_value(scenario(Projectile::Apfsds(
        ApfsdsProjectile::generic_120mm(),
    )))
    .expect("scenarios serialize");
    assert_eq!(json["projectile"]["type"], "apfsds");
    assert_eq!(json["projectile"]["nose"]["shape"], "cone");
    let half_angle = json["projectile"]["nose"]["half_angle_deg"]
        .as_f64()
        .expect("a number");
    assert!((half_angle - 15.0).abs() < 1e-9);
    let obliquity = json["armor"]["obliquity_deg"].as_f64().expect("a number");
    assert!((obliquity - 60.0).abs() < 1e-9);
    assert_eq!(json["armor"]["layers"][0]["material"], STEEL_4340);
}

#[test]
fn hand_written_scenario_is_accepted_and_typos_are_rejected() {
    let text = r#"{
        "projectile": {
            "type": "apbc",
            "caliber": 0.1,
            "body_length": 0.25,
            "nose": { "shape": "ogive", "caliber_radius_head": 1.5 },
            "cap": null,
            "material": "steel-4340"
        },
        "armor": {
            "layers": [{ "material": "steel-4340", "thickness": 0.1 }],
            "obliquity_deg": 30
        },
        "impact_speed": 800
    }"#;
    let parsed: Scenario = serde_json::from_str(text).expect("a valid scenario");
    assert_eq!(parsed.validate(), Ok(()));
    assert!((parsed.armor.obliquity.to_degrees() - 30.0).abs() < 1e-12);
    assert!(parsed.armor.layers[0].gap_before.abs() < f64::EPSILON);

    let typo = text.replace("impact_speed", "impact_sped");
    assert!(serde_json::from_str::<Scenario>(&typo).is_err());
    let typo = text.replace("body_length", "body_lenght");
    assert!(serde_json::from_str::<Scenario>(&typo).is_err());
}
