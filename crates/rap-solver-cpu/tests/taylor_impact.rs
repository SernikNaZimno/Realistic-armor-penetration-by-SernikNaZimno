//! Taylor impact: a bar strikes a rigid, frictionless wall end-on.
//!
//! The bar is perfectly plastic, so its final length can be compared with
//! the one-dimensional estimate of Wilkins and Guinan (1973),
//!
//! ```text
//! L_f/L_0 = (1 − h)·exp(−ρ·v²/(2·Y)) + h,    h ≈ 0.12.
//! ```
//!
//! Two versions are run: a cylinder in an axisymmetric model, which is the
//! classical test, and a flat bar in plane strain, for which the flow stress
//! is `2Y/√3`.

use glam::DVec2;
use rap_analysis::{Totals, bounding_box};
use rap_geometry::fill_rectangle;
use rap_materials::{EquationOfState, Material, Strength, library};
use rap_solver::{Grid, Model, ParticleSeed, Solver, SolverConfig, Symmetry, Wall, Walls};
use rap_solver_cpu::MpmSolver;

const CELL: f64 = 0.5e-3;
const SPACING: f64 = CELL / 2.0;
const LENGTH: f64 = 25.0e-3;
const WIDTH: f64 = 5.0e-3;
const YIELD_STRESS: f64 = 792.0e6;

/// Steel that neither hardens nor softens.
fn perfectly_plastic_steel() -> Material {
    let mut steel = library::steel_4340();
    let Strength::JohnsonCook(mut strength) = steel.strength else {
        panic!("built-in steel uses Johnson–Cook strength");
    };
    strength.yield_stress = YIELD_STRESS;
    strength.hardening_modulus = 0.0;
    strength.rate_coefficient = 0.0;
    strength.reference.melting_temperature = 1.0e9;
    steel.strength = Strength::JohnsonCook(strength);
    steel
}

fn bar_against_left_wall(speed: f64) -> MpmSolver {
    let grid = Grid {
        cells: [70, 44],
        cell_size: CELL,
    };
    let (lower, upper) = grid.interior();
    let centre = f64::midpoint(lower.y, upper.y);
    let nose = lower.x + CELL;
    let seeds = fill_rectangle(
        DVec2::new(nose, centre - WIDTH / 2.0),
        DVec2::new(nose + LENGTH, centre + WIDTH / 2.0),
        SPACING,
    )
    .into_iter()
    .map(|position| ParticleSeed {
        position,
        velocity: DVec2::new(-speed, 0.0),
        volume: SPACING * SPACING,
        material: 0,
        body: 0,
    })
    .collect();
    let model = Model {
        grid,
        symmetry: Symmetry::Planar,
        walls: Walls::default(),
        config: SolverConfig::default(),
        materials: vec![perfectly_plastic_steel()],
        seeds,
        initial_temperature: 294.0,
    };
    MpmSolver::new(model).expect("the bar model is valid")
}

#[test]
fn bar_shortens_as_predicted_and_mushrooms_at_the_wall() {
    let speed = 200.0;
    let mut solver = bar_against_left_wall(speed);
    let before = Totals::of(&solver.particles());
    solver
        .run_until(80.0e-6)
        .expect("the simulation stays valid");
    let particles = solver.particles();
    let after = Totals::of(&particles);

    let (lower, upper) = bounding_box(&particles).expect("the bar has particles");
    let length_ratio = (upper.x - lower.x + SPACING) / LENGTH;
    let foot_width = {
        let foot: Vec<_> = particles
            .iter()
            .filter(|particle| particle.position.x < lower.x + 2.0 * CELL)
            .map(|particle| particle.position.y)
            .collect();
        let widest = foot.iter().copied().fold(f64::MIN, f64::max);
        let narrowest = foot.iter().copied().fold(f64::MAX, f64::min);
        widest - narrowest + SPACING
    };
    let tail_strain = particles
        .iter()
        .filter(|particle| particle.position.x > upper.x - 2.0 * CELL)
        .map(|particle| particle.plastic_strain)
        .fold(0.0, f64::max);
    let nose_strain = particles
        .iter()
        .filter(|particle| particle.position.x < lower.x + 2.0 * CELL)
        .map(|particle| particle.plastic_strain)
        .fold(0.0, f64::max);

    let density = perfectly_plastic_steel().eos.reference_density();
    let flow_stress = 2.0 * YIELD_STRESS / 3.0_f64.sqrt();
    let expected_ratio = 0.88 * (-density * speed * speed / (2.0 * flow_stress)).exp() + 0.12;
    eprintln!(
        "taylor: L_f/L_0 = {length_ratio:.4} (estimate {expected_ratio:.4}), foot width = \
         {:.2} x initial, nose strain = {nose_strain:.3}, tail strain = {tail_strain:.4}, \
         kinetic energy left = {:.3e}, energy drift = {:.3e}, mean v_x = {:.2}",
        foot_width / WIDTH,
        after.kinetic_energy / before.kinetic_energy,
        after.total_energy() / before.total_energy() - 1.0,
        after.momentum.x / after.mass,
    );

    assert!((length_ratio - expected_ratio).abs() < 0.03);
    assert!(foot_width > 1.3 * WIDTH);
    assert!(nose_strain > 0.3);
    assert!(tail_strain < 0.01);
    assert!(after.kinetic_energy < 0.02 * before.kinetic_energy);
    // The rigid wall removes the momentum of the nodes it constrains without
    // crediting the lost kinetic energy as heat; about 1.6 % is lost here.
    assert!((after.total_energy() / before.total_energy() - 1.0).abs() < 0.03);
    assert!(
        after.momentum.x >= 0.0,
        "the bar must not keep moving into the wall"
    );
}

fn cylinder_against_left_wall(speed: f64, length: f64, radius: f64) -> MpmSolver {
    let grid = Grid {
        cells: [80, 34],
        cell_size: CELL,
    };
    let (lower, _) = grid.interior();
    let axis = lower.y;
    let nose = lower.x + CELL;
    let seeds = fill_rectangle(
        DVec2::new(nose, axis),
        DVec2::new(nose + length, axis + radius),
        SPACING,
    )
    .into_iter()
    .map(|position| ParticleSeed {
        position,
        velocity: DVec2::new(-speed, 0.0),
        // Per radian: cross-section area times radius.
        volume: SPACING * SPACING * (position.y - axis),
        material: 0,
        body: 0,
    })
    .collect();
    let model = Model {
        grid,
        symmetry: Symmetry::Axisymmetric,
        walls: Walls {
            bottom: Wall::Sliding,
            ..Walls::default()
        },
        config: SolverConfig::default(),
        materials: vec![perfectly_plastic_steel()],
        seeds,
        initial_temperature: 294.0,
    };
    MpmSolver::new(model).expect("the cylinder model is valid")
}

#[test]
fn cylinder_shortens_as_predicted_and_conserves_mass_and_energy() {
    let speed = 200.0;
    let (length, radius) = (32.0e-3, 4.0e-3);
    let mut solver = cylinder_against_left_wall(speed, length, radius);
    let before = Totals::of(&solver.particles());
    solver
        .run_until(100.0e-6)
        .expect("the simulation stays valid");
    let particles = solver.particles();
    let after = Totals::of(&particles);

    let (lower, upper) = bounding_box(&particles).expect("the cylinder has particles");
    let length_ratio = (upper.x - lower.x + SPACING) / length;
    let axis = 2.0 * CELL;
    let foot_radius = particles
        .iter()
        .filter(|particle| particle.position.x < lower.x + 2.0 * CELL)
        .map(|particle| particle.position.y - axis + SPACING / 2.0)
        .fold(0.0, f64::max);
    // Volume per radian of the deformed cylinder against the initial one.
    let volume: f64 = particles.iter().map(|particle| particle.volume).sum();
    let initial_volume = length * radius * radius / 2.0;

    let density = perfectly_plastic_steel().eos.reference_density();
    let expected_ratio = 0.88 * (-density * speed * speed / (2.0 * YIELD_STRESS)).exp() + 0.12;
    eprintln!(
        "cylinder: L_f/L_0 = {length_ratio:.4} (estimate {expected_ratio:.4}), foot radius = \
         {:.2} x initial, volume = {:.5} x initial, kinetic energy left = {:.3e}, energy \
         drift = {:.3e}",
        foot_radius / radius,
        volume / initial_volume,
        after.kinetic_energy / before.kinetic_energy,
        after.total_energy() / before.total_energy() - 1.0,
    );

    assert!((length_ratio - expected_ratio).abs() < 0.03);
    assert!(foot_radius > 1.2 * radius);
    assert!((volume / initial_volume - 1.0).abs() < 0.01);
    assert!((after.mass / before.mass - 1.0).abs() < 1e-12);
    assert!(after.kinetic_energy < 0.02 * before.kinetic_energy);
    assert!((after.total_energy() / before.total_energy() - 1.0).abs() < 1e-3);
}
