//! The three-dimensional reference solver.
//!
//! The scheme is the one documented at the crate root, with a 3 x 3 x 3
//! interpolation stencil and a full deviatoric stress tensor.

use std::borrow::Cow;

use glam::{DMat3, DVec3};
use rap_materials::{DamageModel, EquationOfState, Material, PlasticState, StrengthModel};
use rap_solver::model::count_to_f64;
use rap_solver::solid::equivalent_stress;
use rap_solver::{
    Grid3, Model3, Particle3, Solver3, SolverConfig, SolverError, WALL_CELLS, Wall, Walls3, contact,
};
use rayon::prelude::*;

use crate::constitutive::wave_speed;

/// Stress triaxiality is limited to this range before it enters the damage
/// model; see the two-dimensional solver.
const TRIAXIALITY_RANGE: (f64, f64) = (-10.0, 1.5);
/// Offsets of the three stencil nodes along one axis, in cells.
const OFFSETS: [f64; 3] = [0.0, 1.0, 2.0];
/// Unit vectors of the coordinate axes.
const AXES: [DVec3; 3] = [DVec3::X, DVec3::Y, DVec3::Z];

/// Quadratic B-spline weights of the three nodes for a particle at
/// `fraction` cells from the first of them.
fn weights(fraction: f64) -> [f64; 3] {
    [
        0.5 * (1.5 - fraction).powi(2),
        0.75 - (fraction - 1.0).powi(2),
        0.5 * (fraction - 0.5).powi(2),
    ]
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the solver verifies that every particle is inside the grid, so the value is a small non-negative integer"
)]
fn to_index(value: f64) -> usize {
    value as usize
}

/// The 3 x 3 x 3 block of grid nodes a particle interacts with.
struct Stencil {
    base: [usize; 3],
    weights: [[f64; 3]; 3],
    /// Particle position relative to the base node, in cells.
    fraction: DVec3,
}

impl Stencil {
    fn new(position: DVec3, inverse_cell_size: f64) -> Self {
        let scaled = position * inverse_cell_size;
        let base = (scaled - 0.5).floor();
        let fraction = scaled - base;
        Self {
            base: base.to_array().map(to_index),
            weights: fraction.to_array().map(weights),
            fraction,
        }
    }

    /// Calls `visit(node, weight, arm)` for every stencil node; `arm` is
    /// the vector from the particle to the node, in m.
    fn for_each(
        &self,
        nodes: [usize; 3],
        cell_size: f64,
        mut visit: impl FnMut(usize, f64, DVec3),
    ) {
        for (k, (weight_z, offset_z)) in self.weights[2].into_iter().zip(OFFSETS).enumerate() {
            let layer = (self.base[2] + k) * nodes[1];
            for (j, (weight_y, offset_y)) in self.weights[1].into_iter().zip(OFFSETS).enumerate() {
                let row = (layer + self.base[1] + j) * nodes[0];
                let weight_yz = weight_y * weight_z;
                for (i, (weight_x, offset_x)) in
                    self.weights[0].into_iter().zip(OFFSETS).enumerate()
                {
                    let arm =
                        (DVec3::new(offset_x, offset_y, offset_z) - self.fraction) * cell_size;
                    visit(row + self.base[0] + i, weight_x * weight_yz, arm);
                }
            }
        }
    }
}

fn apply_wall(velocity: DVec3, wall: Wall, inward: DVec3) -> DVec3 {
    let normal_speed = velocity.dot(inward);
    match wall {
        Wall::Sticky => DVec3::ZERO,
        Wall::Outflow => velocity,
        Wall::Sliding => velocity - normal_speed * inward,
        Wall::Frictionless => velocity - normal_speed.min(0.0) * inward,
    }
}

/// Applies every wall whose band contains the node with indices `index`.
fn apply_walls(mut velocity: DVec3, index: [usize; 3], nodes: [usize; 3], walls: Walls3) -> DVec3 {
    for axis in 0..3 {
        if index[axis] <= WALL_CELLS {
            velocity = apply_wall(velocity, walls.low[axis], AXES[axis]);
        }
        if index[axis] + WALL_CELLS >= nodes[axis] - 1 {
            velocity = apply_wall(velocity, walls.high[axis], -AXES[axis]);
        }
    }
    velocity
}

/// Whether a body touches the others at a node; see
/// `constraints::touching` for the two-dimensional version.
fn touching(outward: DVec3, reach: DVec3, share: f64, cell_size: f64) -> bool {
    let Some(normal) = outward.try_normalize() else {
        return false;
    };
    contact::touches(reach.dot(normal) / cell_size, share.min(1.0 - share))
}

/// Contact of one body with the others at a node; see
/// `constraints::resolve_contact` for the two-dimensional version.
fn resolve_contact(velocity: DVec3, centre: DVec3, outward: DVec3, friction: f64) -> DVec3 {
    let Some(normal) = outward.try_normalize() else {
        return velocity;
    };
    let relative = velocity - centre;
    let approach = relative.dot(normal);
    if approach <= 0.0 {
        return velocity;
    }
    let tangential = relative - approach * normal;
    let slip = tangential.length();
    let friction_change = if slip > 0.0 {
        tangential * ((friction * approach).min(slip) / slip)
    } else {
        DVec3::ZERO
    };
    velocity - approach * normal - friction_change
}

/// Sum of the products of corresponding components of two tensors.
fn inner(a: &DMat3, b: &DMat3) -> f64 {
    a.x_axis.dot(b.x_axis) + a.y_axis.dot(b.y_axis) + a.z_axis.dot(b.z_axis)
}

fn trace(matrix: &DMat3) -> f64 {
    matrix.x_axis.x + matrix.y_axis.y + matrix.z_axis.z
}

/// Advances the material state of `particle` over `time_step`; the
/// three-dimensional form of `constitutive::update`.
fn update_material(
    particle: &mut Particle3,
    material: &Material,
    gradient: &DMat3,
    work_gradient: &DMat3,
    time_step: f64,
    cell_size: f64,
    config: &SolverConfig,
) {
    let transposed = gradient.transpose();
    let rate = (*gradient + transposed) * 0.5;
    let spin = (*gradient - transposed) * 0.5;
    let divergence = trace(gradient);

    // Work of the stress that produced the grid forces, per unit volume.
    let old = particle.deviator;
    let normal = particle.pressure + particle.viscous_pressure;
    let stress_power = inner(&old, work_gradient) - normal * trace(work_gradient);
    particle.specific_energy += time_step * stress_power / particle.density();

    particle.volume *= (time_step * divergence).exp();
    let density = particle.density();

    // Failed metal flows freely; fractured ceramic keeps a reduced,
    // pressure-dependent strength that its strength model accounts for.
    let flows_freely_when_failed = !material.strength.keeps_strength_when_failed();
    if particle.is_failed() && flows_freely_when_failed {
        particle.plastic_strain_rate = 0.0;
    } else {
        // Trial elastic deviator: ṡ = 2G·dev(D) + W·s − s·W.
        let shear_modulus = material.strength.shear_modulus();
        let deviatoric_rate = rate - DMat3::IDENTITY * (divergence / 3.0);
        let trial =
            old + (deviatoric_rate * (2.0 * shear_modulus) + spin * old - old * spin) * time_step;

        let state = PlasticState {
            strain: particle.plastic_strain,
            strain_rate: particle.plastic_strain_rate,
            temperature: particle.temperature,
            pressure: particle.pressure,
            damage: particle.damage,
        };
        let yield_stress = material.strength.flow_stress(state);
        let equivalent = equivalent_stress(&trial);
        if equivalent > yield_stress {
            let strain_increment = (equivalent - yield_stress) / (3.0 * shear_modulus);
            let triaxiality = (-particle.pressure / yield_stress.max(1.0))
                .clamp(TRIAXIALITY_RANGE.0, TRIAXIALITY_RANGE.1);
            let damage = material
                .damage
                .damage_increment(strain_increment, triaxiality, state);
            particle.damage = (particle.damage + damage).min(1.0);
            particle.deviator = trial * (yield_stress / equivalent);
            particle.temperature +=
                material.adiabatic_heating(yield_stress * strain_increment / density);
            particle.plastic_strain += strain_increment;
            particle.plastic_strain_rate = strain_increment / time_step;
        } else {
            particle.deviator = trial;
            particle.plastic_strain_rate = 0.0;
        }
    }

    particle.pressure = material.eos.pressure(density, particle.specific_energy);
    if particle.pressure < -material.spall_strength {
        particle.damage = 1.0;
    }
    if particle.is_failed() {
        if flows_freely_when_failed {
            particle.deviator = DMat3::ZERO;
        }
        particle.pressure = particle.pressure.max(0.0);
    }
    particle.sound_speed = wave_speed(
        material,
        density,
        particle.specific_energy,
        // Failed material without confinement is loose: metal always,
        // fractured ceramic once its pressure is gone. It has no shear
        // stiffness to carry a faster wave.
        particle.is_failed() && (flows_freely_when_failed || particle.pressure <= 0.0),
    );

    let compression_rate = (-divergence).max(0.0);
    particle.viscous_pressure = density
        * cell_size
        * compression_rate
        * (config.linear_viscosity * particle.sound_speed
            + config.quadratic_viscosity * cell_size * compression_rate);
}

/// Grid quantities of one body.
#[derive(Debug, Clone)]
struct BodyField {
    mass: Vec<f64>,
    /// Node momentum during the particle-to-grid transfer, node velocity at
    /// the end of the step afterwards.
    velocity: Vec<DVec3>,
    /// Impulse of the stress forces during the particle-to-grid transfer,
    /// time-centred node velocity afterwards.
    work_velocity: Vec<DVec3>,
    /// Vector along the outward surface normal of the body.
    outward: Vec<DVec3>,
}

impl BodyField {
    fn new(node_count: usize) -> Self {
        Self {
            mass: vec![0.0; node_count],
            velocity: vec![DVec3::ZERO; node_count],
            work_velocity: vec![DVec3::ZERO; node_count],
            outward: vec![DVec3::ZERO; node_count],
        }
    }

    fn clear(&mut self) {
        self.mass.fill(0.0);
        self.velocity.fill(DVec3::ZERO);
        self.work_velocity.fill(DVec3::ZERO);
        self.outward.fill(DVec3::ZERO);
    }
}

/// Explicit MLS-MPM solver for three-dimensional models.
#[derive(Debug, Clone)]
pub struct MpmSolver3 {
    grid: Grid3,
    walls: Walls3,
    config: SolverConfig,
    materials: Vec<Material>,
    particles: Vec<Particle3>,
    /// Particles that crossed an outflow wall.
    departed: Vec<Particle3>,
    time: f64,
    fields: Vec<BodyField>,
    /// Kinetic energy dissipated at each node by contact and walls during
    /// the current step, per unit of node mass.
    node_heat: Vec<f64>,
}

impl MpmSolver3 {
    /// Creates a solver for `model` with every particle in its material's
    /// reference state.
    ///
    /// # Errors
    /// Returns [`SolverError::Model`] if the model is inconsistent.
    pub fn new(model: Model3) -> Result<Self, SolverError> {
        model.validate()?;
        let body_count = model.body_count();
        let Model3 {
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
                Particle3 {
                    position: seed.position,
                    velocity: seed.velocity,
                    velocity_gradient: DMat3::ZERO,
                    mass: density * seed.volume,
                    volume: seed.volume,
                    body: seed.body,
                    material: seed.material,
                    deviator: DMat3::ZERO,
                    pressure: 0.0,
                    viscous_pressure: 0.0,
                    specific_energy: 0.0,
                    plastic_strain: 0.0,
                    plastic_strain_rate: 0.0,
                    temperature: initial_temperature,
                    damage: 0.0,
                    sound_speed: wave_speed(material, density, 0.0, false),
                }
            })
            .collect();
        Ok(Self {
            grid,
            walls,
            config,
            materials,
            particles,
            departed: Vec::new(),
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

    /// Moves particles that crossed an outflow wall out of the simulation.
    fn retire_departed(&mut self) {
        let walls = self.walls;
        if !walls
            .low
            .iter()
            .chain(&walls.high)
            .any(|wall| *wall == Wall::Outflow)
        {
            return;
        }
        let (lower, upper) = self.grid.interior();
        let has_left = |particle: &Particle3| {
            let (position, lower, upper) = (
                particle.position.to_array(),
                lower.to_array(),
                upper.to_array(),
            );
            (0..3).any(|axis| {
                (walls.low[axis] == Wall::Outflow && position[axis] < lower[axis])
                    || (walls.high[axis] == Wall::Outflow && position[axis] > upper[axis])
            })
        };
        let (gone, kept) = std::mem::take(&mut self.particles)
            .into_iter()
            .partition(has_left);
        self.particles = kept;
        self.departed.extend::<Vec<Particle3>>(gone);
    }

    /// Verifies that every particle is finite and far enough inside the
    /// grid for its interpolation stencil.
    fn check_particles(&self) -> Result<(), SolverError> {
        let upper = DVec3::from_array(self.grid.nodes().map(count_to_f64)) - 2.0;
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
        let nodes = self.grid.nodes();
        let gradient_scale = 4.0 / (cell_size * cell_size);
        for particle in &self.particles {
            let field = &mut self.fields[particle.body];
            // MLS-MPM nodal force: f_i = −V·(4/Δx²)·w·σ·(x_i − x_p).
            let force_scale = -time_step * particle.volume * gradient_scale;
            let normal = particle.pressure + particle.viscous_pressure;
            Stencil::new(particle.position, 1.0 / cell_size).for_each(
                nodes,
                cell_size,
                |node, weight, arm| {
                    let traction = particle.deviator * arm - normal * arm;
                    let momentum =
                        particle.mass * (particle.velocity + particle.velocity_gradient * arm);
                    field.mass[node] += weight * particle.mass;
                    field.velocity[node] += weight * momentum;
                    field.work_velocity[node] += weight * force_scale * traction;
                    field.outward[node] += weight * particle.mass * arm;
                },
            );
        }
    }

    fn update_grid(&mut self) {
        let nodes = self.grid.nodes();
        let (walls, friction) = (self.walls, self.config.friction);
        for k in 0..nodes[2] {
            for j in 0..nodes[1] {
                for i in 0..nodes[0] {
                    let node = i + (j + k * nodes[1]) * nodes[0];
                    self.update_node(node, [i, j, k], nodes, walls, friction);
                }
            }
        }
    }

    /// Turns the gathered momentum of one node into velocities, applying
    /// contact between bodies and the walls.
    fn update_node(
        &mut self,
        node: usize,
        index: [usize; 3],
        nodes: [usize; 3],
        walls: Walls3,
        friction: f64,
    ) {
        let cell_size = self.grid.cell_size;
        let mut total_mass = 0.0;
        let mut total_momentum = DVec3::ZERO;
        let mut total_outward = DVec3::ZERO;
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
            return;
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
                let own = field.outward[node];
                let outward = 2.0 * own - total_outward;
                // From each centre of mass to the node.
                let reach = own / mass - (total_outward - own) / (total_mass - mass);
                if touching(outward, reach, mass / total_mass, cell_size) {
                    velocity = resolve_contact(velocity, centre, outward, friction);
                }
            }
            velocity = apply_walls(velocity, index, nodes, walls);
            dissipated += 0.5 * mass * (unconstrained - velocity).dot(before + velocity);
            field.velocity[node] = velocity;
            field.work_velocity[node] = before.midpoint(velocity);
        }
        self.node_heat[node] = dissipated / total_mass;
    }

    fn grid_to_particles(&mut self, time_step: f64) {
        let cell_size = self.grid.cell_size;
        let nodes = self.grid.nodes();
        let gradient_scale = 4.0 / (cell_size * cell_size);
        let (fields, node_heat) = (&self.fields, &self.node_heat);
        let (materials, config) = (&self.materials, &self.config);
        self.particles.par_iter_mut().for_each(|particle| {
            let field = &fields[particle.body];
            let (old_velocity, old_gradient) = (particle.velocity, particle.velocity_gradient);
            let mut velocity = DVec3::ZERO;
            let mut gradient = DMat3::ZERO;
            let mut work_gradient = DMat3::ZERO;
            let mut mixing_loss = 0.0;
            let mut grid_energy = 0.0;
            let mut constraint_heat = 0.0;
            Stencil::new(particle.position, 1.0 / cell_size).for_each(
                nodes,
                cell_size,
                |node, weight, arm| {
                    let node_velocity = field.velocity[node];
                    let node_work_velocity = field.work_velocity[node];
                    let weighted = weight * node_velocity;
                    velocity += weighted;
                    gradient +=
                        DMat3::from_cols(weighted * arm.x, weighted * arm.y, weighted * arm.z);
                    let work = weight * node_work_velocity;
                    work_gradient += DMat3::from_cols(work * arm.x, work * arm.y, work * arm.z);
                    let before = 2.0 * node_work_velocity - node_velocity;
                    let sent = old_velocity + old_gradient * arm;
                    mixing_loss += 0.5 * weight * (sent - before).length_squared();
                    grid_energy += 0.5 * weight * node_velocity.length_squared();
                    constraint_heat += weight * node_heat[node];
                },
            );
            let gradient = gradient * gradient_scale;
            let work_gradient = work_gradient * gradient_scale;

            // Kinetic energy the transfers cannot represent returns as heat.
            // ½·(|v|² + Δx²/4·‖∇v‖²) is the kinetic energy of the affine field.
            let affine_energy = f64::midpoint(
                velocity.length_squared(),
                inner(&gradient, &gradient) / gradient_scale,
            );
            let projection_loss = (grid_energy - affine_energy).max(0.0);
            particle.specific_energy += mixing_loss + projection_loss + constraint_heat;

            particle.velocity = velocity;
            particle.velocity_gradient = gradient;
            particle.position += time_step * velocity;
            update_material(
                particle,
                &materials[particle.material],
                &gradient,
                &work_gradient,
                time_step,
                cell_size,
                config,
            );
        });
    }
}

impl Solver3 for MpmSolver3 {
    fn time(&self) -> f64 {
        self.time
    }

    fn step(&mut self, max_time_step: f64) -> Result<f64, SolverError> {
        self.retire_departed();
        self.check_particles()?;
        let time_step = self.stable_time_step().min(max_time_step);
        self.particles_to_grid(time_step);
        self.update_grid();
        self.grid_to_particles(time_step);
        self.time += time_step;
        Ok(time_step)
    }

    fn particles(&self) -> Cow<'_, [Particle3]> {
        Cow::Borrowed(&self.particles)
    }

    fn departed(&self) -> Cow<'_, [Particle3]> {
        Cow::Borrowed(&self.departed)
    }
}

#[cfg(test)]
mod tests {
    use glam::{DMat3, DVec3};
    use proptest::prelude::*;
    use rap_materials::{EquationOfState, StrengthModel, library};
    use rap_solver::{Particle3, SolverConfig, Wall};

    use super::{Stencil, apply_wall, resolve_contact, update_material};
    use crate::constitutive::wave_speed;

    const CELL: f64 = 1.0e-3;
    const NO_VISCOSITY: SolverConfig = SolverConfig {
        cfl: 0.3,
        linear_viscosity: 0.0,
        quadratic_viscosity: 0.0,
        friction: 0.0,
    };

    fn particle() -> Particle3 {
        let steel = library::steel_4340();
        let density = steel.eos.reference_density();
        Particle3 {
            position: DVec3::ZERO,
            velocity: DVec3::ZERO,
            velocity_gradient: DMat3::ZERO,
            mass: density * 1.0e-9,
            volume: 1.0e-9,
            body: 0,
            material: 0,
            deviator: DMat3::ZERO,
            pressure: 0.0,
            viscous_pressure: 0.0,
            specific_energy: 0.0,
            plastic_strain: 0.0,
            plastic_strain_rate: 0.0,
            temperature: 294.0,
            damage: 0.0,
            sound_speed: wave_speed(&steel, density, 0.0, false),
        }
    }

    proptest! {
        #[test]
        fn stencil_reproduces_constant_and_linear_fields(
            x in 1.0e-3..9.0e-3_f64, y in 1.0e-3..9.0e-3_f64, z in 1.0e-3..9.0e-3_f64,
        ) {
            let stencil = Stencil::new(DVec3::new(x, y, z), 1.0 / CELL);
            let mut total = 0.0;
            let mut first_moment = DVec3::ZERO;
            let mut second_moment = DMat3::ZERO;
            let mut count = 0;
            let mut largest_node = 0;
            stencil.for_each([12, 12, 12], CELL, |node, weight, arm| {
                total += weight;
                first_moment += weight * arm;
                second_moment += DMat3::from_cols(arm * arm.x, arm * arm.y, arm * arm.z) * weight;
                count += 1;
                largest_node = largest_node.max(node);
            });
            prop_assert!(largest_node < 12 * 12 * 12);
            prop_assert_eq!(count, 27);
            prop_assert!((total - 1.0).abs() < 1e-12);
            prop_assert!(first_moment.length() < 1e-15);
            let expected = DMat3::IDENTITY * (CELL * CELL / 4.0);
            prop_assert!((second_moment - expected).abs_diff_eq(DMat3::ZERO, 1e-18));
        }
    }

    #[test]
    fn small_uniaxial_compression_is_linear_elastic() {
        let steel = library::steel_4340();
        let mut state = particle();
        let strain = -1.0e-4;
        let steps = 100;
        let mut gradient = DMat3::ZERO;
        gradient.z_axis.z = strain / 1.0e-6 / f64::from(steps);
        for _ in 0..steps {
            update_material(
                &mut state,
                &steel,
                &gradient,
                &gradient,
                1.0e-6,
                CELL,
                &NO_VISCOSITY,
            );
        }
        let shear = steel.strength.shear_modulus();
        let bulk = steel.eos.reference_density() * 4569.0_f64.powi(2);
        let expected = (bulk + 4.0 * shear / 3.0) * strain;
        let stress_zz = state.deviator.z_axis.z - state.pressure;
        assert!((stress_zz / expected - 1.0).abs() < 2e-3, "{stress_zz}");
        // The lateral deviators are half the axial one, with opposite sign.
        assert!((state.deviator.x_axis.x / (-2.0 * shear * strain / 3.0) - 1.0).abs() < 1e-6);
        assert!(state.plastic_strain.abs() < f64::EPSILON);
    }

    #[test]
    fn large_shear_returns_to_the_yield_surface() {
        let steel = library::steel_4340();
        let mut state = particle();
        let mut gradient = DMat3::ZERO;
        // ∂v_y/∂z: simple shear in the y–z plane.
        gradient.z_axis.y = 0.05 / 1.0e-7;
        update_material(
            &mut state,
            &steel,
            &gradient,
            &gradient,
            1.0e-7,
            CELL,
            &NO_VISCOSITY,
        );
        assert!((state.equivalent_stress() / 792.0e6 - 1.0).abs() < 1e-9);
        assert!(state.plastic_strain > 0.02);
        assert!((state.volume - 1.0e-9).abs() < 1e-21);
        // The stress stays symmetric.
        assert!((state.deviator.z_axis.y - state.deviator.y_axis.z).abs() < 1e-3);
    }

    #[test]
    fn rigid_rotation_rotates_the_stress() {
        let steel = library::steel_4340();
        let mut state = particle();
        state.deviator = DMat3::from_diagonal(DVec3::new(2.0e8, -1.0e8, -1.0e8));
        // Rotate by 90° about z: x → y.
        let steps = 20_000;
        let spin = std::f64::consts::FRAC_PI_2 / f64::from(steps) / 1.0e-9;
        let mut gradient = DMat3::ZERO;
        gradient.y_axis.x = -spin;
        gradient.x_axis.y = spin;
        for _ in 0..steps {
            update_material(
                &mut state,
                &steel,
                &gradient,
                &gradient,
                1.0e-9,
                CELL,
                &NO_VISCOSITY,
            );
        }
        assert!(
            (state.deviator.x_axis.x / -1.0e8 - 1.0).abs() < 1e-3,
            "{:?}",
            state.deviator
        );
        assert!(
            (state.deviator.y_axis.y / 2.0e8 - 1.0).abs() < 1e-3,
            "{:?}",
            state.deviator
        );
        assert!((state.deviator.z_axis.z / -1.0e8 - 1.0).abs() < 1e-3);
    }

    #[test]
    fn walls_and_contact_behave_as_in_two_dimensions() {
        let velocity = DVec3::new(-3.0, 2.0, 1.0);
        assert_eq!(
            apply_wall(velocity, Wall::Frictionless, DVec3::X),
            DVec3::new(0.0, 2.0, 1.0)
        );
        assert_eq!(
            apply_wall(velocity, Wall::Sliding, DVec3::Z),
            DVec3::new(-3.0, 2.0, 0.0)
        );
        assert_eq!(apply_wall(velocity, Wall::Sticky, DVec3::Y), DVec3::ZERO);
        assert_eq!(apply_wall(velocity, Wall::Outflow, DVec3::X), velocity);

        // Two equal bodies meeting along z share their normal velocity.
        let (first, second) = (DVec3::new(1.0, 0.0, 4.0), DVec3::new(0.0, 2.0, -2.0));
        let centre = (first + second) / 2.0;
        let after_first = resolve_contact(first, centre, DVec3::Z, 0.0);
        let after_second = resolve_contact(second, centre, DVec3::NEG_Z, 0.0);
        assert!((after_first.z - after_second.z).abs() < 1e-12);
        assert!((after_first + after_second - first - second).length() < 1e-12);
        // Separating bodies are left alone.
        assert_eq!(resolve_contact(second, centre, DVec3::Z, 0.5), second);
    }
}
