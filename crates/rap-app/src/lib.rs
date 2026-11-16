//! Application use cases shared by the front ends: turning a scenario into
//! a solver model, running it and reporting the outcome.

pub mod assemble;
pub mod presets;
pub mod simulation;

use rap_domain::{DomainError, MaterialId};
use rap_solver::SolverError;
use thiserror::Error;

pub use assemble::{Assembly, Layout, ModelKind, SolidAssembly, assemble, assemble_solid};
pub use simulation::Simulation;

/// Reasons a scenario cannot be simulated.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum AppError {
    /// The scenario or its settings are invalid.
    #[error(transparent)]
    Invalid(#[from] DomainError),
    /// The scenario refers to a material that is neither built in nor
    /// defined alongside it.
    #[error("material `{0}` is not defined; run `rap materials` to list the built-in ones")]
    UnknownMaterial(MaterialId),
    /// The requested resolution needs more grid nodes than is reasonable.
    #[error(
        "the grid would have {nodes} nodes, more than the limit of {limit}; lower \
         `cells_per_radius` or `armor_extent_radii`",
        limit = assemble::MAX_NODES
    )]
    GridTooLarge {
        /// Number of nodes the grid would need.
        nodes: usize,
    },
    /// A three-dimensional model was requested on the GPU solver, which
    /// only handles two-dimensional models.
    #[error(
        "the GPU solver cannot run three-dimensional models yet; use the CPU solver, or model \
         the oblique impact in plane strain"
    )]
    SolidOnGpu,
    /// The solver rejected the model or failed while running.
    #[error(transparent)]
    Solver(#[from] SolverError),
}
