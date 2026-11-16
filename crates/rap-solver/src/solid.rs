//! Three-dimensional models: the counterparts of [`crate::Model`],
//! [`crate::Particle`] and [`crate::Solver`] for problems without a
//! symmetry that reduces them to two dimensions.

use std::borrow::Cow;

use glam::{DMat3, DVec3};
use rap_domain::validation;
use rap_materials::Material;

use crate::SolverError;
use crate::model::{MAX_BODIES, ModelError, SolverConfig, WALL_CELLS, Wall, count_to_f64};

/// Uniform background grid with its origin at `(0, 0, 0)`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Grid3 {
    /// Number of cells along `x`, `y` and `z`.
    pub cells: [usize; 3],
    /// Edge length of a cell, in m.
    pub cell_size: f64,
}

impl Grid3 {
    /// Number of nodes along `x`, `y` and `z`.
    #[must_use]
    pub fn nodes(&self) -> [usize; 3] {
        self.cells.map(|cells| cells + 1)
    }

    /// Total number of nodes.
    #[must_use]
    pub fn node_count(&self) -> usize {
        self.nodes().iter().product()
    }

    /// Lower and upper corners of the region enclosed by the walls, in m.
    #[must_use]
    pub fn interior(&self) -> (DVec3, DVec3) {
        let lower = DVec3::splat(count_to_f64(WALL_CELLS) * self.cell_size);
        let upper = DVec3::from_array(self.cells.map(|cells| count_to_f64(cells - WALL_CELLS)))
            * self.cell_size;
        (lower, upper)
    }
}

/// The six rigid walls enclosing the grid interior.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Walls3 {
    /// Walls at the low side of `x`, `y` and `z`.
    pub low: [Wall; 3],
    /// Walls at the high side of `x`, `y` and `z`.
    pub high: [Wall; 3],
}

/// Initial state of one material point.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ParticleSeed3 {
    /// Position, in m.
    pub position: DVec3,
    /// Velocity, in m/s.
    pub velocity: DVec3,
    /// Volume represented by the particle, in m³.
    pub volume: f64,
    /// Index into [`Model3::materials`].
    pub material: usize,
    /// Body the particle belongs to.
    pub body: usize,
}

/// A complete three-dimensional problem.
#[derive(Debug, Clone, PartialEq)]
pub struct Model3 {
    /// Background grid.
    pub grid: Grid3,
    /// Rigid walls around the grid interior.
    pub walls: Walls3,
    /// Numerical parameters.
    pub config: SolverConfig,
    /// Materials referenced by the particles.
    pub materials: Vec<Material>,
    /// Initial particles.
    pub seeds: Vec<ParticleSeed3>,
    /// Uniform initial temperature, in K.
    pub initial_temperature: f64,
}

impl Model3 {
    /// Number of bodies, one more than the largest body index in use.
    #[must_use]
    pub fn body_count(&self) -> usize {
        self.seeds
            .iter()
            .map(|seed| seed.body + 1)
            .max()
            .unwrap_or(0)
    }

    /// Checks the model for internal consistency.
    ///
    /// # Errors
    /// Returns the first [`ModelError`] found.
    pub fn validate(&self) -> Result<(), ModelError> {
        validation::positive("grid.cell_size", self.grid.cell_size)?;
        let [cells_x, cells_y, cells_z] = self.grid.cells;
        if cells_x.min(cells_y).min(cells_z) <= 2 * WALL_CELLS {
            return Err(ModelError::GridTooSmall(cells_x, cells_y.min(cells_z)));
        }
        validation::within(
            "config.cfl",
            self.config.cfl,
            &(f64::MIN_POSITIVE..=1.0),
            "in (0, 1]",
        )?;
        validation::non_negative("config.linear_viscosity", self.config.linear_viscosity)?;
        validation::non_negative(
            "config.quadratic_viscosity",
            self.config.quadratic_viscosity,
        )?;
        validation::non_negative("config.friction", self.config.friction)?;
        validation::positive("initial_temperature", self.initial_temperature)?;
        for material in &self.materials {
            material.validate()?;
        }
        if self.seeds.is_empty() {
            return Err(ModelError::NoParticles);
        }
        let (lower, upper) = self.grid.interior();
        for (particle, seed) in self.seeds.iter().enumerate() {
            validation::positive("seed.volume", seed.volume)?;
            for (field, value) in [
                ("seed.velocity.x", seed.velocity.x),
                ("seed.velocity.y", seed.velocity.y),
                ("seed.velocity.z", seed.velocity.z),
            ] {
                validation::finite(field, value)?;
            }
            if seed.material >= self.materials.len() {
                return Err(ModelError::UnknownMaterial {
                    particle,
                    material: seed.material,
                });
            }
            if seed.body >= MAX_BODIES {
                return Err(ModelError::TooManyBodies {
                    particle,
                    body: seed.body,
                });
            }
            let inside = seed.position.cmpge(lower).all() && seed.position.cmple(upper).all();
            if !inside {
                return Err(ModelError::ParticleOutsideWalls { particle });
            }
        }
        Ok(())
    }
}

/// One material point of a three-dimensional model.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Particle3 {
    /// Position, in m.
    pub position: DVec3,
    /// Velocity, in m/s.
    pub velocity: DVec3,
    /// Velocity gradient `∂v_i/∂x_j`, in 1/s.
    pub velocity_gradient: DMat3,
    /// Mass, in kg.
    pub mass: f64,
    /// Current volume, in m³.
    pub volume: f64,
    /// Body the particle belongs to.
    pub body: usize,
    /// Index of the particle's material in [`Model3::materials`].
    pub material: usize,
    /// Deviatoric Cauchy stress, in Pa; symmetric and traceless.
    pub deviator: DMat3,
    /// Pressure from the equation of state, in Pa, positive in compression.
    pub pressure: f64,
    /// Artificial bulk-viscosity pressure, in Pa.
    pub viscous_pressure: f64,
    /// Specific internal energy above the reference state, in J/kg.
    pub specific_energy: f64,
    /// Accumulated equivalent plastic strain.
    pub plastic_strain: f64,
    /// Equivalent plastic strain rate over the last step, in 1/s.
    pub plastic_strain_rate: f64,
    /// Temperature, in K.
    pub temperature: f64,
    /// Accumulated ductile damage in `[0, 1]`; at one the material has failed.
    pub damage: f64,
    /// Fastest wave speed in the material, in m/s; limits the time step.
    pub sound_speed: f64,
}

impl Particle3 {
    /// Current density, in kg/m³.
    #[must_use]
    pub fn density(&self) -> f64 {
        self.mass / self.volume
    }

    /// Whether the material point has failed.
    #[must_use]
    pub fn is_failed(&self) -> bool {
        self.damage >= 1.0
    }

    /// Von Mises equivalent stress, `sqrt(3/2 · s:s)`, in Pa.
    #[must_use]
    pub fn equivalent_stress(&self) -> f64 {
        equivalent_stress(&self.deviator)
    }
}

/// Von Mises equivalent stress of a deviatoric stress tensor.
#[must_use]
pub fn equivalent_stress(deviator: &DMat3) -> f64 {
    let squares = deviator.x_axis.length_squared()
        + deviator.y_axis.length_squared()
        + deviator.z_axis.length_squared();
    (1.5 * squares).sqrt()
}

/// A numerical back end that advances a [`Model3`] in time.
pub trait Solver3 {
    /// Simulation time reached so far, in seconds.
    fn time(&self) -> f64;

    /// Advances by one stable time step, shortened to `max_time_step` if
    /// necessary, and returns the step taken, in seconds.
    ///
    /// # Errors
    /// Returns [`SolverError`] if the state has become invalid.
    fn step(&mut self, max_time_step: f64) -> Result<f64, SolverError>;

    /// Current state of every particle still being simulated.
    fn particles(&self) -> Cow<'_, [Particle3]>;

    /// Particles that left the simulation through an outflow wall, in the
    /// state they left in.
    fn departed(&self) -> Cow<'_, [Particle3]> {
        Cow::Borrowed(&[])
    }

    /// Steps until `end_time` is reached and returns the number of steps.
    ///
    /// # Errors
    /// Returns [`SolverError`] if the state has become invalid.
    fn run_until(&mut self, end_time: f64) -> Result<usize, SolverError> {
        let mut steps = 0;
        while self.time() < end_time {
            self.step(end_time - self.time())?;
            steps += 1;
        }
        Ok(steps)
    }
}

#[cfg(test)]
mod tests {
    use glam::{DMat3, DVec3};
    use rap_materials::library;

    use super::{Grid3, Model3, ParticleSeed3, Walls3, equivalent_stress};
    use crate::model::{ModelError, SolverConfig};

    fn model() -> Model3 {
        Model3 {
            grid: Grid3 {
                cells: [10, 8, 6],
                cell_size: 1.0e-3,
            },
            walls: Walls3::default(),
            config: SolverConfig::default(),
            materials: vec![library::steel_4340()],
            seeds: vec![ParticleSeed3 {
                position: DVec3::new(5.0e-3, 4.0e-3, 3.0e-3),
                velocity: DVec3::ZERO,
                volume: 1.25e-10,
                material: 0,
                body: 0,
            }],
            initial_temperature: 294.0,
        }
    }

    #[test]
    fn grid_counts_nodes_and_excludes_wall_bands() {
        let grid = model().grid;
        assert_eq!(grid.nodes(), [11, 9, 7]);
        assert_eq!(grid.node_count(), 11 * 9 * 7);
        let (lower, upper) = grid.interior();
        assert!((lower - DVec3::splat(2.0e-3)).length() < 1e-15);
        assert!((upper - DVec3::new(8.0e-3, 6.0e-3, 4.0e-3)).length() < 1e-15);
    }

    #[test]
    fn consistent_model_is_accepted_and_inconsistencies_reported() {
        assert_eq!(model().validate(), Ok(()));
        assert_eq!(model().body_count(), 1);

        let mut small = model();
        small.grid.cells = [10, 8, 4];
        assert!(matches!(
            small.validate(),
            Err(ModelError::GridTooSmall(..))
        ));

        let mut outside = model();
        outside.seeds[0].position.z = 1.0e-3;
        assert_eq!(
            outside.validate(),
            Err(ModelError::ParticleOutsideWalls { particle: 0 })
        );

        let mut unknown = model();
        unknown.seeds[0].material = 2;
        assert!(matches!(
            unknown.validate(),
            Err(ModelError::UnknownMaterial { .. })
        ));

        let mut empty = model();
        empty.seeds.clear();
        assert_eq!(empty.validate(), Err(ModelError::NoParticles));
    }

    #[test]
    fn equivalent_stress_of_uniaxial_and_shear_states() {
        // Uniaxial stress σ along x: s = diag(2σ/3, −σ/3, −σ/3).
        let stress = 300.0e6;
        let uniaxial = DMat3::from_diagonal(DVec3::new(2.0, -1.0, -1.0) * stress / 3.0);
        assert!((equivalent_stress(&uniaxial) - stress).abs() < 1e-3);
        // Pure shear τ: equivalent stress √3·τ.
        let mut shear = DMat3::ZERO;
        shear.x_axis.y = 100.0e6;
        shear.y_axis.x = 100.0e6;
        assert!((equivalent_stress(&shear) - 3.0_f64.sqrt() * 100.0e6).abs() < 1e-3);
    }
}
