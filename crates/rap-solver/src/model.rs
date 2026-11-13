//! Description of a problem handed to a solver.

use glam::DVec2;
use rap_domain::validation::{self, InvalidParameter};
use rap_materials::Material;
use thiserror::Error;

/// Number of grid cells between each edge of the grid and the wall plane.
/// The band keeps particle interpolation stencils inside the grid.
pub const WALL_CELLS: usize = 2;

/// Uniform background grid with its origin at `(0, 0)`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Grid {
    /// Number of cells along `x` and `y`.
    pub cells: [usize; 2],
    /// Edge length of a cell, in m.
    pub cell_size: f64,
}

impl Grid {
    /// Number of nodes along `x` and `y`.
    #[must_use]
    pub fn nodes(&self) -> [usize; 2] {
        [self.cells[0] + 1, self.cells[1] + 1]
    }

    /// Total number of nodes.
    #[must_use]
    pub fn node_count(&self) -> usize {
        let [x, y] = self.nodes();
        x * y
    }

    /// Lower and upper corners of the region enclosed by the walls, in m.
    #[must_use]
    pub fn interior(&self) -> (DVec2, DVec2) {
        let lower = DVec2::splat(count_to_f64(WALL_CELLS) * self.cell_size);
        let upper = DVec2::new(
            count_to_f64(self.cells[0] - WALL_CELLS),
            count_to_f64(self.cells[1] - WALL_CELLS),
        ) * self.cell_size;
        (lower, upper)
    }
}

/// Converts a cell or node count to `f64`.
#[expect(
    clippy::cast_precision_loss,
    reason = "grid dimensions are far below 2^52, where the conversion is exact"
)]
#[must_use]
pub fn count_to_f64(count: usize) -> f64 {
    count as f64
}

/// Behaviour of a rigid wall.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Wall {
    /// Blocks motion into the wall; material may slide along it and separate.
    #[default]
    Frictionless,
    /// Holds touching material at rest.
    Sticky,
}

/// The four rigid walls enclosing the grid interior.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Walls {
    /// Wall at the low-`x` side.
    pub left: Wall,
    /// Wall at the high-`x` side.
    pub right: Wall,
    /// Wall at the low-`y` side.
    pub bottom: Wall,
    /// Wall at the high-`y` side.
    pub top: Wall,
}

/// Numerical parameters of a solver.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SolverConfig {
    /// Fraction of the Courant limit used as the time step, in `(0, 1]`.
    pub cfl: f64,
    /// Coefficient of the linear artificial bulk viscosity.
    pub linear_viscosity: f64,
    /// Coefficient of the quadratic artificial bulk viscosity.
    pub quadratic_viscosity: f64,
}

impl Default for SolverConfig {
    fn default() -> Self {
        Self {
            cfl: 0.3,
            linear_viscosity: 0.2,
            quadratic_viscosity: 1.5,
        }
    }
}

/// Initial state of one material point.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ParticleSeed {
    /// Position, in m.
    pub position: DVec2,
    /// Velocity, in m/s.
    pub velocity: DVec2,
    /// Volume represented by the particle, in m³ per metre of depth.
    pub volume: f64,
    /// Index into [`Model::materials`].
    pub material: usize,
}

/// A complete problem: grid, boundaries, materials and initial particles.
#[derive(Debug, Clone, PartialEq)]
pub struct Model {
    /// Background grid.
    pub grid: Grid,
    /// Rigid walls around the grid interior.
    pub walls: Walls,
    /// Numerical parameters.
    pub config: SolverConfig,
    /// Materials referenced by the particles.
    pub materials: Vec<Material>,
    /// Initial particles.
    pub seeds: Vec<ParticleSeed>,
    /// Uniform initial temperature, in K.
    pub initial_temperature: f64,
}

/// Reasons a [`Model`] is rejected.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum ModelError {
    /// A numeric parameter is out of range.
    #[error(transparent)]
    InvalidParameter(#[from] InvalidParameter),
    /// The grid is too small to hold the wall bands and an interior.
    #[error("grid of {0} x {1} cells is too small; each side needs at least {min} cells", min = 2 * WALL_CELLS + 1)]
    GridTooSmall(usize, usize),
    /// The model has no particles.
    #[error("model contains no particles")]
    NoParticles,
    /// A particle refers to a material that is not in the model.
    #[error("particle {particle} refers to unknown material index {material}")]
    UnknownMaterial {
        /// Index of the particle.
        particle: usize,
        /// The invalid material index.
        material: usize,
    },
    /// A particle starts outside the region enclosed by the walls.
    #[error("particle {particle} starts outside the walls")]
    ParticleOutsideWalls {
        /// Index of the particle.
        particle: usize,
    },
}

impl Model {
    /// Checks the model for internal consistency.
    ///
    /// # Errors
    /// Returns the first [`ModelError`] found.
    pub fn validate(&self) -> Result<(), ModelError> {
        validation::positive("grid.cell_size", self.grid.cell_size)?;
        let [cells_x, cells_y] = self.grid.cells;
        if cells_x.min(cells_y) <= 2 * WALL_CELLS {
            return Err(ModelError::GridTooSmall(cells_x, cells_y));
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
            validation::finite("seed.velocity.x", seed.velocity.x)?;
            validation::finite("seed.velocity.y", seed.velocity.y)?;
            if seed.material >= self.materials.len() {
                return Err(ModelError::UnknownMaterial {
                    particle,
                    material: seed.material,
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

#[cfg(test)]
mod tests {
    use glam::DVec2;
    use rap_materials::library;

    use super::{Grid, Model, ModelError, ParticleSeed, SolverConfig, Walls};

    fn model() -> Model {
        Model {
            grid: Grid {
                cells: [10, 8],
                cell_size: 1.0e-3,
            },
            walls: Walls::default(),
            config: SolverConfig::default(),
            materials: vec![library::steel_4340()],
            seeds: vec![ParticleSeed {
                position: DVec2::new(5.0e-3, 4.0e-3),
                velocity: DVec2::ZERO,
                volume: 2.5e-7,
                material: 0,
            }],
            initial_temperature: 294.0,
        }
    }

    #[test]
    fn consistent_model_is_accepted() {
        assert_eq!(model().validate(), Ok(()));
    }

    #[test]
    fn interior_excludes_the_wall_bands() {
        let (lower, upper) = model().grid.interior();
        assert!((lower - DVec2::splat(2.0e-3)).length() < 1e-15);
        assert!((upper - DVec2::new(8.0e-3, 6.0e-3)).length() < 1e-15);
    }

    #[test]
    fn inconsistencies_are_reported() {
        let mut small = model();
        small.grid.cells = [4, 8];
        assert_eq!(small.validate(), Err(ModelError::GridTooSmall(4, 8)));

        let mut empty = model();
        empty.seeds.clear();
        assert_eq!(empty.validate(), Err(ModelError::NoParticles));

        let mut unknown = model();
        unknown.seeds[0].material = 3;
        assert_eq!(
            unknown.validate(),
            Err(ModelError::UnknownMaterial {
                particle: 0,
                material: 3
            })
        );

        let mut outside = model();
        outside.seeds[0].position.x = 1.0e-3;
        assert_eq!(
            outside.validate(),
            Err(ModelError::ParticleOutsideWalls { particle: 0 })
        );

        let mut unstable = model();
        unstable.config.cfl = 0.0;
        assert!(
            matches!(unstable.validate(), Err(ModelError::InvalidParameter(e)) if e.field == "config.cfl")
        );
    }
}
