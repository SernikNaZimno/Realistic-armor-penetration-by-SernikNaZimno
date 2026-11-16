//! The GPU solver against the CPU reference solver on identical models.
//!
//! The two differ only in arithmetic precision and summation order, so
//! integral quantities must agree closely. The tests are skipped, with a
//! note, on machines without a suitable graphics adapter.

use glam::DVec2;
use rap_analysis::{Totals, bounding_box, bounding_box_where};
use rap_geometry::fill_rectangle;
use rap_materials::{Material, Strength, library};
use rap_solver::{
    Grid, Model, Particle, ParticleSeed, Solver, SolverConfig, SolverError, Symmetry, Wall, Walls,
};
use rap_solver_cpu::MpmSolver;
use rap_solver_gpu::GpuSolver;

const CELL: f64 = 0.5e-3;
const SPACING: f64 = CELL / 2.0;

/// Both solvers advanced to `end_time`, or `None` without a GPU.
fn run_both(model: Model, end_time: f64) -> Option<(MpmSolver, GpuSolver)> {
    let mut gpu = match GpuSolver::new(&model) {
        Ok(solver) => solver,
        Err(SolverError::NoGpu) => {
            eprintln!("skipped: no compatible GPU on this machine");
            return None;
        }
        Err(error) => panic!("the GPU solver could not be created: {error}"),
    };
    let mut cpu = MpmSolver::new(model).expect("the model is valid");
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
    particles: &[Particle],
    keep: impl Fn(&Particle) -> bool,
    value: impl Fn(&Particle) -> f64,
) -> f64 {
    let (sum, count) = particles
        .iter()
        .filter(|particle| keep(particle))
        .fold((0.0, 0.0), |(sum, count), particle| {
            (sum + value(particle), count + 1.0)
        });
    sum / count
}

fn seeds(
    lower: DVec2,
    upper: DVec2,
    velocity: DVec2,
    index: usize,
    volume: impl Fn(DVec2) -> f64,
) -> impl Iterator<Item = ParticleSeed> {
    fill_rectangle(lower, upper, SPACING)
        .into_iter()
        .map(move |position| ParticleSeed {
            position,
            velocity,
            volume: volume(position),
            material: index,
            body: index,
        })
}

fn modified_steel(change: impl Fn(&mut rap_materials::JohnsonCookStrength)) -> Material {
    let mut steel = library::steel_4340();
    let Strength::JohnsonCook(mut strength) = steel.strength else {
        panic!("built-in steel uses Johnson–Cook strength");
    };
    change(&mut strength);
    steel.strength = Strength::JohnsonCook(strength);
    steel
}

#[test]
fn plate_impact_shock_matches() {
    let steel = modified_steel(|strength| {
        strength.yield_stress = 1.0e6;
        strength.hardening_modulus = 0.0;
        strength.rate_coefficient = 0.0;
    });
    let grid = Grid {
        cells: [100, 12],
        cell_size: CELL,
    };
    let (lower, upper) = grid.interior();
    let interface = 25.0e-3;
    let area = |_| SPACING * SPACING;
    let left = seeds(
        DVec2::new(5.0e-3, lower.y),
        DVec2::new(interface, upper.y),
        DVec2::new(500.0, 0.0),
        0,
        area,
    );
    let right = seeds(
        DVec2::new(interface, lower.y),
        DVec2::new(45.0e-3, upper.y),
        DVec2::new(-500.0, 0.0),
        0,
        area,
    );
    let model = Model {
        grid,
        symmetry: Symmetry::Planar,
        walls: Walls::default(),
        config: SolverConfig::default(),
        materials: vec![steel],
        seeds: left.chain(right).collect(),
        initial_temperature: 294.0,
    };
    let Some((cpu, gpu)) = run_both(model, 2.0e-6) else {
        return;
    };

    let behind_front =
        |particle: &Particle| (2.0e-3..=6.0e-3).contains(&(particle.position.x - interface).abs());
    let pressure = |solver: &dyn Solver| mean(&solver.particles(), behind_front, |p| p.pressure);
    let density = |solver: &dyn Solver| mean(&solver.particles(), behind_front, Particle::density);
    let (cpu_pressure, gpu_pressure) = (pressure(&cpu), pressure(&gpu));
    let (cpu_totals, gpu_totals) = (Totals::of(&cpu.particles()), Totals::of(&gpu.particles()));
    eprintln!(
        "shock: pressure {gpu_pressure:.5e} (CPU {cpu_pressure:.5e}), density {:.2} (CPU {:.2}), \
         energy {:.6e} (CPU {:.6e}), momentum {:.2e}",
        density(&gpu),
        density(&cpu),
        gpu_totals.total_energy(),
        cpu_totals.total_energy(),
        gpu_totals.momentum.x
    );
    assert!(relative(gpu_pressure, cpu_pressure) < 2e-3);
    assert!(relative(density(&gpu), density(&cpu)) < 1e-4);
    assert!(relative(gpu_totals.total_energy(), cpu_totals.total_energy()) < 1e-3);
    assert!(relative(gpu_totals.mass, cpu_totals.mass) < 1e-6);
    // Opposite momenta cancel; what is left is single-precision rounding.
    assert!(gpu_totals.momentum.x.abs() < 1e-4 * gpu_totals.mass * 500.0);
}

#[test]
fn axisymmetric_taylor_cylinder_matches() {
    let steel = modified_steel(|strength| {
        strength.hardening_modulus = 0.0;
        strength.rate_coefficient = 0.0;
        strength.reference.melting_temperature = 1.0e9;
    });
    let grid = Grid {
        cells: [80, 34],
        cell_size: CELL,
    };
    let (lower, _) = grid.interior();
    let axis = lower.y;
    let nose = lower.x + CELL;
    let (length, radius) = (32.0e-3, 4.0e-3);
    let model = Model {
        grid,
        symmetry: Symmetry::Axisymmetric,
        walls: Walls {
            bottom: Wall::Sliding,
            ..Walls::default()
        },
        config: SolverConfig::default(),
        materials: vec![steel],
        seeds: seeds(
            DVec2::new(nose, axis),
            DVec2::new(nose + length, axis + radius),
            DVec2::new(-200.0, 0.0),
            0,
            |position| SPACING * SPACING * (position.y - axis),
        )
        .collect(),
        initial_temperature: 294.0,
    };
    let initial_energy = Totals::of(&MpmSolver::new(model.clone()).expect("valid").particles());
    let Some((cpu, gpu)) = run_both(model, 100.0e-6) else {
        return;
    };

    let length_ratio = |solver: &dyn Solver| {
        let (low, high) = bounding_box(&solver.particles()).expect("particles exist");
        (high.x - low.x + SPACING) / length
    };
    let strain = |solver: &dyn Solver| {
        solver
            .particles()
            .iter()
            .map(|particle| particle.plastic_strain)
            .fold(0.0, f64::max)
    };
    let gpu_totals = Totals::of(&gpu.particles());
    eprintln!(
        "taylor: length ratio {:.5} (CPU {:.5}), peak strain {:.3} (CPU {:.3}), energy drift {:.2e}",
        length_ratio(&gpu),
        length_ratio(&cpu),
        strain(&gpu),
        strain(&cpu),
        gpu_totals.total_energy() / initial_energy.total_energy() - 1.0
    );
    assert!((length_ratio(&gpu) - length_ratio(&cpu)).abs() < 5e-3);
    assert!(relative(strain(&gpu), strain(&cpu)) < 0.1);
    assert!(relative(gpu_totals.total_energy(), initial_energy.total_energy()) < 5e-3);
}

#[test]
fn contact_between_bars_matches() {
    let grid = Grid {
        cells: [140, 40],
        cell_size: CELL,
    };
    let (lower, upper) = grid.interior();
    let centre = f64::midpoint(lower.y, upper.y);
    let (bar_length, half_width) = (20.0e-3, 3.0e-3);
    let area = |_| SPACING * SPACING;
    let bar = |from: f64, speed: f64, index: usize| {
        seeds(
            DVec2::new(from, centre - half_width),
            DVec2::new(from + bar_length, centre + half_width),
            DVec2::new(speed, 0.0),
            index,
            area,
        )
    };
    let model = Model {
        grid,
        symmetry: Symmetry::Planar,
        walls: Walls::default(),
        config: SolverConfig::default(),
        materials: vec![library::steel_4340(), library::steel_4340()],
        seeds: bar(10.0e-3, 20.0, 0)
            .chain(bar(10.0e-3 + bar_length + CELL, 0.0, 1))
            .collect(),
        initial_temperature: 294.0,
    };
    let Some((cpu, gpu)) = run_both(model, 80.0e-6) else {
        return;
    };

    let speed = |solver: &dyn Solver, body: usize| {
        let particles = solver.particles();
        let (momentum, mass) = particles
            .iter()
            .filter(|particle| particle.body == body)
            .fold((0.0, 0.0), |(momentum, mass), particle| {
                (
                    momentum + particle.mass * particle.velocity.x,
                    mass + particle.mass,
                )
            });
        momentum / mass
    };
    eprintln!(
        "contact: striker {:.3} (CPU {:.3}), struck {:.3} (CPU {:.3})",
        speed(&gpu, 0),
        speed(&cpu, 0),
        speed(&gpu, 1),
        speed(&cpu, 1)
    );
    assert!((speed(&gpu, 0) - speed(&cpu, 0)).abs() < 0.2);
    assert!((speed(&gpu, 1) - speed(&cpu, 1)).abs() < 0.2);
    // Total momentum is that of the striker before impact.
    assert!((speed(&gpu, 0) + speed(&gpu, 1) - 20.0).abs() < 1e-3);
}

#[test]
fn long_rod_penetration_with_outflow_matches() {
    let cell = 1.0e-3;
    let spacing = cell / 2.0;
    let grid = Grid {
        cells: [130, 40],
        cell_size: cell,
    };
    let (lower, _) = grid.interior();
    let axis = lower.y;
    let rod_tail = lower.x + 2.0 * cell;
    let (rod_length, rod_radius) = (40.0e-3, 4.0e-3);
    let face = rod_tail + rod_length + cell;
    let body = |from: f64, length: f64, radius: f64, speed: f64, index: usize| {
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
            material: index,
            body: index,
        })
    };
    let open = Walls {
        left: Wall::Outflow,
        right: Wall::Outflow,
        bottom: Wall::Sliding,
        top: Wall::Outflow,
    };
    let model = Model {
        grid,
        symmetry: Symmetry::Axisymmetric,
        walls: open,
        config: SolverConfig::default(),
        materials: vec![library::tungsten_heavy_alloy(), library::steel_4340()],
        seeds: body(rod_tail, rod_length, rod_radius, 1500.0, 0)
            .chain(body(face, 70.0e-3, 30.0e-3, 0.0, 1))
            .collect(),
        initial_temperature: 294.0,
    };
    let Some((cpu, gpu)) = run_both(model, 90.0e-6) else {
        return;
    };

    let depth = |solver: &dyn Solver| {
        let (_, deepest) =
            bounding_box_where(&solver.particles(), |particle| particle.body == 0).expect("rod");
        deepest.x - face
    };
    let failed = |solver: &dyn Solver| {
        mean(
            &solver.particles(),
            |particle| particle.body == 1,
            |particle| f64::from(u8::from(particle.is_failed())),
        )
    };
    eprintln!(
        "long rod: depth {:.3} mm (CPU {:.3} mm), failed target {:.4} (CPU {:.4}), departed {} \
         (CPU {})",
        depth(&gpu) * 1.0e3,
        depth(&cpu) * 1.0e3,
        failed(&gpu),
        failed(&cpu),
        gpu.departed().len(),
        cpu.departed().len()
    );
    assert!(relative(depth(&gpu), depth(&cpu)) < 0.03);
    assert!((failed(&gpu) - failed(&cpu)).abs() < 0.01);
    assert!(depth(&gpu) > 0.5 * rod_length);
}

#[test]
fn single_steps_respect_the_time_limit() {
    let grid = Grid {
        cells: [20, 12],
        cell_size: CELL,
    };
    let (lower, upper) = grid.interior();
    let model = Model {
        grid,
        symmetry: Symmetry::Planar,
        walls: Walls::default(),
        config: SolverConfig::default(),
        materials: vec![library::steel_4340()],
        seeds: seeds(
            lower + 1.0e-3,
            upper - 1.0e-3,
            DVec2::new(10.0, 0.0),
            0,
            |_| SPACING * SPACING,
        )
        .collect(),
        initial_temperature: 294.0,
    };
    let mut gpu = match GpuSolver::new(&model) {
        Ok(solver) => solver,
        Err(SolverError::NoGpu) => {
            eprintln!("skipped: no compatible GPU on this machine");
            return;
        }
        Err(error) => panic!("the GPU solver could not be created: {error}"),
    };
    let cpu = MpmSolver::new(model).expect("the model is valid");
    assert_eq!(gpu.particles().len(), cpu.particles().len());

    // A free step takes the Courant step, the same as the CPU solver's.
    let free = gpu.step(1.0).expect("a valid step");
    assert!(
        relative(free, cpu.stable_time_step()) < 1e-5,
        "{free} vs {}",
        cpu.stable_time_step()
    );
    // A limited step takes exactly the limit.
    let limited = gpu.step(free / 4.0).expect("a valid step");
    assert!(relative(limited, free / 4.0) < 1e-5);
    assert!(relative(gpu.time(), 1.25 * free) < 1e-5);
    // Running to a time already reached does nothing.
    assert_eq!(gpu.run_until(gpu.time() / 2.0).expect("nothing to do"), 0);
}
