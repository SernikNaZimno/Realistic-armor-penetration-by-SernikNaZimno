//! The three-dimensional GPU solver, and ceramics on either GPU solver,
//! against the CPU reference solvers on identical models.
//!
//! The solvers differ only in arithmetic precision and summation order, so
//! integral quantities must agree closely. The tests are skipped, with a
//! note, on machines without a suitable graphics adapter.

use glam::{DVec2, DVec3};
use rap_geometry::{fill_box, fill_rectangle};
use rap_materials::{Material, Strength, library};
use rap_solver::{
    Grid, Grid3, Model, Model3, Particle, Particle3, ParticleSeed, ParticleSeed3, Solver, Solver3,
    SolverConfig, SolverError, Symmetry, Wall, Walls, Walls3,
};
use rap_solver_cpu::{MpmSolver, MpmSolver3};
use rap_solver_gpu::{GpuSolver, GpuSolver3};

/// The solver, or `None` with a note if this machine has no GPU.
fn on_gpu<T>(solver: Result<T, SolverError>) -> Option<T> {
    match solver {
        Ok(solver) => Some(solver),
        Err(SolverError::NoGpu) => {
            eprintln!("skipped: no compatible GPU on this machine");
            None
        }
        Err(error) => panic!("the GPU solver could not be created: {error}"),
    }
}

/// Both three-dimensional solvers advanced to `end_time`.
fn run_both(model: Model3, end_time: f64) -> Option<(MpmSolver3, GpuSolver3)> {
    let mut gpu = on_gpu(GpuSolver3::new(&model))?;
    let mut cpu = MpmSolver3::new(model).expect("the model is valid");
    let gpu_steps = gpu
        .run_until(end_time)
        .expect("the GPU simulation stays valid");
    let cpu_steps = cpu
        .run_until(end_time)
        .expect("the CPU simulation stays valid");
    eprintln!(
        "{}: {gpu_steps} GPU steps, {cpu_steps} CPU steps",
        gpu.adapter_name()
    );
    assert!((gpu.time() - end_time).abs() < 1e-12 * end_time.max(1.0));
    Some((cpu, gpu))
}

fn relative(actual: f64, expected: f64) -> f64 {
    ((actual - expected) / expected).abs()
}

fn mean(
    particles: &[Particle3],
    keep: impl Fn(&Particle3) -> bool,
    value: impl Fn(&Particle3) -> f64,
) -> f64 {
    let (sum, count) = particles
        .iter()
        .filter(|particle| keep(particle))
        .fold((0.0, 0.0), |(sum, count), particle| {
            (sum + value(particle), count + 1.0)
        });
    sum / count
}

/// Mass, momentum and kinetic plus internal energy of `particles`.
fn totals(particles: &[Particle3]) -> (f64, DVec3, f64) {
    particles.iter().fold(
        (0.0, DVec3::ZERO, 0.0),
        |(mass, momentum, energy), particle| {
            (
                mass + particle.mass,
                momentum + particle.mass * particle.velocity,
                energy
                    + particle.mass
                        * (0.5 * particle.velocity.length_squared() + particle.specific_energy),
            )
        },
    )
}

fn block(
    lower: DVec3,
    upper: DVec3,
    spacing: f64,
    velocity: DVec3,
    index: usize,
    keep: impl Fn(DVec3) -> bool,
) -> impl Iterator<Item = ParticleSeed3> {
    fill_box(lower, upper, spacing, keep)
        .into_iter()
        .map(move |position| ParticleSeed3 {
            position,
            velocity,
            volume: spacing.powi(3),
            material: index,
            body: index,
        })
}

fn soft_steel() -> Material {
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

#[test]
fn plate_impact_shock_matches() {
    let cell = 0.5e-3;
    let spacing = cell / 2.0;
    let grid = Grid3 {
        cells: [52, 8, 8],
        cell_size: cell,
    };
    let (lower, upper) = grid.interior();
    let interface = 13.0e-3;
    let plate = |from: f64, to: f64, speed: f64| {
        block(
            DVec3::new(from, lower.y, lower.z),
            DVec3::new(to, upper.y, upper.z),
            spacing,
            DVec3::new(speed, 0.0, 0.0),
            0,
            |_| true,
        )
    };
    let model = Model3 {
        grid,
        walls: Walls3::default(),
        config: SolverConfig::default(),
        materials: vec![soft_steel()],
        seeds: plate(3.0e-3, interface, 500.0)
            .chain(plate(interface, 23.0e-3, -500.0))
            .collect(),
        initial_temperature: 294.0,
    };
    let Some((cpu, gpu)) = run_both(model, 1.2e-6) else {
        return;
    };

    let behind_front =
        |particle: &Particle3| (1.5e-3..=4.0e-3).contains(&(particle.position.x - interface).abs());
    let pressure = |solver: &dyn Solver3| mean(&solver.particles(), behind_front, |p| p.pressure);
    let density =
        |solver: &dyn Solver3| mean(&solver.particles(), behind_front, Particle3::density);
    let (cpu_mass, _, cpu_energy) = totals(&cpu.particles());
    let (gpu_mass, gpu_momentum, gpu_energy) = totals(&gpu.particles());
    eprintln!(
        "3D shock: pressure {:.5e} (CPU {:.5e}), density {:.2} (CPU {:.2}), energy {gpu_energy:.6e} \
         (CPU {cpu_energy:.6e}), momentum {:.2e}",
        pressure(&gpu),
        pressure(&cpu),
        density(&gpu),
        density(&cpu),
        gpu_momentum.x
    );
    assert!(relative(pressure(&gpu), pressure(&cpu)) < 2e-3);
    assert!(relative(density(&gpu), density(&cpu)) < 1e-4);
    assert!(relative(gpu_energy, cpu_energy) < 1e-3);
    assert!(relative(gpu_mass, cpu_mass) < 1e-6);
    // Opposite momenta cancel; what is left is single-precision rounding.
    assert!(gpu_momentum.length() < 1e-4 * gpu_mass * 500.0);
}

#[test]
fn oblique_rod_against_a_plate_matches() {
    // Half of a tungsten rod striking a steel plate tilted by 45°: contact,
    // damage, a sliding symmetry plane and open boundaries all at once.
    oblique_rod_matches(library::tungsten_heavy_alloy(), 0.01);
}

#[test]
fn brittle_rod_breaking_against_a_plate_matches() {
    // The same with a rod of shell steel, which cracks where a principal
    // stress passes its tensile strength. Where a crack starts depends on
    // the last digits of the stress, so the solvers agree less closely.
    oblique_rod_matches(library::shell_steel(), 0.03);
}

/// Runs half of a rod of `rod_material` against a tilted steel plate on
/// both solvers and compares them; the rod velocities may differ by
/// `tolerance` of the impact speed.
fn oblique_rod_matches(rod_material: Material, tolerance: f64) {
    let cell = 1.0e-3;
    let spacing = cell / 2.0;
    let grid = Grid3 {
        cells: [56, 44, 20],
        cell_size: cell,
    };
    let (lower, _) = grid.interior();
    let mirror = lower.z;
    let shot_line = lower.y + 20.0e-3;
    let (radius, length) = (2.0e-3, 12.0e-3);
    let tail = lower.x + 4.0e-3;
    let face = DVec3::new(tail + length + 5.0e-3, shot_line, mirror);
    let slant = std::f64::consts::FRAC_1_SQRT_2;
    let normal = DVec3::new(slant, slant, 0.0);
    let along = DVec3::new(-slant, slant, 0.0);
    let (thickness, half_width) = (4.0e-3, 12.0e-3);

    let rod = block(
        DVec3::new(tail, shot_line - radius, mirror),
        DVec3::new(tail + length, shot_line + radius, mirror + radius),
        spacing,
        DVec3::new(1500.0, 0.0, 0.0),
        0,
        |point| (point.y - shot_line).hypot(point.z - mirror) <= radius,
    );
    let plate = block(
        DVec3::new(face.x - 10.0e-3, shot_line - 10.0e-3, mirror),
        DVec3::new(face.x + 16.0e-3, shot_line + 14.0e-3, mirror + half_width),
        spacing,
        DVec3::ZERO,
        1,
        |point| {
            let offset = point - face;
            (0.0..thickness).contains(&offset.dot(normal)) && offset.dot(along).abs() <= half_width
        },
    );
    let model = Model3 {
        grid,
        walls: Walls3 {
            low: [Wall::Outflow, Wall::Outflow, Wall::Sliding],
            high: [Wall::Outflow; 3],
        },
        config: SolverConfig::default(),
        materials: vec![rod_material, library::steel_4340()],
        seeds: rod.chain(plate).collect(),
        initial_temperature: 294.0,
    };
    let Some((cpu, gpu)) = run_both(model, 16.0e-6) else {
        return;
    };

    // The rod, wherever its particles are now.
    let rod_state = |solver: &dyn Solver3| {
        let (particles, departed) = (solver.particles(), solver.departed());
        let rod: Vec<Particle3> = particles
            .iter()
            .chain(departed.iter())
            .filter(|particle| particle.body == 0)
            .copied()
            .collect();
        let (mass, momentum, _) = totals(&rod);
        let tip = rod
            .iter()
            .map(|particle| particle.position.x)
            .fold(f64::MIN, f64::max);
        (momentum / mass, tip)
    };
    let failed = |solver: &dyn Solver3| {
        mean(
            &solver.particles(),
            |particle| particle.body == 1,
            |particle| f64::from(u8::from(particle.is_failed())),
        )
    };
    let ((cpu_velocity, cpu_tip), (gpu_velocity, gpu_tip)) = (rod_state(&cpu), rod_state(&gpu));
    eprintln!(
        "3D oblique: rod velocity {gpu_velocity:.1} (CPU {cpu_velocity:.1}), tip {:.3} mm (CPU \
         {:.3} mm), failed plate {:.4} (CPU {:.4}), particles {} (CPU {})",
        gpu_tip * 1.0e3,
        cpu_tip * 1.0e3,
        failed(&gpu),
        failed(&cpu),
        gpu.particles().len(),
        cpu.particles().len()
    );
    // The rod has been slowed and pushed off its line by the tilted plate.
    assert!(cpu_velocity.x < 1450.0 && cpu_velocity.y.abs() > 5.0);
    assert!((gpu_velocity - cpu_velocity).length() < tolerance * 1500.0);
    assert!((gpu_tip - cpu_tip).abs() < 0.5 * cell);
    assert!((failed(&gpu) - failed(&cpu)).abs() < 0.01);
    // Nothing crossed the symmetry plane.
    assert!(
        gpu.particles()
            .iter()
            .all(|particle| particle.position.z > mirror)
    );
}

#[test]
fn single_steps_respect_the_time_limit() {
    let cell = 0.5e-3;
    let grid = Grid3 {
        cells: [16, 12, 10],
        cell_size: cell,
    };
    let (lower, upper) = grid.interior();
    let model = Model3 {
        grid,
        walls: Walls3::default(),
        config: SolverConfig::default(),
        materials: vec![library::steel_4340()],
        seeds: block(
            lower + 1.0e-3,
            upper - 1.0e-3,
            cell / 2.0,
            DVec3::new(10.0, 0.0, 0.0),
            0,
            |_| true,
        )
        .collect(),
        initial_temperature: 294.0,
    };
    let Some(mut gpu) = on_gpu(GpuSolver3::new(&model)) else {
        return;
    };
    let cpu = MpmSolver3::new(model).expect("the model is valid");
    assert_eq!(gpu.particles().len(), cpu.particles().len());

    // A free step takes the Courant step, the same as the CPU solver's.
    let free = gpu.step(1.0).expect("a valid step");
    assert!(relative(free, cpu.stable_time_step()) < 1e-5);
    // A limited step takes exactly the limit.
    let limited = gpu.step(free / 4.0).expect("a valid step");
    assert!(relative(limited, free / 4.0) < 1e-5);
    assert!(relative(gpu.time(), 1.25 * free) < 1e-5);
    // Running to a time already reached does nothing.
    assert_eq!(gpu.run_until(gpu.time() / 2.0).expect("nothing to do"), 0);
}

#[test]
fn ceramic_tile_struck_by_a_rod_matches_in_two_dimensions() {
    // Axisymmetric: a tungsten rod against an alumina tile on a steel
    // backing. The ceramic yields, fractures and keeps a reduced strength
    // while confined, which only the JH-2 path of the shader can show.
    let cell = 1.0e-3;
    let spacing = cell / 2.0;
    let grid = Grid {
        cells: [70, 34],
        cell_size: cell,
    };
    let (lower, _) = grid.interior();
    let axis = lower.y;
    let tail = lower.x + 2.0 * cell;
    let (rod_length, rod_radius) = (20.0e-3, 3.0e-3);
    let face = tail + rod_length + cell;
    let tile = 8.0e-3;
    let part = |from: f64, length: f64, radius: f64, speed: f64, material: usize, body: usize| {
        fill_rectangle(
            DVec2::new(from, axis),
            DVec2::new(from + length, axis + radius),
            spacing,
        )
        .into_iter()
        .map(move |position| ParticleSeed {
            position,
            velocity: DVec2::new(speed, 0.0),
            volume: spacing * spacing * (position.y - axis),
            material,
            body,
        })
    };
    let model = Model {
        grid,
        symmetry: Symmetry::Axisymmetric,
        walls: Walls {
            left: Wall::Outflow,
            right: Wall::Outflow,
            bottom: Wall::Sliding,
            top: Wall::Outflow,
        },
        config: SolverConfig::default(),
        materials: vec![
            library::tungsten_heavy_alloy(),
            library::alumina(),
            library::steel_4340(),
        ],
        // Tile and backing are one body: they are bonded.
        seeds: part(tail, rod_length, rod_radius, 1500.0, 0, 0)
            .chain(part(face, tile, 24.0e-3, 0.0, 1, 1))
            .chain(part(face + tile, 12.0e-3, 24.0e-3, 0.0, 2, 1))
            .collect(),
        initial_temperature: 294.0,
    };
    let Some(mut gpu) = on_gpu(GpuSolver::new(&model)) else {
        return;
    };
    let mut cpu = MpmSolver::new(model).expect("the model is valid");
    let end_time = 4.0e-6;
    gpu.run_until(end_time)
        .expect("the GPU simulation stays valid");
    cpu.run_until(end_time)
        .expect("the CPU simulation stays valid");

    let state = |solver: &dyn Solver| {
        let particles = solver.particles();
        let of = |keep: &dyn Fn(&Particle) -> bool, value: &dyn Fn(&Particle) -> f64| {
            let (sum, weight) = particles.iter().filter(|particle| keep(particle)).fold(
                (0.0, 0.0),
                |(sum, weight), particle| {
                    (
                        sum + particle.mass * value(particle),
                        weight + particle.mass,
                    )
                },
            );
            sum / weight
        };
        (
            of(&|particle| particle.body == 0, &|particle| {
                particle.velocity.x
            }),
            of(&|particle| particle.material == 1, &|particle| {
                particle.damage
            }),
            of(&|particle| particle.material == 1, &|particle| {
                f64::from(u8::from(particle.is_failed()))
            }),
            of(&|particle| particle.material == 1, &|particle| {
                particle.deviator.equivalent_stress()
            }),
        )
    };
    let (cpu_state, gpu_state) = (state(&cpu), state(&gpu));
    eprintln!("ceramic: GPU {gpu_state:.4?}\n         CPU {cpu_state:.4?}");
    // The tile is fractured under the rod and still carries shear there.
    assert!(cpu_state.2 > 0.05 && cpu_state.3 > 1.0e7);
    assert!(relative(gpu_state.0, cpu_state.0) < 0.01);
    assert!((gpu_state.1 - cpu_state.1).abs() < 0.01);
    assert!((gpu_state.2 - cpu_state.2).abs() < 0.01);
    assert!(relative(gpu_state.3, cpu_state.3) < 0.03);
}
