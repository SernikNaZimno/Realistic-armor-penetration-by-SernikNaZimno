//! Contact between separate bodies: an elastic bar strikes an identical bar
//! at rest. One-dimensional wave theory predicts that the striker stops and
//! the struck bar leaves with the striker's speed, after which they separate.

use glam::DVec2;
use rap_analysis::{Totals, bounding_box_where};
use rap_geometry::fill_rectangle;
use rap_materials::library;
use rap_solver::{Grid, Model, Particle, ParticleSeed, Solver, SolverConfig, Symmetry, Walls};
use rap_solver_cpu::MpmSolver;

const CELL: f64 = 0.5e-3;
const SPACING: f64 = CELL / 2.0;
const BAR_LENGTH: f64 = 20.0e-3;
const BAR_WIDTH: f64 = 6.0e-3;
const GAP: f64 = CELL;
const STRIKER_SPEED: f64 = 20.0;

fn bars(struck_body: usize) -> MpmSolver {
    let grid = Grid {
        cells: [140, 40],
        cell_size: CELL,
    };
    let (lower, upper) = grid.interior();
    let centre = f64::midpoint(lower.y, upper.y);
    let bar = |from: f64, speed: f64, body: usize| {
        fill_rectangle(
            DVec2::new(from, centre - BAR_WIDTH / 2.0),
            DVec2::new(from + BAR_LENGTH, centre + BAR_WIDTH / 2.0),
            SPACING,
        )
        .into_iter()
        .map(move |position| ParticleSeed {
            position,
            velocity: DVec2::new(speed, 0.0),
            volume: SPACING * SPACING,
            material: 0,
            body,
        })
    };
    let striker_start = 10.0e-3;
    let seeds = bar(striker_start, STRIKER_SPEED, 0)
        .chain(bar(striker_start + BAR_LENGTH + GAP, 0.0, struck_body))
        .collect();
    let model = Model {
        grid,
        symmetry: Symmetry::Planar,
        walls: Walls::default(),
        config: SolverConfig::default(),
        materials: vec![library::steel_4340()],
        seeds,
        initial_temperature: 294.0,
    };
    MpmSolver::new(model).expect("the bar model is valid")
}

fn mean_speed(particles: &[Particle], keep: impl Fn(&Particle) -> bool) -> f64 {
    let (momentum, mass) = particles.iter().filter(|particle| keep(particle)).fold(
        (0.0, 0.0),
        |(momentum, mass), particle| {
            (
                momentum + particle.mass * particle.velocity.x,
                mass + particle.mass,
            )
        },
    );
    momentum / mass
}

#[test]
fn striker_hands_its_momentum_to_the_struck_bar_and_they_separate() {
    let mut solver = bars(1);
    let before = Totals::of(&solver.particles());
    solver
        .run_until(80.0e-6)
        .expect("the simulation stays valid");
    let particles = solver.particles();
    let after = Totals::of(&particles);

    let striker = mean_speed(&particles, |particle| particle.body == 0);
    let struck = mean_speed(&particles, |particle| particle.body == 1);
    let (_, striker_upper) =
        bounding_box_where(&particles, |particle| particle.body == 0).expect("striker exists");
    let (struck_lower, _) =
        bounding_box_where(&particles, |particle| particle.body == 1).expect("struck bar exists");
    let gap = struck_lower.x - striker_upper.x - SPACING;
    eprintln!(
        "contact: striker {striker:.3} m/s, struck {struck:.3} m/s, gap {:.3} mm, momentum \
         drift {:.2e}, energy drift {:.2e}",
        gap * 1.0e3,
        (after.momentum.x / before.momentum.x) - 1.0,
        after.total_energy() / before.total_energy() - 1.0
    );

    assert!(striker.abs() < 0.15 * STRIKER_SPEED);
    assert!((struck - STRIKER_SPEED).abs() < 0.15 * STRIKER_SPEED);
    assert!(gap > GAP, "the bars must have separated again");
    assert!((after.momentum.x / before.momentum.x - 1.0).abs() < 1e-9);
    assert!((after.total_energy() / before.total_energy() - 1.0).abs() < 1e-3);
    let plastic_strain = particles
        .iter()
        .map(|particle| particle.plastic_strain)
        .fold(0.0, f64::max);
    assert!(
        plastic_strain < 0.02,
        "only the contact corners may yield slightly, got {plastic_strain:.3e}"
    );
}

#[test]
fn bars_of_one_body_stick_together() {
    let mut solver = bars(0);
    solver
        .run_until(80.0e-6)
        .expect("the simulation stays valid");
    let particles = solver.particles();
    let split = 10.0e-3 + BAR_LENGTH + GAP / 2.0 + STRIKER_SPEED / 2.0 * 80.0e-6;
    let left = mean_speed(&particles, |particle| particle.position.x < split);
    let right = mean_speed(&particles, |particle| particle.position.x >= split);
    eprintln!("merged: left {left:.3} m/s, right {right:.3} m/s");
    // Welded on contact, the bars move on together at half the striker speed
    // while waves keep travelling back and forth inside them.
    assert!((left - right).abs() < 0.5 * STRIKER_SPEED);
    assert!((f64::midpoint(left, right) - STRIKER_SPEED / 2.0).abs() < 0.1 * STRIKER_SPEED);
}
