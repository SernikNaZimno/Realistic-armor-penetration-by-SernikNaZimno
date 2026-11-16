//! The example scenarios shipped with the documentation stay valid.

use std::path::PathBuf;

use rap_app::{ModelKind, assemble, assemble_solid};
use rap_io::ScenarioFile;

fn example(name: &str) -> ScenarioFile {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/examples")
        .join(name);
    ScenarioFile::load(&path).unwrap_or_else(|error| panic!("{name}: {error}"))
}

#[test]
fn every_example_is_listed_here() {
    let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/examples");
    let mut names: Vec<String> = std::fs::read_dir(directory)
        .expect("the examples directory exists")
        .map(|entry| {
            entry
                .expect("a readable entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();
    assert_eq!(
        names,
        [
            "capped-shot-domed-plate.json",
            "ceramic-spaced-armor.json",
            "oblique-plate.json"
        ]
    );
}

#[test]
fn ceramic_spaced_armor_assembles_axisymmetrically() {
    let file = example("ceramic-spaced-armor.json");
    assert_eq!(
        ModelKind::of(&file.scenario, &file.settings),
        ModelKind::Axisymmetric
    );
    let assembly =
        assemble(&file.scenario, &file.settings, &file.materials).expect("a valid example");
    assert_eq!(assembly.model.validate(), Ok(()));
    assert_eq!(assembly.model.materials.len(), 3);
    // 10 + 10 mm, a 20 mm gap, 30 mm.
    let layout = assembly.layout;
    assert!((layout.armor_back - layout.armor_face - 0.07).abs() < 1e-12);
}

#[test]
fn capped_shot_uses_its_own_material_and_a_shaped_plate() {
    let file = example("capped-shot-domed-plate.json");
    assert_eq!(file.materials.len(), 1);
    assert_eq!(file.materials[0].validate(), Ok(()));
    let assembly =
        assemble(&file.scenario, &file.settings, &file.materials).expect("a valid example");
    assert_eq!(assembly.model.validate(), Ok(()));
    assert_eq!(assembly.model.materials[0].id.as_str(), "hard-steel");
    // At this resolution the 4 mm cap wall is resolved as its own body.
    assert_eq!(assembly.layout.cap_body, Some(1));
    assert!(assembly.model.seeds.iter().any(|seed| seed.body == 1));
    assert!((assembly.layout.end_time - 6.0e-4).abs() < 1e-15);
}

#[test]
fn oblique_plate_assembles_as_a_solid_half_model() {
    let file = example("oblique-plate.json");
    assert_eq!(
        ModelKind::of(&file.scenario, &file.settings),
        ModelKind::Solid
    );
    let assembly =
        assemble_solid(&file.scenario, &file.settings, &file.materials).expect("a valid example");
    assert_eq!(assembly.model.validate(), Ok(()));
    assert!((assembly.layout.armor_back - assembly.layout.armor_face - 0.024).abs() < 1e-9);
}
