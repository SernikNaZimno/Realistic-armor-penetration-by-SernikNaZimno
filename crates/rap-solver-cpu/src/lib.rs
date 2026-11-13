//! Reference double-precision MPM solver running on the CPU.
//!
//! The scheme is explicit MLS-MPM with quadratic B-splines and APIC
//! transfers. One step consists of
//!
//! 1. particle-to-grid transfer of mass, momentum and stress forces,
//! 2. grid velocity update, contact between bodies and wall conditions,
//! 3. grid-to-particle transfer of velocity and velocity gradient,
//! 4. the constitutive update of every particle.
//!
//! Every body has its own grid velocity field. Where the fields of several
//! bodies overlap, a contact condition stops them from interpenetrating and
//! applies Coulomb friction; bodies are free to separate.
//!
//! Energy is conserved by construction: the internal energy is advanced
//! with the work the stress forces did on the grid, and the kinetic energy
//! dissipated by the particle–grid transfers, by contact and by walls is
//! returned to the particles as heat.

mod constitutive;
mod constraints;
mod kernel;

use std::borrow::Cow;

use glam::{DMat2, DVec2};
use rap_materials::{EquationOfState, Material};
use rap_solver::model::count_to_f64;
use rap_solver::{
    Deviator, Grid, Model, Particle, Solver, SolverConfig, SolverError, Symmetry, Walls,
};
use rayon::prelude::*;

use crate::constitutive::Kinematics;
use crate::kernel::Stencil;

/// Smallest radius, in cells, used when dividing by the distance from the
/// axis of an axisymmetric model.
const MIN_RADIUS_CELLS: f64 = 0.1;

/// Grid quantities of one body.
#[derive(Debug, Clone)]
struct BodyField {
    mass: Vec<f64>,
    /// Node momentum during the particle-to-grid transfer, node velocity at
    /// the end of the step afterwards.
    velocity: Vec<DVec2>,
    /// Impulse of the stress forces during the particle-to-grid transfer,
    /// time-centred node velocity afterwards.
    work_velocity: Vec<DVec2>,
    /// Vector along the outward surface normal of the body.
    outward: Vec<DVec2>,
}

impl BodyField {
    fn new(node_count: usize) -> Self {
        Self {
            mass: vec![0.0; node_count],
            velocity: vec![DVec2::ZERO; node_count],
            work_velocity: vec![DVec2::ZERO; node_count],
            outward: vec![DVec2::ZERO; node_count],
        }
    }

    fn clear(&mut self) {
        self.mass.fill(0.0);
        self.velocity.fill(DVec2::ZERO);
        self.work_velocity.fill(DVec2::ZERO);
        self.outward.fill(DVec2::ZERO);
    }
}

/// Explicit MLS-MPM solver.
#[derive(Debug, Clone)]
pub struct MpmSolver {
    grid: Grid,
    symmetry: Symmetry,
    walls: Walls,
    config: SolverConfig,
    materials: Vec<Material>,
    particles: Vec<Particle>,
    time: f64,
    fields: Vec<BodyField>,
    /// Kinetic energy dissipated at each node by contact and walls during
    /// the current step, per unit of node mass.
    node_heat: Vec<f64>,
}

impl MpmSolver {
    /// Creates a solver for `model` with every particle in its material's
    /// reference state.
    ///
    /// # Errors
    /// Returns [`SolverError::Model`] if the model is inconsistent.
    pub fn new(model: Model) -> Result<Self, SolverError> {
        model.validate()?;
        let body_count = model.body_count();
        let Model {
            grid,
            symmetry,
            walls,
            config,
            materials,
            seeds,
            initial_temperature,
        } = model;
        let particles = seeds
            .iter()
            .map(|seed| {
                let material = &materials[seed.material];
                let density = material.eos.reference_density();
                Particle {
                    position: seed.position,
                    velocity: seed.velocity,
                    velocity_gradient: DMat2::ZERO,
                    mass: density * seed.volume,
                    volume: seed.volume,
                    body: seed.body,
                    material: seed.material,
                    deviator: Deviator::default(),
                    pressure: 0.0,
                    viscous_pressure: 0.0,
                    specific_energy: 0.0,
                    plastic_strain: 0.0,
                    plastic_strain_rate: 0.0,
                    temperature: initial_temperature,
                    damage: 0.0,
                    sound_speed: constitutive::wave_speed(material, density, 0.0, false),
                }
            })
            .collect();
        Ok(Self {
            grid,
            symmetry,
            walls,
            config,
            materials,
            particles,
            time: 0.0,
            fields: vec![BodyField::new(grid.node_count()); body_count],
            node_heat: vec![0.0; grid.node_count()],
        })
    }

    /// Largest time step allowed by the Courant condition, in seconds.
    #[must_use]
    pub fn stable_time_step(&self) -> f64 {
        let fastest = self
            .particles
            .iter()
            .map(|particle| particle.sound_speed + particle.velocity.length())
            .fold(0.0, f64::max);
        self.config.cfl * self.grid.cell_size / fastest
    }

    /// `y` coordinate of the axis of an axisymmetric model.
    fn axis(&self) -> Option<f64> {
        match self.symmetry {
            Symmetry::Planar => None,
            Symmetry::Axisymmetric => Some(self.grid.interior().0.y),
        }
    }

    /// Verifies that every particle is finite and far enough inside the
    /// grid for its interpolation stencil.
    fn check_particles(&self) -> Result<(), SolverError> {
        let [nodes_x, nodes_y] = self.grid.nodes();
        let upper = DVec2::new(count_to_f64(nodes_x), count_to_f64(nodes_y)) - 2.0;
        for (index, particle) in self.particles.iter().enumerate() {
            if !particle.position.is_finite() {
                return Err(SolverError::NonFiniteState {
                    particle: index,
                    time: self.time,
                });
            }
            let base = particle.position / self.grid.cell_size - 0.5;
            if base.min_element() < 0.0 || base.cmpge(upper).any() {
                return Err(SolverError::ParticleLeftGrid {
                    particle: index,
                    time: self.time,
                });
            }
        }
        Ok(())
    }

    fn particles_to_grid(&mut self, time_step: f64) {
        for field in &mut self.fields {
            field.clear();
        }
        let cell_size = self.grid.cell_size;
        let row_length = self.grid.nodes()[0];
        let axis = self.axis();
        let gradient_scale = 4.0 / (cell_size * cell_size);
        let minimum_radius = MIN_RADIUS_CELLS * cell_size;
        for particle in &self.particles {
            let field = &mut self.fields[particle.body];
            // MLS-MPM nodal force: f_i = −V·(4/Δx²)·w·σ·(x_i − x_p).
            let force_scale = -time_step * particle.volume * gradient_scale;
            let normal = particle.pressure + particle.viscous_pressure;
            let deviator = particle.deviator;
            // Axisymmetric hoop stress pulls every node towards the axis:
            // f_i = −V·w·σ_θθ/r along the radius.
            let hoop_impulse = axis.map_or(0.0, |axis| {
                let radius = (particle.position.y - axis).max(minimum_radius);
                -time_step * particle.volume * (deviator.zz() - normal) / radius
            });
            Stencil::new(particle.position, 1.0 / cell_size).for_each(
                row_length,
                cell_size,
                |node, weight, arm| {
                    let traction = DVec2::new(
                        (deviator.xx - normal) * arm.x + deviator.xy * arm.y,
                        deviator.xy * arm.x + (deviator.yy - normal) * arm.y,
                    );
                    let momentum =
                        particle.mass * (particle.velocity + particle.velocity_gradient * arm);
                    let impulse = force_scale * traction + DVec2::new(0.0, hoop_impulse);
                    field.mass[node] += weight * particle.mass;
                    field.velocity[node] += weight * momentum;
                    field.work_velocity[node] += weight * impulse;
                    // Mass-weighted arms point from the body towards the
                    // node, that is out of the body at its surface.
                    field.outward[node] += weight * particle.mass * arm;
                },
            );
        }
    }

    fn update_grid(&mut self) {
        let [nodes_x, nodes_y] = self.grid.nodes();
        let (grid, walls, friction) = (self.grid, self.walls, self.config.friction);
        for j in 0..nodes_y {
            for i in 0..nodes_x {
                let node = i + j * nodes_x;
                // Free motion of every body, and the mean motion of them all.
                let mut total_mass = 0.0;
                let mut total_momentum = DVec2::ZERO;
                let mut total_outward = DVec2::ZERO;
                for field in &mut self.fields {
                    let mass = field.mass[node];
                    if mass <= 0.0 {
                        continue;
                    }
                    let before = field.velocity[node] / mass;
                    let unconstrained = before + field.work_velocity[node] / mass;
                    field.velocity[node] = before;
                    field.work_velocity[node] = unconstrained;
                    total_mass += mass;
                    total_momentum += mass * unconstrained;
                    total_outward += field.outward[node];
                }
                if total_mass <= 0.0 {
                    self.node_heat[node] = 0.0;
                    continue;
                }
                let centre = total_momentum / total_mass;

                let mut dissipated = 0.0;
                for field in &mut self.fields {
                    let mass = field.mass[node];
                    if mass <= 0.0 {
                        continue;
                    }
                    let before = field.velocity[node];
                    let unconstrained = field.work_velocity[node];
                    let mut velocity = unconstrained;
                    if mass < total_mass {
                        // The normal separates this body from all others.
                        let outward = 2.0 * field.outward[node] - total_outward;
                        velocity =
                            constraints::resolve_contact(velocity, centre, outward, friction);
                    }
                    velocity = constraints::apply_walls(velocity, i, j, &grid, walls);
                    // Kinetic energy removed by the constraint impulses,
                    // consistent with the time-centred velocity used for the
                    // stress work.
                    dissipated += 0.5 * mass * (unconstrained - velocity).dot(before + velocity);
                    field.velocity[node] = velocity;
                    field.work_velocity[node] = before.midpoint(velocity);
                }
                self.node_heat[node] = dissipated / total_mass;
            }
        }
    }

    fn grid_to_particles(&mut self, time_step: f64) {
        let cell_size = self.grid.cell_size;
        let row_length = self.grid.nodes()[0];
        let axis = self.axis();
        let gradient_scale = 4.0 / (cell_size * cell_size);
        let minimum_radius = MIN_RADIUS_CELLS * cell_size;
        let (fields, node_heat) = (&self.fields, &self.node_heat);
        let (materials, config) = (&self.materials, &self.config);
        self.particles.par_iter_mut().for_each(|particle| {
            let field = &fields[particle.body];
            let (old_velocity, old_gradient) = (particle.velocity, particle.velocity_gradient);
            let mut velocity = DVec2::ZERO;
            let mut work_velocity = DVec2::ZERO;
            let mut gradient = DMat2::ZERO;
            let mut work_gradient = DMat2::ZERO;
            let mut mixing_loss = 0.0;
            let mut grid_energy = 0.0;
            let mut constraint_heat = 0.0;
            Stencil::new(particle.position, 1.0 / cell_size).for_each(
                row_length,
                cell_size,
                |node, weight, arm| {
                    let node_velocity = field.velocity[node];
                    let node_work_velocity = field.work_velocity[node];
                    let weighted = weight * node_velocity;
                    velocity += weighted;
                    gradient += DMat2::from_cols(weighted * arm.x, weighted * arm.y);
                    let work = weight * node_work_velocity;
                    work_velocity += work;
                    work_gradient += DMat2::from_cols(work * arm.x, work * arm.y);
                    // Node velocity before forces and constraints acted,
                    // recovered from the time-centred and final velocities.
                    let before = 2.0 * node_work_velocity - node_velocity;
                    let sent = old_velocity + old_gradient * arm;
                    mixing_loss += 0.5 * weight * (sent - before).length_squared();
                    grid_energy += 0.5 * weight * node_velocity.length_squared();
                    constraint_heat += weight * node_heat[node];
                },
            );
            let gradient = gradient * gradient_scale;

            // Kinetic energy the transfers cannot represent is dissipated;
            // it is returned as heat so that total energy is conserved.
            // `mixing_loss` is what averaging particle velocities on the
            // grid removed, `projection_loss` what the affine fit of the
            // grid velocity leaves out. Both are non-negative.
            let gradient_norm = gradient.x_axis.length_squared() + gradient.y_axis.length_squared();
            // ½·(|v|² + Δx²/4·‖∇v‖²), the kinetic energy of the affine field.
            let affine_energy =
                f64::midpoint(velocity.length_squared(), gradient_norm / gradient_scale);
            let projection_loss = (grid_energy - affine_energy).max(0.0);
            particle.specific_energy += mixing_loss + projection_loss + constraint_heat;

            let (hoop_rate, work_hoop_rate) = axis.map_or((0.0, 0.0), |axis| {
                let radius = (particle.position.y - axis).max(minimum_radius);
                (velocity.y / radius, work_velocity.y / radius)
            });
            particle.velocity = velocity;
            particle.velocity_gradient = gradient;
            particle.position += time_step * velocity;
            let kinematics = Kinematics {
                gradient,
                work_gradient: work_gradient * gradient_scale,
                hoop_rate,
                work_hoop_rate,
            };
            constitutive::update(
                particle,
                &materials[particle.material],
                &kinematics,
                time_step,
                cell_size,
                config,
            );
        });
    }
}

impl Solver for MpmSolver {
    fn time(&self) -> f64 {
        self.time
    }

    fn step(&mut self, max_time_step: f64) -> Result<f64, SolverError> {
        self.check_particles()?;
        let time_step = self.stable_time_step().min(max_time_step);
        self.particles_to_grid(time_step);
        self.update_grid();
        self.grid_to_particles(time_step);
        self.time += time_step;
        Ok(time_step)
    }

    fn particles(&self) -> Cow<'_, [Particle]> {
        Cow::Borrowed(&self.particles)
    }
}
