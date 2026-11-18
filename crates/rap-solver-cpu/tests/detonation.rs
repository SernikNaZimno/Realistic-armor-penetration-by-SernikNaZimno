//! Detonation of a charge at a set time: a steel tube filled with TNT.
//!
//! The tube is a slice of an infinitely long cylinder, so its final speed
//! can be compared with Gurney's formula for a cylinder,
//! `v = √(2E) / √(M/C + ½)`, where `M/C` is the mass of the metal over
//! that of the charge and `√(2E)` the Gurney velocity of the explosive.

use glam::DVec2;
use rap_geometry::fill_rectangle;
use rap_materials::library;
use rap_solver::{
    Detonation, Grid, Model, Particle, ParticleSeed, Solver, SolverConfig, Symmetry, Wall, Walls,
};
use rap_solver_cpu::MpmSolver;

/// Gurney velocity commonly quoted for TNT, in m/s.
const GURNEY_VELOCITY: f64 = 2440.0;
const CHARGE_RADIUS: f64 = 10.0e-3;
const TUBE_RADIUS: f64 = 12.0e-3;
/// Edge of a grid cell, in m.
const CELL: f64 = 0.5e-3;
/// Height of the axis: above the two cells of the wall band.
const AXIS: f64 = 2.0 * CELL;
const STEEL: usize = 0;
const CHARGE: usize = 1;
const PRODUCTS: usize = 2;

/// A tube of steel around a charge that goes off at `time`.
fn filled_tube(time: f64) -> Model {
    let spacing = CELL / 2.0;
    let grid = Grid {
        cells: [12, 100],
        cell_size: CELL,
    };
    let (lower, upper) = grid.interior();
    let axis = AXIS;
    assert!((lower.y - axis).abs() < 1e-12);
    let ring = |from: f64, to: f64, material: usize| {
        fill_rectangle(
            DVec2::new(lower.x, axis + from),
            DVec2::new(upper.x, axis + to),
            spacing,
        )
        .into_iter()
        .map(move |position| ParticleSeed {
            position,
            velocity: DVec2::ZERO,
            volume: spacing * spacing * (position.y - axis),
            material,
            body: 0,
        })
    };
    let tnt = library::tnt();
    let energy = tnt.explosive.expect("TNT is an explosive").energy;
    let products = tnt.products().expect("TNT is an explosive");
    Model {
        grid,
        symmetry: Symmetry::Axisymmetric,
        walls: Walls {
            left: Wall::Sliding,
            right: Wall::Sliding,
            bottom: Wall::Sliding,
            top: Wall::Outflow,
        },
        config: SolverConfig {
            detonation: Some(Detonation {
                time,
                material: CHARGE,
                products: PRODUCTS,
                energy,
            }),
            ..SolverConfig::default()
        },
        materials: vec![library::steel_4340(), tnt, products],
        seeds: ring(0.0, CHARGE_RADIUS, CHARGE)
            .chain(ring(CHARGE_RADIUS, TUBE_RADIUS, STEEL))
            .collect(),
        initial_temperature: 294.0,
    }
}

fn energy(particles: &[Particle]) -> f64 {
    particles
        .iter()
        .map(|particle| {
            particle.mass * (0.5 * particle.velocity.length_squared() + particle.specific_energy)
        })
        .sum()
}

fn mass_of(particles: &[Particle], material: usize) -> f64 {
    particles
        .iter()
        .filter(|particle| particle.material == material)
        .map(|particle| particle.mass)
        .sum()
}

#[test]
fn a_charge_stays_solid_until_its_time_and_then_bursts_its_tube() {
    let time = 2.0e-6;
    let mut solver = MpmSolver::new(filled_tube(time)).expect("the model is valid");
    let axis = AXIS;
    let start = solver.particles().into_owned();
    let charge = mass_of(&start, CHARGE);
    let metal = mass_of(&start, STEEL);

    // Before its time the charge is a solid at rest.
    solver.run_until(0.9 * time).expect("the run stays valid");
    {
        let particles = solver.particles();
        assert!(particles.iter().all(|particle| {
            particle.material != PRODUCTS && particle.velocity.length() < 1.0e-6
        }));
    }

    // One step after it, all of it is products at several gigapascals,
    // holding the energy of the detonation.
    solver.run_until(time).expect("the run stays valid");
    solver.step(1.0e-9).expect("the run stays valid");
    {
        let particles = solver.particles();
        assert!((mass_of(&particles, PRODUCTS) / charge - 1.0).abs() < 1e-12);
        assert!(mass_of(&particles, CHARGE).abs() < f64::EPSILON);
        let released = library::tnt().explosive.expect("an explosive").energy * charge;
        assert!((energy(&particles) / released - 1.0).abs() < 1e-3);
        for particle in particles.iter().filter(|p| p.material == PRODUCTS) {
            assert!(particle.is_failed());
            assert!(
                particle.pressure > 8.0e9 && particle.pressure < 9.0e9,
                "{}",
                particle.pressure
            );
        }
    }

    // The tube is thrown outward. By the time it has doubled its radius it
    // has most of the speed Gurney's formula gives.
    let released = energy(&solver.particles());
    solver
        .run_until(time + 9.0e-6)
        .expect("the run stays valid");
    let particles = solver.particles();
    let (momentum, radius) = particles
        .iter()
        .filter(|particle| particle.material == STEEL)
        .fold((0.0, 0.0), |(momentum, radius), particle| {
            (
                momentum + particle.mass * particle.velocity.y,
                radius + particle.mass * (particle.position.y - axis),
            )
        });
    let (speed, radius) = (momentum / metal, radius / metal);
    let gurney = GURNEY_VELOCITY / (metal / charge + 0.5).sqrt();
    eprintln!(
        "tube: M/C {:.3}, speed {speed:.0} m/s at {:.1} mm, Gurney {gurney:.0} m/s, energy kept {:.4}",
        metal / charge,
        radius * 1.0e3,
        energy(&particles) / released
    );
    assert!(radius > 1.6 * TUBE_RADIUS, "{radius}");
    assert!(
        speed > 0.8 * gurney && speed < 1.1 * gurney,
        "{speed} vs {gurney}"
    );
    assert!((energy(&particles) / released - 1.0).abs() < 0.03);
}
