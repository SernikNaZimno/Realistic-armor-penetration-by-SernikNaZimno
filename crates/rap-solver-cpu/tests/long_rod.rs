//! Long-rod penetration: a tungsten-alloy rod strikes a thick steel target
//! along the axis of an axisymmetric model. The depth of penetration is
//! compared with the Alekseevskii–Tate model.

use glam::DVec2;
use rap_analysis::tate::TateRod;
use rap_analysis::{Totals, bounding_box_where};
use rap_geometry::fill_rectangle;
use rap_materials::{EquationOfState, library};
use rap_solver::{Grid, Model, ParticleSeed, Solver, SolverConfig, Symmetry, Wall, Walls};
use rap_solver_cpu::MpmSolver;

const CELL: f64 = 1.0e-3;
const SPACING: f64 = CELL / 2.0;
const ROD_RADIUS: f64 = 4.0e-3;
const ROD_LENGTH: f64 = 80.0e-3;
const TARGET_RADIUS: f64 = 40.0e-3;
const TARGET_THICKNESS: f64 = 150.0e-3;
const IMPACT_SPEED: f64 = 1500.0;
const ROD: usize = 0;
const TARGET: usize = 1;

struct Setup {
    solver: MpmSolver,
    target_face: f64,
}

fn rod_against_target() -> Setup {
    let grid = Grid {
        cells: [242, 50],
        cell_size: CELL,
    };
    let (lower, _) = grid.interior();
    let axis = lower.y;
    let rod_tail = lower.x + 2.0 * CELL;
    let target_face = rod_tail + ROD_LENGTH + CELL;
    let body = |from: f64, length: f64, radius: f64, speed: f64, index: usize| {
        fill_rectangle(
            DVec2::new(from, axis),
            DVec2::new(from + length, axis + radius),
            SPACING,
        )
        .into_iter()
        .map(move |position| ParticleSeed {
            position,
            velocity: DVec2::new(speed, 0.0),
            // Per radian: cross-section area times radius.
            volume: SPACING * SPACING * (position.y - axis),
            material: index,
            body: index,
        })
    };
    let seeds = body(rod_tail, ROD_LENGTH, ROD_RADIUS, IMPACT_SPEED, ROD)
        .chain(body(
            target_face,
            TARGET_THICKNESS,
            TARGET_RADIUS,
            0.0,
            TARGET,
        ))
        .collect();
    let model = Model {
        grid,
        symmetry: Symmetry::Axisymmetric,
        walls: Walls {
            bottom: Wall::Sliding,
            ..Walls::default()
        },
        config: SolverConfig::default(),
        materials: vec![library::tungsten_heavy_alloy(), library::steel_4340()],
        seeds,
        initial_temperature: 294.0,
    };
    Setup {
        solver: MpmSolver::new(model).expect("the rod model is valid"),
        target_face,
    }
}

#[test]
fn penetration_depth_is_close_to_the_tate_model() {
    let Setup {
        mut solver,
        target_face,
    } = rod_against_target();
    let before = Totals::of(&solver.particles());
    solver
        .run_until(200.0e-6)
        .expect("the simulation stays valid");
    let particles = solver.particles();
    let after = Totals::of(&particles);

    let (_, deepest) =
        bounding_box_where(&particles, |particle| particle.body == ROD).expect("rod exists");
    let depth = deepest.x - target_face;
    let rod_particles: Vec<_> = particles
        .iter()
        .filter(|particle| particle.body == ROD)
        .collect();
    let rod_kinetic: f64 = rod_particles
        .iter()
        .map(|particle| 0.5 * particle.mass * particle.velocity.length_squared())
        .sum();
    let failed = |body: usize| {
        let (failed, all) = particles
            .iter()
            .filter(|particle| particle.body == body)
            .fold((0.0, 0.0), |(failed, all), particle| {
                (
                    failed + f64::from(u8::from(particle.is_failed())),
                    all + 1.0,
                )
            });
        failed / all
    };
    let crater_radius = particles
        .iter()
        .filter(|particle| particle.body == ROD)
        .filter(|particle| {
            let along = particle.position.x - target_face;
            along > 0.25 * depth && along < 0.75 * depth
        })
        .map(|particle| particle.position.y)
        .fold(0.0, f64::max)
        - solver_axis();

    // Strengths representative of the flow stress of the two alloys at
    // large strain and high strain rate; the target resistance follows from
    // cavity expansion, R_t ≈ (2/3)·Y·(1 + ln(2E/(3Y))).
    let tate = TateRod {
        rod_density: library::tungsten_heavy_alloy().eos.reference_density(),
        rod_strength: 1.5e9,
        rod_length: ROD_LENGTH,
        target_density: library::steel_4340().eos.reference_density(),
        target_resistance: 4.5e9,
        impact_speed: IMPACT_SPEED,
    }
    .penetrate();
    eprintln!(
        "long rod: P/L = {:.3} (Tate {:.3}), crater radius = {:.2} rod radii, rod kinetic \
         energy left = {:.2e}, failed rod = {:.2}, failed target = {:.3}, energy drift = {:.2e}",
        depth / ROD_LENGTH,
        tate.depth / ROD_LENGTH,
        crater_radius / ROD_RADIUS,
        rod_kinetic / before.kinetic_energy,
        failed(ROD),
        failed(TARGET),
        after.total_energy() / before.total_energy() - 1.0
    );

    assert!(
        rod_kinetic < 0.01 * before.kinetic_energy,
        "the rod must have stopped"
    );
    assert!((depth / tate.depth - 1.0).abs() < 0.25);
    assert!(crater_radius > ROD_RADIUS);
    assert!((after.total_energy() / before.total_energy() - 1.0).abs() < 0.02);
}

/// Radial coordinate of the axis in the grid of [`rod_against_target`].
fn solver_axis() -> f64 {
    2.0 * CELL
}
