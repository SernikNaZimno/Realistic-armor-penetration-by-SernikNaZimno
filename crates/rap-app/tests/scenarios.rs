//! Assembling scenarios into models and simulating them end to end.

use rap_app::{AppError, ModelKind, Simulation, assemble, assemble_solid, presets};
use rap_domain::material_id::{STEEL_4340, TUNGSTEN_HEAVY_ALLOY};
use rap_domain::{
    ApfsdsProjectile, Armor, ArmorLayer, MaterialId, NoseShape, ObliqueModel, Projectile, Scenario,
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
            face_profile: Vec::new(),
        }
        .into(),
        impact_speed: 1500.0,
    }
}

fn plates(scenario: &mut Scenario) -> &mut Armor {
    scenario.armor.plates_mut().expect("an array of plates")
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
    let settings = SimulationSettings {
        oblique_model: ObliqueModel::PlaneStrain,
        ..SimulationSettings::default()
    };
    let assembly = assemble(&scenario(vec![plate(0.02, 0.0)], 60.0), &settings, &[])
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
    plates(&mut unknown).layers[0].material = MaterialId::new("unobtainium");
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

#[test]
fn face_profile_shapes_every_layer() {
    use rap_domain::ProfilePoint;
    let mut shaped = scenario(vec![plate(0.01, 0.0), plate(0.02, 0.0)], 0.0);
    // Flat within 10 mm of the shot line, then sloping 10 mm back by 20 mm
    // and 5 mm forward of the centre from 30 mm outwards.
    plates(&mut shaped).face_profile = vec![
        ProfilePoint {
            lateral: 0.010,
            offset: 0.0,
        },
        ProfilePoint {
            lateral: 0.020,
            offset: 0.010,
        },
        ProfilePoint {
            lateral: 0.030,
            offset: -0.005,
        },
    ];
    let settings = SimulationSettings::default();
    let assembly = assemble(&shaped, &settings, &[]).expect("the scenario is valid");
    let layout = assembly.layout;
    let spacing = layout.cell_size / 2.0;
    // Position of the face at a given radius: the smallest x of armor there.
    let face_at = |radius: f64| {
        assembly
            .model
            .seeds
            .iter()
            .filter(|seed| seed.body == layout.armor_body)
            .filter(|seed| (seed.position.y - layout.shot_line - radius).abs() < spacing)
            .map(|seed| seed.position.x - layout.armor_face)
            .fold(f64::MAX, f64::min)
    };
    assert!(face_at(0.005).abs() < spacing);
    assert!((face_at(0.020) - 0.010).abs() < spacing);
    assert!((face_at(0.035) - -0.005).abs() < spacing);
    // The shot line still crosses the unshifted 30 mm of armor.
    assert!((layout.armor_back - layout.armor_face - 0.03).abs() < 1e-12);
    // The protruding rim does not start inside the projectile.
    let tip = assembly
        .model
        .seeds
        .iter()
        .filter(|seed| seed.body == layout.projectile_body)
        .map(|seed| seed.position.x)
        .fold(f64::MIN, f64::max);
    assert!(tip < layout.armor_face - 0.005);
    assert_eq!(assembly.model.validate(), Ok(()));
}

#[test]
fn gpu_backend_gives_the_same_outcome_as_the_cpu() {
    use rap_domain::Backend;
    use rap_solver::SolverError;

    let scenario = scenario(vec![plate(0.008, 0.0)], 0.0);
    let cpu_settings = SimulationSettings {
        cells_per_radius: 3.0,
        armor_extent_radii: 6.0,
        ..SimulationSettings::default()
    };
    let gpu_settings = SimulationSettings {
        backend: Backend::Gpu,
        ..cpu_settings.clone()
    };
    let mut gpu = match Simulation::new(&scenario, &gpu_settings, &[]) {
        Ok(simulation) => simulation,
        Err(AppError::Solver(SolverError::NoGpu)) => {
            eprintln!("skipped: no compatible GPU on this machine");
            return;
        }
        Err(error) => panic!("the GPU simulation could not be created: {error}"),
    };
    assert!(gpu.solver_name().starts_with("GPU"));
    let mut cpu = Simulation::new(&scenario, &cpu_settings, &[]).expect("the scenario is valid");
    assert_eq!(cpu.solver_name(), "CPU");

    let end_time = cpu.layout().end_time;
    cpu.advance_to(end_time)
        .expect("the CPU simulation stays valid");
    gpu.advance_to(end_time)
        .expect("the GPU simulation stays valid");
    let (cpu_summary, gpu_summary) = (cpu.summary(), gpu.summary());
    eprintln!("cpu: {cpu_summary:?}\ngpu: {gpu_summary:?}");
    assert!(gpu.is_finished());
    assert_eq!(gpu_summary.perforated, cpu_summary.perforated);
    assert!((gpu_summary.residual_speed / cpu_summary.residual_speed - 1.0).abs() < 0.02);
    assert!((gpu_summary.residual_mass_fraction - cpu_summary.residual_mass_fraction).abs() < 0.02);
    // Material that flew out of the grid is still accounted for.
    assert!(!gpu.departed().is_empty());
}

/// A short rod and a thin plate, coarse enough for a quick solid model.
fn small_oblique_scenario(obliquity_degrees: f64) -> (Scenario, SimulationSettings) {
    let mut scenario = scenario(vec![plate(0.006, 0.0)], obliquity_degrees);
    scenario.projectile = rod(4.0e-3, 24.0e-3);
    let settings = SimulationSettings {
        cells_per_radius: 2.0,
        armor_extent_radii: 5.0,
        ..SimulationSettings::default()
    };
    (scenario, settings)
}

#[test]
fn oblique_impact_becomes_a_solid_half_model_by_default() {
    let (scenario, settings) = small_oblique_scenario(45.0);
    assert_eq!(ModelKind::of(&scenario, &settings), ModelKind::Solid);
    let assembly = assemble_solid(&scenario, &settings, &[]).expect("the scenario is valid");
    let (model, layout) = (&assembly.model, &assembly.layout);
    assert_eq!(layout.kind, ModelKind::Solid);
    assert_eq!(model.validate(), Ok(()));
    // Line-of-sight thickness grows by 1/cos 45°.
    assert!((layout.armor_back - layout.armor_face - 0.006 * 2.0_f64.sqrt()).abs() < 1e-9);

    // Half of the body of revolution: π·R²·(L + L_nose/3)/2.
    let (radius, length) = (4.0e-3_f64, 24.0e-3);
    let nose_length = radius / 30.0_f64.to_radians().tan();
    let expected = std::f64::consts::PI * radius.powi(2) * (length + nose_length / 3.0) / 2.0;
    let rod_volume: f64 = model
        .seeds
        .iter()
        .filter(|seed| seed.body == layout.projectile_body)
        .map(|seed| seed.volume)
        .sum();
    assert!(
        (rod_volume / expected - 1.0).abs() < 0.12,
        "{}",
        rod_volume / expected
    );

    // Everything lies on one side of the symmetry plane, and the plate
    // leans towards the projectile above the shot line.
    assert!(
        model
            .seeds
            .iter()
            .all(|seed| seed.position.z > layout.symmetry_plane)
    );
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
}

#[test]
fn solid_oblique_impact_runs_and_deflects_the_projectile() {
    let (scenario, settings) = small_oblique_scenario(45.0);
    let mut simulation = Simulation::new(&scenario, &settings, &[]).expect("the scenario is valid");
    assert_eq!(simulation.layout().kind, ModelKind::Solid);
    assert!(simulation.solid_particles().is_some());
    // The picture is a slice: far fewer particles than the model has.
    assert!(simulation.particles().len() * 4 < simulation.particle_count());

    let end_time = simulation.layout().end_time;
    simulation
        .advance_to(end_time)
        .expect("the simulation stays valid");
    let summary = simulation.summary();
    eprintln!("solid oblique: {summary:?}");
    assert!(summary.perforated);
    assert!(summary.residual_speed > 0.6 * 1500.0 && summary.residual_speed < 1500.0);

    // The tilted plate pushes the rod sideways, away from the plate normal:
    // an asymmetry no axisymmetric model can show.
    let solid = simulation.solid_particles().expect("a solid model");
    let departed = simulation.departed();
    let lateral: f64 = solid
        .iter()
        .filter(|particle| particle.body == 0)
        .map(|particle| particle.mass * particle.velocity.y)
        .sum();
    eprintln!(
        "lateral momentum of the remaining rod: {lateral:.3e}, departed slice: {}",
        departed.len()
    );
    assert!(lateral.abs() > 0.0);
    // Nothing crossed the symmetry plane.
    assert!(solid.iter().all(|particle| particle.position.z > 0.0));
}

/// Runs `scenario` to `end_time` on both solvers and returns their
/// summaries, GPU first, or `None` with a note on a machine without a GPU.
fn summaries_on_both(
    scenario: &Scenario,
    settings: &SimulationSettings,
    end_time: f64,
) -> Option<(rap_analysis::ImpactSummary, rap_analysis::ImpactSummary)> {
    use rap_domain::Backend;
    use rap_solver::SolverError;
    let run = |backend: Backend| {
        let settings = SimulationSettings {
            backend,
            ..settings.clone()
        };
        let mut simulation = Simulation::new(scenario, &settings, &[])?;
        simulation.advance_to(end_time)?;
        Ok::<_, AppError>(simulation)
    };
    let gpu = match run(Backend::Gpu) {
        Ok(simulation) => simulation,
        Err(AppError::Solver(SolverError::NoGpu)) => {
            eprintln!("skipped: no compatible GPU on this machine");
            return None;
        }
        Err(error) => panic!("the GPU run failed: {error}"),
    };
    assert!(gpu.solver_name().starts_with("GPU"));
    let cpu = run(Backend::Cpu).expect("the CPU run succeeds");
    assert_eq!(cpu.solver_name(), "CPU");
    Some((gpu.summary(), cpu.summary()))
}

#[test]
fn solid_oblique_impact_runs_on_the_gpu_like_on_the_cpu() {
    let (scenario, settings) = small_oblique_scenario(45.0);
    let Some((gpu, cpu)) = summaries_on_both(&scenario, &settings, 3.0e-5) else {
        return;
    };
    eprintln!("solid oblique\nGPU: {gpu:?}\nCPU: {cpu:?}");
    assert!(cpu.perforated && gpu.perforated);
    assert!((gpu.residual_speed / cpu.residual_speed - 1.0).abs() < 0.01);
    assert!((gpu.residual_mass_fraction - cpu.residual_mass_fraction).abs() < 0.01);
    assert!((gpu.armor_failed_fraction - cpu.armor_failed_fraction).abs() < 0.01);
}

#[test]
fn ceramic_faced_armor_fractures_under_the_rod() {
    use rap_domain::material_id::ALUMINA;
    let ceramic = ArmorLayer {
        material: MaterialId::new(ALUMINA),
        thickness: 0.008,
        gap_before: 0.0,
    };
    let scenario = scenario(vec![ceramic, plate(0.008, 0.0)], 0.0);
    let settings = SimulationSettings {
        cells_per_radius: 3.0,
        armor_extent_radii: 6.0,
        ..SimulationSettings::default()
    };
    let assembly = assemble(&scenario, &settings, &[]).expect("the scenario is valid");
    let names: Vec<_> = assembly
        .model
        .materials
        .iter()
        .map(|material| material.id.as_str())
        .collect();
    assert_eq!(names, [TUNGSTEN_HEAVY_ALLOY, ALUMINA, STEEL_4340]);

    let mut simulation = Simulation::new(&scenario, &settings, &[]).expect("the scenario is valid");
    let end_time = simulation.layout().end_time;
    simulation
        .advance_to(end_time)
        .expect("the simulation stays valid");
    let summary = simulation.summary();
    eprintln!("ceramic-faced: {summary:?}");
    // The tile under the rod is comminuted; the steel behind is not.
    let particles = simulation.particles();
    let failed_fraction = |material: usize| {
        let (failed, all) = particles
            .iter()
            .chain(simulation.departed().iter())
            .filter(|particle| particle.material == material)
            .fold((0.0, 0.0), |(failed, all), particle| {
                (
                    failed + f64::from(u8::from(particle.is_failed())),
                    all + 1.0,
                )
            });
        failed / all
    };
    assert!(
        failed_fraction(1) > 0.2,
        "ceramic failed fraction {}",
        failed_fraction(1)
    );
    assert!(failed_fraction(1) > 2.0 * failed_fraction(2));
    assert!(summary.penetration_depth > 0.008);
}

#[test]
fn ceramic_faced_armor_runs_on_the_gpu_like_on_the_cpu() {
    use rap_domain::material_id::ALUMINA;
    let ceramic = ArmorLayer {
        material: MaterialId::new(ALUMINA),
        thickness: 0.008,
        gap_before: 0.0,
    };
    let scenario = scenario(vec![ceramic, plate(0.008, 0.0)], 0.0);
    let settings = SimulationSettings {
        cells_per_radius: 3.0,
        armor_extent_radii: 6.0,
        ..SimulationSettings::default()
    };
    let Some((gpu, cpu)) = summaries_on_both(&scenario, &settings, 2.0e-5) else {
        return;
    };
    eprintln!("ceramic-faced\nGPU: {gpu:?}\nCPU: {cpu:?}");
    assert!(cpu.penetration_depth > 0.008);
    assert!((gpu.penetration_depth / cpu.penetration_depth - 1.0).abs() < 0.02);
    assert!((gpu.armor_failed_fraction - cpu.armor_failed_fraction).abs() < 0.01);
    assert!((gpu.projectile_failed_fraction - cpu.projectile_failed_fraction).abs() < 0.02);
}

#[test]
fn shell_with_a_bursting_charge_has_its_mass_and_a_bonded_filler() {
    use rap_domain::material_id::{CAST_ARMOR_STEEL, SHELL_STEEL, TNT};
    let mut file = presets::aphe_against_cast_armor();
    file.settings.cells_per_radius = 8.0;
    file.settings.armor_extent_radii = 2.0;
    // Perpendicular, so that the model is axisymmetric and quick to build.
    plates(&mut file.scenario).obliquity = 0.0;
    let assembly = assemble(&file.scenario, &file.settings, &[]).expect("the preset is valid");
    let (model, layout) = (&assembly.model, &assembly.layout);
    assert_eq!(model.validate(), Ok(()));
    let names: Vec<_> = model
        .materials
        .iter()
        .map(|material| material.id.as_str())
        .collect();
    assert_eq!(names, [SHELL_STEEL, TNT, CAST_ARMOR_STEEL, "tnt-products"]);

    // The charge goes off a millisecond after the nose reaches the plate.
    let detonation = model.config.detonation.expect("the charge has a fuze");
    assert_eq!((detonation.material, detonation.products), (1, 3));
    let tip = model
        .seeds
        .iter()
        .filter(|seed| seed.body == layout.projectile_body)
        .map(|seed| seed.position.x)
        .fold(f64::MIN, f64::max);
    let flight = (layout.armor_face - tip) / file.scenario.impact_speed;
    assert!(
        (detonation.time - 1.0e-3 - flight).abs() < 5.0e-6,
        "{}",
        detonation.time
    );
    assert!((detonation.energy / 4.29e6 - 1.0).abs() < 0.01);

    // Volumes are per radian.
    let mass_of = |material: usize| -> f64 {
        model
            .seeds
            .iter()
            .filter(|seed| seed.body == layout.projectile_body && seed.material == material)
            .map(|seed| seed.volume * model.materials[material].eos.reference_density())
            .sum::<f64>()
            * std::f64::consts::TAU
    };
    let (steel, charge) = (mass_of(0), mass_of(1));
    eprintln!("shell: {steel:.2} kg of steel, {charge:.3} kg of charge");
    // The BR-540 weighs 48.8 kg; the charge is a cylinder 50 mm by 120 mm.
    assert!(((steel + charge) / 48.8 - 1.0).abs() < 0.03);
    let expected_charge = std::f64::consts::PI * 0.025_f64.powi(2) * 0.12 * 1630.0;
    // At this resolution the cavity is five particles across.
    assert!((charge / expected_charge - 1.0).abs() < 0.15);

    // The charge sits on the axis, behind the nose and ahead of the plug.
    let base = model
        .seeds
        .iter()
        .filter(|seed| seed.body == layout.projectile_body)
        .map(|seed| seed.position.x)
        .fold(f64::MAX, f64::min);
    for seed in model.seeds.iter().filter(|seed| seed.material == 1) {
        assert_eq!(seed.body, layout.projectile_body);
        assert!(seed.position.y - layout.shot_line < 0.025);
        let ahead = seed.position.x - base;
        assert!(ahead > 0.029 && ahead < 0.151, "{ahead}");
    }
}

#[test]
fn shell_preset_is_a_solid_model_at_its_compound_angle() {
    let file = presets::aphe_against_cast_armor();
    assert_eq!(
        ModelKind::of(&file.scenario, &file.settings),
        ModelKind::Solid
    );
    let assembly =
        assemble_solid(&file.scenario, &file.settings, &[]).expect("the preset is valid");
    assert_eq!(assembly.model.validate(), Ok(()));
    // 245 mm at 30° by 30° is about 325 mm along the line of sight.
    let path = assembly.layout.armor_back - assembly.layout.armor_face;
    assert!((path - 0.3267).abs() < 1e-3, "{path}");
}

#[test]
fn a_fuze_needs_an_explosive_and_a_charge_without_one_stays_inert() {
    use rap_domain::material_id::TNT_INERT;
    fn charge(file: &mut rap_io::ScenarioFile) -> &mut rap_domain::Filler {
        let Projectile::Apbc(shot) = &mut file.scenario.projectile else {
            panic!("the preset fires a full-caliber shell");
        };
        shot.filler.as_mut().expect("the preset has a charge")
    }
    let mut file = presets::aphe_against_cast_armor();
    plates(&mut file.scenario).obliquity = 0.0;

    charge(&mut file).material = MaterialId::new(TNT_INERT);
    let error = assemble(&file.scenario, &file.settings, &[]).expect_err("an inert charge");
    assert!(matches!(error, AppError::InertCharge(_)), "{error}");

    charge(&mut file).fuze_delay = None;
    let assembly = assemble(&file.scenario, &file.settings, &[]).expect("a valid scenario");
    assert_eq!(assembly.model.config.detonation, None);
    assert_eq!(assembly.model.materials.len(), 3);
}

#[test]
fn stand_off_moves_the_projectile_back_and_the_margin_widens_the_grid() {
    let mut file = presets::aphe_against_cast_armor();
    file.settings.cells_per_radius = 4.0;
    plates(&mut file.scenario).obliquity = 0.0;
    let build = |file: &rap_io::ScenarioFile| {
        assemble(&file.scenario, &file.settings, &[]).expect("a valid scenario")
    };
    let tip = |assembly: &rap_app::Assembly| {
        assembly
            .model
            .seeds
            .iter()
            .filter(|seed| seed.body == assembly.layout.projectile_body)
            .map(|seed| seed.position.x)
            .fold(f64::MIN, f64::max)
    };
    let close = build(&file);
    let cell = close.layout.cell_size;
    let radius = 0.076;

    // 200 mm further away: the gap grows by that much, the charge goes off
    // later by the time it takes to cross it, and the run lasts as much
    // longer.
    file.settings.standoff = 0.2;
    let far = build(&file);
    let gap = |assembly: &rap_app::Assembly| assembly.layout.armor_face - tip(assembly);
    assert!((gap(&far) - gap(&close) - 0.2).abs() < cell);
    let time = |assembly: &rap_app::Assembly| {
        assembly
            .model
            .config
            .detonation
            .expect("the charge has a fuze")
            .time
    };
    assert!((time(&far) - time(&close) - 0.2 / 810.0).abs() < 1e-12);
    assert!((far.layout.end_time - close.layout.end_time - 0.2 / 810.0).abs() < 1e-12);
    assert_eq!(far.model.seeds.len(), close.model.seeds.len());

    // Three radii of free space on every open side: behind the projectile
    // and past the armor along the shot line, and beyond the armor's edge.
    file.settings.standoff = 0.0;
    file.settings.margin_radii = 3.0;
    let wide = build(&file);
    #[expect(
        clippy::cast_precision_loss,
        reason = "grids are far smaller than 2^52 cells"
    )]
    let grown =
        |axis: usize| (wide.model.grid.cells[axis] - close.model.grid.cells[axis]) as f64 * cell;
    assert!(
        (grown(0) - 2.0 * 3.0 * radius).abs() <= 2.0 * cell,
        "{}",
        grown(0)
    );
    assert!((grown(1) - 3.0 * radius).abs() <= cell, "{}", grown(1));
    assert!((wide.layout.armor_face - close.layout.armor_face - 3.0 * radius).abs() < cell);
    assert_eq!(wide.model.seeds.len(), close.model.seeds.len());
    assert_eq!(wide.model.validate(), Ok(()));
}
