//! Armor given by a mesh: assembling it into a whole solid model and
//! simulating it end to end.

use glam::DVec3;
use rap_app::{
    AppError, ModelKind, Simulation, assemble, assemble_mesh, assemble_solid, load_mesh,
};
use rap_domain::material_id::{STEEL_4340, TUNGSTEN_HEAVY_ALLOY};
use rap_domain::{
    ApfsdsProjectile, Armor, ArmorLayer, Backend, MaterialId, NoseShape, Projectile, Scenario,
    SimulationSettings, SolidArmor,
};
use rap_geometry::{MeshError, TriangleMesh};

const RADIUS: f64 = 4.0e-3;
const ROD_LENGTH: f64 = 24.0e-3;
const THICKNESS: f64 = 6.0e-3;

/// A plate 6 mm thick along `x` and 200 mm square, in millimetres, with the
/// origin at the centre of its struck face.
fn plate_mesh() -> TriangleMesh {
    TriangleMesh::cuboid(
        DVec3::new(0.0, -100.0, -100.0),
        DVec3::new(6.0, 100.0, 100.0),
    )
}

fn solid_armor() -> SolidArmor {
    SolidArmor::new("plate.stl", MaterialId::new(STEEL_4340))
}

/// A short rod against `armor`, coarse enough for a quick solid model.
fn scenario(armor: impl Into<rap_domain::Target>) -> (Scenario, SimulationSettings) {
    let scenario = Scenario {
        projectile: Projectile::Apfsds(ApfsdsProjectile {
            rod_diameter: 2.0 * RADIUS,
            rod_length: ROD_LENGTH,
            nose: NoseShape::Cone {
                half_angle: 30.0_f64.to_radians(),
            },
            material: MaterialId::new(TUNGSTEN_HEAVY_ALLOY),
        }),
        armor: armor.into(),
        impact_speed: 1500.0,
    };
    let settings = SimulationSettings {
        cells_per_radius: 2.0,
        armor_extent_radii: 5.0,
        ..SimulationSettings::default()
    };
    (scenario, settings)
}

/// The armor struck at `obliquity_degrees`, tilted like a plate array is:
/// about the axis across the picture.
fn tilted(obliquity_degrees: f64) -> SolidArmor {
    let (sin, cos) = obliquity_degrees.to_radians().sin_cos();
    SolidArmor {
        direction: [cos, 0.0, -sin],
        ..solid_armor()
    }
}

fn volume_of(model: &rap_solver::Model3, body: usize) -> f64 {
    model
        .seeds
        .iter()
        .filter(|seed| seed.body == body)
        .map(|seed| seed.volume)
        .sum()
}

#[test]
fn mesh_becomes_a_whole_solid_model() {
    let (scenario, settings) = scenario(solid_armor());
    assert_eq!(ModelKind::of(&scenario, &settings), ModelKind::Solid);
    let assembly =
        assemble_mesh(&scenario, &settings, &[], &plate_mesh()).expect("the scenario is valid");
    let (model, layout) = (&assembly.model, &assembly.layout);
    assert_eq!(layout.kind, ModelKind::Solid);
    assert_eq!(model.validate(), Ok(()));
    assert!((layout.armor_back - layout.armor_face - THICKNESS).abs() < 1e-12);

    // The plate is cut to the armor half-width on each side of the shot line.
    let half_width = settings.armor_extent_radii * RADIUS;
    let armor_volume = (2.0 * half_width).powi(2) * THICKNESS;
    assert!((volume_of(model, layout.armor_body) / armor_volume - 1.0).abs() < 1e-9);
    // The whole body of revolution: π·R²·(L + L_nose/3).
    let nose_length = RADIUS / 30.0_f64.to_radians().tan();
    let rod_volume = std::f64::consts::PI * RADIUS.powi(2) * (ROD_LENGTH + nose_length / 3.0);
    let ratio = volume_of(model, layout.projectile_body) / rod_volume;
    assert!((ratio - 1.0).abs() < 0.12, "{ratio}");

    // Both bodies are centred on the shot line, with the rod in front.
    for body in [layout.projectile_body, layout.armor_body] {
        let seeds = || model.seeds.iter().filter(move |seed| seed.body == body);
        let count = seeds().fold(0.0, |count, _| count + 1.0);
        let centre = seeds().map(|seed| seed.position).sum::<DVec3>() / count;
        assert!((centre.y - layout.shot_line).abs() < 1e-9);
        assert!((centre.z - layout.symmetry_plane).abs() < 1e-9);
    }
    let tip = model
        .seeds
        .iter()
        .filter(|seed| seed.body == layout.projectile_body)
        .map(|seed| seed.position.x)
        .fold(f64::MIN, f64::max);
    let gap = layout.armor_face - tip;
    assert!(
        gap > layout.cell_size && gap < 3.0 * layout.cell_size,
        "{gap}"
    );
}

#[test]
fn shot_is_placed_in_the_coordinates_of_the_file() {
    let mesh = plate_mesh();
    let thickness = |solid: SolidArmor| {
        let (scenario, settings) = scenario(solid);
        let layout = assemble_mesh(&scenario, &settings, &[], &mesh)
            .expect("the scenario is valid")
            .layout;
        layout.armor_back - layout.armor_face
    };
    // Anywhere on the plate, from either side, and from above or below.
    let mut off_centre = solid_armor();
    off_centre.aim_point = [-500.0, 30.0, -20.0];
    assert!((thickness(off_centre) - THICKNESS).abs() < 1e-12);
    let mut reversed = solid_armor();
    reversed.direction = [-2.0, 0.0, 0.0];
    assert!((thickness(reversed) - THICKNESS).abs() < 1e-12);
    // Line-of-sight thickness grows by 1/cos 60°.
    assert!((thickness(tilted(60.0)) - 2.0 * THICKNESS).abs() < 1e-9);
    // In inches the plate is 25.4 times thicker.
    let mut inches = solid_armor();
    inches.metres_per_unit = 0.0254;
    inches.max_depth = Some(0.01);
    assert!((thickness(inches) - 0.01).abs() < 1e-12);
}

#[test]
fn tilted_mesh_leans_towards_the_projectile_above_the_shot_line() {
    let (scenario, settings) = scenario(tilted(45.0));
    let assembly =
        assemble_mesh(&scenario, &settings, &[], &plate_mesh()).expect("the scenario is valid");
    let (model, layout) = (&assembly.model, &assembly.layout);
    assert_eq!(model.validate(), Ok(()));
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
    // The rod starts clear of the part of the plate that leans towards it.
    let reach = RADIUS + layout.cell_size;
    let nearest_armor = model
        .seeds
        .iter()
        .filter(|seed| seed.body == layout.armor_body)
        .filter(|seed| {
            (seed.position.y - layout.shot_line).hypot(seed.position.z - layout.symmetry_plane)
                < reach
        })
        .map(|seed| seed.position.x)
        .fold(f64::MAX, f64::min);
    let tip = model
        .seeds
        .iter()
        .filter(|seed| seed.body == layout.projectile_body)
        .map(|seed| seed.position.x)
        .fold(f64::MIN, f64::max);
    assert!(nearest_armor < layout.armor_face);
    assert!(nearest_armor - tip > layout.cell_size);
}

#[test]
fn depth_limit_leaves_out_what_is_further_back() {
    let (mut scenario, settings) = scenario(solid_armor());
    let whole =
        assemble_mesh(&scenario, &settings, &[], &plate_mesh()).expect("the scenario is valid");
    let mut limited = solid_armor();
    limited.max_depth = Some(0.004);
    scenario.armor = limited.into();
    let cut =
        assemble_mesh(&scenario, &settings, &[], &plate_mesh()).expect("the scenario is valid");
    assert!((cut.layout.armor_back - cut.layout.armor_face - 0.004).abs() < 1e-12);
    let armor = cut.layout.armor_body;
    let ratio = volume_of(&cut.model, armor) / volume_of(&whole.model, armor);
    assert!((ratio - 4.0 / 6.0).abs() < 1e-9, "{ratio}");
}

#[test]
fn problems_are_reported() {
    let mesh = plate_mesh();
    let (scenario_ok, settings) = scenario(solid_armor());
    let error = |solid: SolidArmor, mesh: &TriangleMesh| {
        let (scenario, settings) = scenario(solid);
        assemble_mesh(&scenario, &settings, &[], mesh).unwrap_err()
    };

    let mut beside = solid_armor();
    beside.aim_point = [0.0, 150.0, 0.0];
    assert_eq!(error(beside, &mesh), AppError::ShotMissesMesh);

    let mut open = mesh.triangles().to_vec();
    open.truncate(10);
    let open = TriangleMesh::new(open).expect("a valid mesh");
    assert_eq!(
        error(solid_armor(), &open),
        AppError::Mesh(MeshError::NotClosed)
    );

    let speck = TriangleMesh::cuboid(DVec3::splat(-0.05), DVec3::splat(0.05));
    assert_eq!(error(solid_armor(), &speck), AppError::EmptyMesh);

    let mut unknown = solid_armor();
    unknown.material = MaterialId::new("unobtainium");
    assert_eq!(
        error(unknown, &mesh),
        AppError::UnknownMaterial(MaterialId::new("unobtainium"))
    );

    let mut no_direction = solid_armor();
    no_direction.direction = [0.0; 3];
    assert!(matches!(error(no_direction, &mesh), AppError::Invalid(_)));

    // Each kind of armor has its own way to a model.
    assert_eq!(
        assemble(&scenario_ok, &settings, &[]).unwrap_err(),
        AppError::WrongArmorKind
    );
    assert_eq!(
        assemble_solid(&scenario_ok, &settings, &[]).unwrap_err(),
        AppError::WrongArmorKind
    );
    let plates = Armor::flat(
        vec![ArmorLayer {
            material: MaterialId::new(STEEL_4340),
            thickness: THICKNESS,
            gap_before: 0.0,
        }],
        0.0,
    );
    let (plate_scenario, _) = scenario(plates);
    assert_eq!(
        assemble_mesh(&plate_scenario, &settings, &[], &mesh).unwrap_err(),
        AppError::WrongArmorKind
    );

    // A simulation needs the mesh.
    assert!(matches!(
        Simulation::new(&scenario_ok, &settings, &[]),
        Err(AppError::MeshRequired)
    ));
}

#[test]
fn mesh_armor_runs_on_the_gpu_like_on_the_cpu() {
    let (scenario, mut settings) = scenario(tilted(45.0));
    settings.end_time = Some(3.0e-5);
    let mesh = plate_mesh();
    let run = |backend: Backend| {
        let settings = SimulationSettings {
            backend,
            ..settings.clone()
        };
        let mut simulation = Simulation::with_mesh(&scenario, &settings, &[], Some(&mesh))?;
        simulation.advance_to(3.0e-5)?;
        Ok::<_, AppError>(simulation)
    };
    let gpu = match run(Backend::Gpu) {
        Ok(simulation) => simulation,
        Err(AppError::Solver(rap_solver::SolverError::NoGpu)) => {
            eprintln!("skipped: no compatible GPU on this machine");
            return;
        }
        Err(error) => panic!("the GPU run failed: {error}"),
    };
    let cpu = run(Backend::Cpu).expect("the CPU run succeeds");
    assert!(gpu.solver_name().starts_with("GPU"));
    let (gpu, cpu) = (gpu.summary(), cpu.summary());
    eprintln!("GPU: {gpu:?}\nCPU: {cpu:?}");
    assert!((gpu.penetration_depth - cpu.penetration_depth).abs() < 1.0e-4);
    assert!((gpu.residual_speed / cpu.residual_speed - 1.0).abs() < 0.01);
    assert!((gpu.residual_mass_fraction - cpu.residual_mass_fraction).abs() < 0.01);
    assert!((gpu.armor_failed_fraction - cpu.armor_failed_fraction).abs() < 0.01);
}

#[test]
fn mesh_file_is_found_next_to_the_scenario() {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let text = rap_io::stl::to_text(&plate_mesh(), "plate");
    std::fs::write(directory.path().join("plate.stl"), text).expect("the file is written");

    let (mut scenario, settings) = scenario(solid_armor());
    let mesh = load_mesh(&scenario, directory.path())
        .expect("the file is readable")
        .expect("the armor is a mesh");
    assert_eq!(mesh, plate_mesh());
    assert!(assemble_mesh(&scenario, &settings, &[], &mesh).is_ok());

    scenario.armor = SolidArmor::new("missing.stl", MaterialId::new(STEEL_4340)).into();
    let error = load_mesh(&scenario, directory.path()).unwrap_err();
    assert!(matches!(&error, AppError::MeshFile(message) if message.contains("missing.stl")));

    // Plates need no file.
    let plates = Armor::flat(Vec::new(), 0.0);
    scenario.armor = plates.into();
    assert_eq!(load_mesh(&scenario, directory.path()), Ok(None));
}

#[test]
fn tilted_mesh_plate_is_perforated_like_the_same_plate_given_as_layers() {
    let run = |mut simulation: Simulation| {
        let end_time = simulation.layout().end_time;
        simulation
            .advance_to(end_time)
            .expect("the simulation stays valid");
        simulation
    };
    let (mesh_scenario, mut settings) = scenario(tilted(45.0));
    // The same time for both, and a mesh of exactly the piece of plate
    // that the layered model cuts out.
    settings.end_time = Some(6.0e-5);
    let mesh = TriangleMesh::cuboid(DVec3::new(0.0, -20.0, -20.0), DVec3::new(6.0, 20.0, 20.0));
    let from_mesh = run(
        Simulation::with_mesh(&mesh_scenario, &settings, &[], Some(&mesh))
            .expect("the scenario is valid"),
    );
    let plates = Armor::flat(
        vec![ArmorLayer {
            material: MaterialId::new(STEEL_4340),
            thickness: THICKNESS,
            gap_before: 0.0,
        }],
        45.0_f64.to_radians(),
    );
    let (plate_scenario, _) = scenario(plates);
    let from_plates =
        run(Simulation::new(&plate_scenario, &settings, &[]).expect("the scenario is valid"));

    let (mesh_summary, plate_summary) = (from_mesh.summary(), from_plates.summary());
    eprintln!("mesh: {mesh_summary:?}\nplates: {plate_summary:?}");
    assert!(mesh_summary.perforated && plate_summary.perforated);
    let speed_ratio = mesh_summary.residual_speed / plate_summary.residual_speed;
    assert!((speed_ratio - 1.0).abs() < 0.03, "{speed_ratio}");
    let mass_difference =
        mesh_summary.residual_mass_fraction - plate_summary.residual_mass_fraction;
    assert!(mass_difference.abs() < 0.05, "{mass_difference}");

    // The whole model has both halves, and stays symmetric about the plane
    // through the shot line although nothing enforces it.
    // (About twice the particles; the two lose slightly different debris.)
    assert!(5 * from_mesh.particle_count() > 9 * from_plates.particle_count());
    let particles = from_mesh.solid_particles().expect("a solid model");
    let rod = || particles.iter().filter(|particle| particle.body == 0);
    let along: f64 = rod()
        .map(|particle| particle.mass * particle.velocity.x)
        .sum();
    let across: f64 = rod()
        .map(|particle| particle.mass * particle.velocity.z)
        .sum();
    assert!(
        across.abs() < 1e-3 * along.abs(),
        "{across:e} against {along:e}"
    );
}
