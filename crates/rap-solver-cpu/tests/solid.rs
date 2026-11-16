//! The three-dimensional solver against an analytical solution and against
//! the axisymmetric solver on a problem both can represent.

use glam::{DVec2, DVec3};
use rap_analysis::{Totals, bounding_box};
use rap_geometry::{fill_box, fill_rectangle};
use rap_materials::{EquationOfState, Material, Strength, library};
use rap_solver::{
    Grid, Grid3, Model, Model3, Particle3, ParticleSeed, ParticleSeed3, Solver, Solver3,
    SolverConfig, Symmetry, Wall, Walls, Walls3,
};
use rap_solver_cpu::{MpmSolver, MpmSolver3};

fn modified_steel(change: impl Fn(&mut rap_materials::JohnsonCookStrength)) -> Material {
    let mut steel = library::steel_4340();
    let Strength::JohnsonCook(mut strength) = steel.strength else {
        panic!("built-in steel uses Johnson–Cook strength");
    };
    change(&mut strength);
    steel.strength = Strength::JohnsonCook(strength);
    steel
}

fn seeds(points: Vec<DVec3>, velocity: DVec3, spacing: f64) -> impl Iterator<Item = ParticleSeed3> {
    points.into_iter().map(move |position| ParticleSeed3 {
        position,
        velocity,
        volume: spacing.powi(3),
        material: 0,
        body: 0,
    })
}

/// Mass, kinetic and internal energy of a set of particles.
fn totals(particles: &[Particle3]) -> (f64, f64, f64) {
    particles
        .iter()
        .fold((0.0, 0.0, 0.0), |(mass, kinetic, internal), particle| {
            (
                mass + particle.mass,
                kinetic + 0.5 * particle.mass * particle.velocity.length_squared(),
                internal + particle.mass * particle.specific_energy,
            )
        })
}

#[test]
fn strong_shock_follows_the_hugoniot() {
    let steel = modified_steel(|strength| {
        strength.yield_stress = 1.0e6;
        strength.hardening_modulus = 0.0;
        strength.rate_coefficient = 0.0;
    });
    let density = steel.eos.reference_density();
    let cell = 0.5e-3;
    let spacing = cell / 2.0;
    let grid = Grid3 {
        cells: [52, 8, 8],
        cell_size: cell,
    };
    let (lower, upper) = grid.interior();
    let interface = 13.0e-3;
    let particle_velocity = 500.0;
    let plate = |from: f64, to: f64, speed: f64| {
        seeds(
            fill_box(
                DVec3::new(from, lower.y, lower.z),
                DVec3::new(to, upper.y, upper.z),
                spacing,
                |_| true,
            ),
            DVec3::new(speed, 0.0, 0.0),
            spacing,
        )
    };
    let model = Model3 {
        grid,
        walls: Walls3::default(),
        config: SolverConfig::default(),
        materials: vec![steel],
        seeds: plate(3.0e-3, interface, particle_velocity)
            .chain(plate(interface, 23.0e-3, -particle_velocity))
            .collect(),
        initial_temperature: 294.0,
    };
    let mut solver = MpmSolver3::new(model).expect("the model is valid");
    let (mass_before, kinetic_before, internal_before) = totals(&solver.particles());
    solver
        .run_until(1.2e-6)
        .expect("the simulation stays valid");
    let particles = solver.particles();
    let (mass, kinetic, internal) = totals(&particles);

    let behind: Vec<_> = particles
        .iter()
        .filter(|particle| (1.5e-3..=4.0e-3).contains(&(particle.position.x - interface).abs()))
        .collect();
    let count = f64::from(u32::try_from(behind.len()).expect("a few thousand particles"));
    let pressure = behind.iter().map(|particle| particle.pressure).sum::<f64>() / count;
    let shocked_density = behind
        .iter()
        .map(|particle| particle.density())
        .sum::<f64>()
        / count;
    let shock_velocity = 4569.0 + 1.49 * particle_velocity;
    let expected_pressure = density * shock_velocity * particle_velocity;
    let expected_density = density * shock_velocity / (shock_velocity - particle_velocity);
    let drift = (kinetic + internal) / (kinetic_before + internal_before) - 1.0;
    eprintln!(
        "3D shock: p = {pressure:.4e} (expected {expected_pressure:.4e}), ρ = \
         {shocked_density:.1} (expected {expected_density:.1}), energy drift = {drift:.2e}"
    );
    assert!((pressure / expected_pressure - 1.0).abs() < 0.03);
    assert!((shocked_density / expected_density - 1.0).abs() < 0.01);
    assert!((mass / mass_before - 1.0).abs() < 1e-12);
    assert!(drift.abs() < 5e-3);
}

/// Final length over initial length of a perfectly plastic steel cylinder
/// striking a rigid wall, from the axisymmetric solver.
fn axisymmetric_taylor(cell: f64, length: f64, radius: f64, speed: f64, material: Material) -> f64 {
    let spacing = cell / 2.0;
    let grid = Grid {
        cells: [44, 20],
        cell_size: cell,
    };
    let (lower, _) = grid.interior();
    let nose = lower.x + cell;
    let model = Model {
        grid,
        symmetry: Symmetry::Axisymmetric,
        walls: Walls {
            bottom: Wall::Sliding,
            ..Walls::default()
        },
        config: SolverConfig::default(),
        materials: vec![material],
        seeds: fill_rectangle(
            DVec2::new(nose, lower.y),
            DVec2::new(nose + length, lower.y + radius),
            spacing,
        )
        .into_iter()
        .map(|position| ParticleSeed {
            position,
            velocity: DVec2::new(-speed, 0.0),
            volume: spacing * spacing * (position.y - lower.y),
            material: 0,
            body: 0,
        })
        .collect(),
        initial_temperature: 294.0,
    };
    let mut solver = MpmSolver::new(model).expect("the model is valid");
    solver
        .run_until(100.0e-6)
        .expect("the simulation stays valid");
    let particles = solver.particles();
    let (low, high) = bounding_box(&particles).expect("particles exist");
    let _ = Totals::of(&particles);
    (high.x - low.x + spacing) / length
}

#[test]
fn quarter_cylinder_agrees_with_the_axisymmetric_solver() {
    let steel = modified_steel(|strength| {
        strength.hardening_modulus = 0.0;
        strength.rate_coefficient = 0.0;
        strength.reference.melting_temperature = 1.0e9;
    });
    let (cell, length, radius, speed) = (1.0e-3, 32.0e-3, 4.0e-3, 200.0);
    let spacing = cell / 2.0;
    let reference = axisymmetric_taylor(cell, length, radius, speed, steel.clone());

    // A quarter of the cylinder between two symmetry planes.
    let grid = Grid3 {
        cells: [44, 20, 20],
        cell_size: cell,
    };
    let (lower, _) = grid.interior();
    let nose = lower.x + cell;
    let axis = DVec2::new(lower.y, lower.z);
    let points = fill_box(
        DVec3::new(nose, lower.y, lower.z),
        DVec3::new(nose + length, lower.y + radius, lower.z + radius),
        spacing,
        |point| (DVec2::new(point.y, point.z) - axis).length() <= radius,
    );
    let model = Model3 {
        grid,
        walls: Walls3 {
            low: [Wall::Frictionless, Wall::Sliding, Wall::Sliding],
            high: [Wall::Frictionless; 3],
        },
        config: SolverConfig::default(),
        materials: vec![steel],
        seeds: seeds(points, DVec3::new(-speed, 0.0, 0.0), spacing).collect(),
        initial_temperature: 294.0,
    };
    let mut solver = MpmSolver3::new(model).expect("the model is valid");
    let (_, kinetic_before, internal_before) = totals(&solver.particles());
    let initial_volume: f64 = solver
        .particles()
        .iter()
        .map(|particle| particle.volume)
        .sum();
    solver
        .run_until(100.0e-6)
        .expect("the simulation stays valid");
    let particles = solver.particles();
    let (_, kinetic, internal) = totals(&particles);

    let (low, high) = particles
        .iter()
        .fold((f64::MAX, f64::MIN), |(low, high), particle| {
            (low.min(particle.position.x), high.max(particle.position.x))
        });
    let length_ratio = (high - low + spacing) / length;
    let foot_radius = particles
        .iter()
        .filter(|particle| particle.position.x < low + 2.0 * cell)
        .map(|particle| (DVec2::new(particle.position.y, particle.position.z) - axis).length())
        .fold(0.0, f64::max);
    let volume: f64 = particles.iter().map(|particle| particle.volume).sum();
    let drift = (kinetic + internal) / (kinetic_before + internal_before) - 1.0;
    eprintln!(
        "3D taylor: L_f/L_0 = {length_ratio:.4} (axisymmetric {reference:.4}), foot radius = \
         {:.2} x initial, volume = {:.5} x initial, kinetic left = {:.2e}, energy drift = \
         {drift:.2e}",
        foot_radius / radius,
        volume / initial_volume,
        kinetic / kinetic_before
    );
    assert!((length_ratio - reference).abs() < 0.02);
    assert!(foot_radius > 1.2 * radius);
    assert!((volume / initial_volume - 1.0).abs() < 0.01);
    assert!(kinetic < 0.02 * kinetic_before);
    assert!(drift.abs() < 1e-3);
}
