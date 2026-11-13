//! Symmetric plate impact compared with analytical wave solutions.
//!
//! Two identical plates approach each other at `±speed` and meet at a plane
//! that stays at rest by symmetry. Frictionless walls above and below keep
//! the strain uniaxial, so the problem is one-dimensional and each plate
//! sees a wave that brings its material to rest.

use glam::DVec2;
use rap_analysis::Totals;
use rap_geometry::fill_rectangle;
use rap_materials::{EquationOfState, Material, Strength, StrengthModel, library};
use rap_solver::{Grid, Model, Particle, ParticleSeed, Solver, SolverConfig, Symmetry, Walls};
use rap_solver_cpu::MpmSolver;

const CELL: f64 = 0.5e-3;
const INTERFACE: f64 = 25.0e-3;
const PLATE_THICKNESS: f64 = 20.0e-3;
const BULK_SOUND_SPEED: f64 = 4569.0;
const HUGONIOT_SLOPE: f64 = 1.49;

fn colliding_plates(material: Material, speed: f64) -> MpmSolver {
    let grid = Grid {
        cells: [100, 12],
        cell_size: CELL,
    };
    let (lower, upper) = grid.interior();
    let spacing = CELL / 2.0;
    let plate = |from: f64, to: f64, velocity: f64| {
        fill_rectangle(DVec2::new(from, lower.y), DVec2::new(to, upper.y), spacing)
            .into_iter()
            .map(move |position| ParticleSeed {
                position,
                velocity: DVec2::new(velocity, 0.0),
                volume: spacing * spacing,
                material: 0,
                body: 0,
            })
    };
    let seeds = plate(INTERFACE - PLATE_THICKNESS, INTERFACE, speed)
        .chain(plate(INTERFACE, INTERFACE + PLATE_THICKNESS, -speed))
        .collect();
    let model = Model {
        grid,
        symmetry: Symmetry::Planar,
        walls: Walls::default(),
        config: SolverConfig::default(),
        materials: vec![material],
        seeds,
        initial_temperature: 294.0,
    };
    MpmSolver::new(model).expect("the plate model is valid")
}

/// Steel with its equation of state but almost no strength, so that the
/// response is hydrodynamic.
fn strengthless_steel() -> Material {
    let mut steel = library::steel_4340();
    let Strength::JohnsonCook(mut strength) = steel.strength else {
        panic!("built-in steel uses Johnson–Cook strength");
    };
    strength.yield_stress = 1.0e6;
    strength.hardening_modulus = 0.0;
    strength.rate_coefficient = 0.0;
    steel.strength = Strength::JohnsonCook(strength);
    steel
}

fn distance_from_interface(particle: &Particle) -> f64 {
    (particle.position.x - INTERFACE).abs()
}

/// Mean of `value` over the particles between 2 mm and 6 mm from the
/// interface, a region behind the wave front and away from the interface.
fn mean_behind_front(particles: &[Particle], value: impl Fn(&Particle) -> f64) -> f64 {
    let (sum, count) = particles
        .iter()
        .filter(|particle| (2.0e-3..=6.0e-3).contains(&distance_from_interface(particle)))
        .fold((0.0, 0.0), |(sum, count), particle| {
            (sum + value(particle), count + 1.0)
        });
    sum / count
}

/// Distance from the interface of the farthest particle whose compressive
/// stress along `x` exceeds `threshold`.
fn front_distance(particles: &[Particle], threshold: f64) -> f64 {
    particles
        .iter()
        .filter(|particle| -particle.stress_xx() > threshold)
        .map(distance_from_interface)
        .fold(0.0, f64::max)
}

fn relative_error(actual: f64, expected: f64) -> f64 {
    ((actual - expected) / expected).abs()
}

#[test]
fn strong_shock_follows_the_hugoniot() {
    let steel = strengthless_steel();
    let density = steel.eos.reference_density();
    let particle_velocity = 500.0;
    let shock_velocity = BULK_SOUND_SPEED + HUGONIOT_SLOPE * particle_velocity;
    let expected_pressure = density * shock_velocity * particle_velocity;
    let expected_density = density * shock_velocity / (shock_velocity - particle_velocity);

    let mut solver = colliding_plates(steel, particle_velocity);
    let before = Totals::of(&solver.particles());
    let end_time = 2.0e-6;
    solver
        .run_until(end_time)
        .expect("the simulation stays valid");
    let particles = solver.particles();
    let after = Totals::of(&particles);

    let pressure = mean_behind_front(&particles, |particle| particle.pressure);
    let shocked_density = mean_behind_front(&particles, Particle::density);
    let residual_speed = mean_behind_front(&particles, |particle| particle.velocity.x.abs());
    let front = front_distance(&particles, expected_pressure / 2.0);
    let expected_front = (shock_velocity - particle_velocity) * end_time;
    eprintln!(
        "shock: p = {pressure:.4e} (expected {expected_pressure:.4e}), ρ = {shocked_density:.1} \
         (expected {expected_density:.1}), |v| = {residual_speed:.2}, front = {front:.5} \
         (expected {expected_front:.5}), energy drift = {:.3e}",
        after.total_energy() / before.total_energy() - 1.0
    );

    assert!(relative_error(pressure, expected_pressure) < 0.03);
    assert!(relative_error(shocked_density, expected_density) < 0.01);
    assert!(residual_speed < 0.03 * particle_velocity);
    assert!((front - expected_front).abs() < 2.0 * CELL);

    assert!(relative_error(after.mass, before.mass) < 1e-12);
    assert!(after.momentum.length() < 1e-9 * before.mass * particle_velocity);
    assert!(relative_error(after.total_energy(), before.total_energy()) < 0.005);
}

#[test]
fn weak_impact_sends_an_elastic_wave_at_the_longitudinal_speed() {
    let steel = library::steel_4340();
    let density = steel.eos.reference_density();
    let longitudinal_modulus =
        density * BULK_SOUND_SPEED.powi(2) + 4.0 * steel.strength.shear_modulus() / 3.0;
    let wave_speed = (longitudinal_modulus / density).sqrt();
    let particle_velocity = 5.0;
    let expected_stress = density * wave_speed * particle_velocity;

    let mut solver = colliding_plates(steel, particle_velocity);
    let end_time = 2.0e-6;
    solver
        .run_until(end_time)
        .expect("the simulation stays valid");
    let particles = solver.particles();

    let stress = mean_behind_front(&particles, |particle| -particle.stress_xx());
    let plastic_strain = particles
        .iter()
        .map(|particle| particle.plastic_strain)
        .fold(0.0, f64::max);
    let front = front_distance(&particles, expected_stress / 2.0);
    let expected_front = wave_speed * end_time;
    eprintln!(
        "elastic: σ = {stress:.4e} (expected {expected_stress:.4e}), front = {front:.5} \
         (expected {expected_front:.5}), max plastic strain = {plastic_strain:.3e}"
    );

    assert!(relative_error(stress, expected_stress) < 0.03);
    assert!((front - expected_front).abs() < 2.0 * CELL);
    assert!(plastic_strain.abs() < f64::EPSILON);
}
