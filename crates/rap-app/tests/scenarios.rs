//! Assembling scenarios into models and simulating them end to end.

use rap_app::{AppError, Simulation, assemble, presets};
use rap_domain::material_id::{STEEL_4340, TUNGSTEN_HEAVY_ALLOY};
use rap_domain::{
    ApfsdsProjectile, Armor, ArmorLayer, MaterialId, NoseShape, Projectile, Scenario,
    SimulationSettings,
};
use rap_materials::{EquationOfState, library};
use rap_solver::Symmetry;

fn rod(radius: f64, length: f64) -> Projectile {
    Projectile::Apfsds(ApfsdsProjectile {
        rod_diameter: 2.0 * radius,
        rod_length: length,
        nose: NoseShape::Cone {
            half_angle: 30.0_f64.to_radians(),
        },
        material: MaterialId::new(TUNGSTEN_HEAVY_ALLOY),
    })
}

fn plate(thickness: f64, gap_before: f64) -> ArmorLayer {
    ArmorLayer {
        material: MaterialId::new(STEEL_4340),
        thickness,
        gap_before,
    }
}

fn scenario(layers: Vec<ArmorLayer>, obliquity_degrees: f64) -> Scenario {
    Scenario {
        projectile: rod(4.0e-3, 40.0e-3),
        armor: Armor {
            layers,
            obliquity: obliquity_degrees.to_radians(),
        },
        impact_speed: 1500.0,
    }
}

#[test]
fn perpendicular_impact_becomes_an_axisymmetric_model_with_the_right_masses() {
    let settings = SimulationSettings::default();
    let assembly = assemble(&scenario(vec![plate(0.05, 0.0)], 0.0), &settings, &[])
        .expect("the scenario is valid");
    let (model, layout) = (&assembly.model, &assembly.layout);
    assert_eq!(layout.symmetry, Symmetry::Axisymmetric);
    assert_eq!(model.validate(), Ok(()));
    assert_eq!(
        (layout.projectile_body, layout.cap_body, layout.armor_body),
        (0, None, 1)
    );
    assert!((layout.cell_size - 1.0e-3).abs() < 1e-12);
    assert!((layout.armor_back - layout.armor_face - 0.05).abs() < 1e-12);

    // Volumes are per radian: the solid of revolution divided by 2π.
    let volume_of = |body: usize| -> f64 {
        model
            .seeds
            .iter()
            .filter(|seed| seed.body == body)
            .map(|seed| seed.volume)
            .sum()
    };
    let (radius, length) = (4.0e-3_f64, 40.0e-3);
    let nose_length = radius / 30.0_f64.to_radians().tan();
    let rod_volume = radius.powi(2) * (length + nose_length / 3.0) / 2.0;
    assert!(
        (volume_of(0) / rod_volume - 1.0).abs() < 0.05,
        "{}",
        volume_of(0) / rod_volume
    );
    let armor_radius = settings.armor_extent_radii * radius;
    let armor_volume = armor_radius.powi(2) * 0.05 / 2.0;
    assert!((volume_of(1) / armor_volume - 1.0).abs() < 0.01);

    // Projectile first, then armor; every projectile particle moves.
    assert_eq!(model.materials.len(), 2);
    assert_eq!(model.materials[0].id.as_str(), TUNGSTEN_HEAVY_ALLOY);
    assert!(
        model
            .seeds
            .iter()
            .all(|seed| (seed.body == 0) == (seed.velocity.x > 0.0))
    );
    let tip = model
        .seeds
        .iter()
        .filter(|seed| seed.body == 0)
        .map(|seed| seed.position.x);
    assert!(tip.fold(f64::MIN, f64::max) < layout.armor_face);
}

#[test]
fn spaced_layers_leave_a_gap_and_lengthen_the_shot_line() {
    let layers = vec![plate(0.01, 0.0), plate(0.03, 0.02)];
    let assembly = assemble(&scenario(layers, 0.0), &SimulationSettings::default(), &[])
        .expect("the scenario is valid");
    let layout = assembly.layout;
    assert!((layout.armor_back - layout.armor_face - 0.06).abs() < 1e-12);
    let in_gap = assembly.model.seeds.iter().filter(|seed| {
        let depth = seed.position.x - layout.armor_face;
        seed.body == layout.armor_body && depth > 0.0105 && depth < 0.0295
    });
    assert_eq!(in_gap.count(), 0);
}

#[test]
fn oblique_impact_becomes_a_planar_model_with_a_tilted_plate() {
    let assembly = assemble(
        &scenario(vec![plate(0.02, 0.0)], 60.0),
        &SimulationSettings::default(),
        &[],
    )
    .expect("the scenario is valid");
    let (model, layout) = (&assembly.model, &assembly.layout);
    assert_eq!(layout.symmetry, Symmetry::Planar);
    assert_eq!(model.validate(), Ok(()));
    // Line-of-sight thickness doubles at 60°.
    assert!((layout.armor_back - layout.armor_face - 0.04).abs() < 1e-9);

    // The plate leans towards the projectile above the shot line.
    let mean_x = |above: bool| {
        let (sum, count) = model
            .seeds
            .iter()
            .filter(|seed| seed.body == layout.armor_body)
            .filter(|seed| (seed.position.y > layout.shot_line) == above)
            .fold((0.0, 0.0), |(sum, count), seed| {
                (sum + seed.position.x, count + 1.0)
            });
        sum / count
    };
    assert!(mean_x(true) < mean_x(false));
    // No armor particle starts inside the projectile.
    let tip = model
        .seeds
        .iter()
        .filter(|seed| seed.body == layout.projectile_body)
        .map(|seed| seed.position.x)
        .fold(f64::MIN, f64::max);
    let overlapping = model.seeds.iter().filter(|seed| {
        seed.body == layout.armor_body
            && seed.position.x <= tip
            && (seed.position.y - layout.shot_line).abs() <= 4.0e-3
    });
    assert_eq!(overlapping.count(), 0);
}

#[test]
fn capped_shot_gets_a_separate_cap_body() {
    let file = presets::apbc_against_steel();
    let settings = SimulationSettings {
        cells_per_radius: 8.0,
        ..SimulationSettings::default()
    };
    let assembly = assemble(&file.scenario, &settings, &[]).expect("the preset is valid");
    let layout = assembly.layout;
    assert_eq!((layout.cap_body, layout.armor_body), (Some(1), 2));
    let count = |body: usize| {
        assembly
            .model
            .seeds
            .iter()
            .filter(|seed| seed.body == body)
            .count()
    };
    assert!(
        count(1) > 0,
        "the cap wall must be resolved at this resolution"
    );
    assert!(count(0) > 20 * count(1), "the cap is a thin shell");
}

#[test]
fn problems_are_reported() {
    let settings = SimulationSettings::default();
    let mut unknown = scenario(vec![plate(0.05, 0.0)], 0.0);
    unknown.armor.layers[0].material = MaterialId::new("unobtainium");
    assert_eq!(
        assemble(&unknown, &settings, &[]).unwrap_err(),
        AppError::UnknownMaterial(MaterialId::new("unobtainium"))
    );

    // The same identifier defined alongside the scenario is accepted.
    let mut custom = library::steel_4340();
    custom.id = MaterialId::new("unobtainium");
    assert!(assemble(&unknown, &settings, &[custom]).is_ok());

    let mut invalid = scenario(vec![plate(0.05, 0.0)], 0.0);
    invalid.impact_speed = -1.0;
    assert!(matches!(
        assemble(&invalid, &settings, &[]),
        Err(AppError::Invalid(_))
    ));

    let fine = SimulationSettings {
        cells_per_radius: 64.0,
        armor_extent_radii: 100.0,
        ..settings
    };
    assert!(matches!(
        assemble(&scenario(vec![plate(0.5, 0.0)], 0.0), &fine, &[]),
        Err(AppError::GridTooLarge { .. })
    ));
}

#[test]
fn custom_material_replaces_the_built_in_one() {
    let mut heavy = library::steel_4340();
    let rap_materials::Eos::MieGruneisen(mut eos) = heavy.eos else {
        panic!("built-in steel uses the Mie–Grüneisen equation of state");
    };
    eos.reference_density = 9000.0;
    heavy.eos = rap_materials::Eos::MieGruneisen(eos);
    let assembly = assemble(
        &scenario(vec![plate(0.05, 0.0)], 0.0),
        &SimulationSettings::default(),
        &[heavy],
    )
    .expect("the scenario is valid");
    let steel = &assembly.model.materials[1];
    assert!((steel.eos.reference_density() - 9000.0).abs() < 1e-9);
}

#[test]
fn rod_perforates_a_thin_plate_and_keeps_most_of_its_speed() {
    let settings = SimulationSettings {
        cells_per_radius: 3.0,
        armor_extent_radii: 6.0,
        ..SimulationSettings::default()
    };
    let mut simulation = Simulation::new(&scenario(vec![plate(0.008, 0.0)], 0.0), &settings, &[])
        .expect("the scenario is valid");
    let end_time = simulation.layout().end_time;
    simulation
        .advance_to(end_time)
        .expect("the simulation stays valid");
    assert!(simulation.is_finished());
    let summary = simulation.summary();
    eprintln!("thin plate: {summary:?}");
    assert!(summary.perforated);
    assert!(summary.residual_speed > 0.8 * 1500.0 && summary.residual_speed < 1500.0);
    assert!(summary.residual_mass_fraction > 0.5);
    assert!((summary.penetration_depth - 0.008).abs() < 1e-12);
}

#[test]
fn rod_is_stopped_by_a_thick_plate() {
    let settings = SimulationSettings {
        cells_per_radius: 3.0,
        armor_extent_radii: 6.0,
        ..SimulationSettings::default()
    };
    let mut simulation = Simulation::new(&scenario(vec![plate(0.12, 0.0)], 0.0), &settings, &[])
        .expect("the scenario is valid");
    let end_time = simulation.layout().end_time;
    simulation
        .advance_to(end_time)
        .expect("the simulation stays valid");
    let summary = simulation.summary();
    eprintln!("thick plate: {summary:?}");
    assert!(!summary.perforated);
    // A 40 mm rod at 1500 m/s penetrates roughly its own length.
    assert!(summary.penetration_depth > 0.02 && summary.penetration_depth < 0.07);
    assert!(summary.residual_speed.abs() < f64::EPSILON);
}
