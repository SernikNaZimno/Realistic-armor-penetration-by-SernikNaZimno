//! Reference double-precision MPM solver running on the CPU.
//!
//! The scheme is explicit MLS-MPM with quadratic B-splines and APIC
//! transfers. One step consists of
//!
//! 1. particle-to-grid transfer of mass, momentum and stress forces,
//! 2. grid velocity update and wall conditions,
//! 3. grid-to-particle transfer of velocity and velocity gradient,
//! 4. the constitutive update of every particle.
//!
//! Energy is conserved by construction where possible: the internal energy
//! is advanced with the work the stress forces did on the grid, and kinetic
//! energy dissipated by the particle–grid transfers is returned as heat.
//! Rigid walls are the exception; they absorb the kinetic energy of the
//! motion they stop.
//!
//! All bodies share one velocity field, so touching bodies stick together;
//! frictional contact between bodies is not modelled yet.

mod constitutive;
mod kernel;

use std::borrow::Cow;

use glam::{DMat2, DVec2};
use rap_materials::{EquationOfState, Material};
use rap_solver::model::count_to_f64;
use rap_solver::{
    Deviator, Grid, Model, Particle, Solver, SolverConfig, SolverError, WALL_CELLS, Wall, Walls,
};
use rayon::prelude::*;

use crate::kernel::Stencil;

/// Explicit MLS-MPM solver.
#[derive(Debug, Clone)]
pub struct MpmSolver {
    grid: Grid,
    walls: Walls,
    config: SolverConfig,
    materials: Vec<Material>,
    particles: Vec<Particle>,
    time: f64,
    node_mass: Vec<f64>,
    /// Node momentum during the particle-to-grid transfer, node velocity
    /// at the end of the step afterwards.
    node_velocity: Vec<DVec2>,
    /// Impulse of the stress forces during the particle-to-grid transfer,
    /// time-centred node velocity afterwards.
    node_work_velocity: Vec<DVec2>,
}

impl MpmSolver {
    /// Creates a solver for `model` with every particle in its material's
    /// reference state.
    ///
    /// # Errors
    /// Returns [`SolverError::Model`] if the model is inconsistent.
    pub fn new(model: Model) -> Result<Self, SolverError> {
        model.validate()?;
        let Model {
            grid,
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
                    material: seed.material,
                    deviator: Deviator::default(),
                    pressure: 0.0,
                    viscous_pressure: 0.0,
                    specific_energy: 0.0,
                    plastic_strain: 0.0,
                    plastic_strain_rate: 0.0,
                    temperature: initial_temperature,
                    sound_speed: constitutive::longitudinal_speed(material, density, 0.0),
                }
            })
            .collect();
        Ok(Self {
            grid,
            walls,
            config,
            materials,
            particles,
            time: 0.0,
            node_mass: vec![0.0; grid.node_count()],
            node_velocity: vec![DVec2::ZERO; grid.node_count()],
            node_work_velocity: vec![DVec2::ZERO; grid.node_count()],
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
        self.node_mass.fill(0.0);
        self.node_velocity.fill(DVec2::ZERO);
        self.node_work_velocity.fill(DVec2::ZERO);
        let cell_size = self.grid.cell_size;
        let row_length = self.grid.nodes()[0];
        for particle in &self.particles {
            // MLS-MPM nodal force: f_i = −V·(4/Δx²)·w·σ·(x_i − x_p).
            let force_scale = -time_step * particle.volume * 4.0 / (cell_size * cell_size);
            let normal = particle.pressure + particle.viscous_pressure;
            let deviator = particle.deviator;
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
                    self.node_mass[node] += weight * particle.mass;
                    self.node_velocity[node] += weight * momentum;
                    self.node_work_velocity[node] += weight * force_scale * traction;
                },
            );
        }
    }

    fn update_grid(&mut self) {
        let [nodes_x, nodes_y] = self.grid.nodes();
        let walls = self.walls;
        for j in 0..nodes_y {
            for i in 0..nodes_x {
                let node = i + j * nodes_x;
                let mass = self.node_mass[node];
                if mass <= 0.0 {
                    continue;
                }
                let before = self.node_velocity[node] / mass;
                let mut velocity = before + self.node_work_velocity[node] / mass;
                if i <= WALL_CELLS {
                    velocity = constrain(velocity, walls.left, DVec2::X);
                }
                if i + WALL_CELLS >= nodes_x - 1 {
                    velocity = constrain(velocity, walls.right, DVec2::NEG_X);
                }
                if j <= WALL_CELLS {
                    velocity = constrain(velocity, walls.bottom, DVec2::Y);
                }
                if j + WALL_CELLS >= nodes_y - 1 {
                    velocity = constrain(velocity, walls.top, DVec2::NEG_Y);
                }
                self.node_velocity[node] = velocity;
                self.node_work_velocity[node] = before.midpoint(velocity);
            }
        }
    }

    fn grid_to_particles(&mut self, time_step: f64) {
        let cell_size = self.grid.cell_size;
        let row_length = self.grid.nodes()[0];
        let gradient_scale = 4.0 / (cell_size * cell_size);
        let (node_velocity, node_work_velocity) = (&self.node_velocity, &self.node_work_velocity);
        let (materials, config) = (&self.materials, &self.config);
        self.particles.par_iter_mut().for_each(|particle| {
            let mut velocity = DVec2::ZERO;
            let mut gradient = DMat2::ZERO;
            let mut work_gradient = DMat2::ZERO;
            let mut mixing_loss = 0.0;
            let mut grid_energy = 0.0;
            let (old_velocity, old_gradient) = (particle.velocity, particle.velocity_gradient);
            Stencil::new(particle.position, 1.0 / cell_size).for_each(
                row_length,
                cell_size,
                |node, weight, arm| {
                    let weighted = weight * node_velocity[node];
                    velocity += weighted;
                    gradient += DMat2::from_cols(weighted * arm.x, weighted * arm.y);
                    let work = weight * node_work_velocity[node];
                    work_gradient += DMat2::from_cols(work * arm.x, work * arm.y);
                    // Node velocity before the forces acted, recovered from
                    // the time-centred and final velocities.
                    let before = 2.0 * node_work_velocity[node] - node_velocity[node];
                    let sent = old_velocity + old_gradient * arm;
                    mixing_loss += 0.5 * weight * (sent - before).length_squared();
                    grid_energy += 0.5 * weight * node_velocity[node].length_squared();
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
            particle.specific_energy += mixing_loss + projection_loss;
            particle.velocity = velocity;
            particle.velocity_gradient = gradient;
            particle.position += time_step * velocity;
            constitutive::update(
                particle,
                &materials[particle.material],
                gradient,
                work_gradient * gradient_scale,
                time_step,
                cell_size,
                config,
            );
        });
    }
}

/// Applies a wall with inward unit normal `inward` to a node velocity.
fn constrain(velocity: DVec2, wall: Wall, inward: DVec2) -> DVec2 {
    match wall {
        Wall::Sticky => DVec2::ZERO,
        Wall::Frictionless => velocity - velocity.dot(inward).min(0.0) * inward,
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

#[cfg(test)]
mod tests {
    use glam::DVec2;
    use rap_solver::Wall;

    use super::constrain;

    #[test]
    fn frictionless_wall_removes_only_inward_motion() {
        let into_wall = constrain(DVec2::new(-3.0, 2.0), Wall::Frictionless, DVec2::X);
        assert_eq!(into_wall, DVec2::new(0.0, 2.0));
        let away = constrain(DVec2::new(3.0, 2.0), Wall::Frictionless, DVec2::X);
        assert_eq!(away, DVec2::new(3.0, 2.0));
    }

    #[test]
    fn sticky_wall_stops_everything() {
        assert_eq!(
            constrain(DVec2::new(3.0, 2.0), Wall::Sticky, DVec2::NEG_Y),
            DVec2::ZERO
        );
    }
}
