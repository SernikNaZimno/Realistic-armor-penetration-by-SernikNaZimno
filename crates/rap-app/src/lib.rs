//! Application use cases shared by the front ends: turning a scenario into
//! a solver model, running it and reporting the outcome.

pub mod assemble;
pub mod presets;
pub mod simulation;

use rap_domain::{DomainError, MaterialId};
use rap_geometry::MeshError;
use rap_solver::SolverError;
use thiserror::Error;

pub use assemble::mesh::{assemble_mesh, load_mesh};
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
         `cells_per_radius` or `armor_extent_radii`, or for armor given by a mesh set \
         `max_depth`",
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
    /// A model was requested for the other kind of armor than the scenario
    /// has: plates where it has a mesh, or a mesh where it has plates.
    #[error("the armor of the scenario is not of the kind this model is built from")]
    WrongArmorKind,
    /// The armor is given by a mesh, which was not supplied.
    #[error("the armor is given by a mesh file, which has to be loaded first")]
    MeshRequired,
    /// The mesh file of the armor cannot be used.
    #[error("{0}")]
    MeshFile(String),
    /// The mesh of the armor is not a closed surface.
    #[error(transparent)]
    Mesh(#[from] MeshError),
    /// The shot line does not pass through the mesh of the armor.
    #[error(
        "the shot line misses the armor mesh; check `aim_point`, `direction` and \
         `metres_per_unit`"
    )]
    ShotMissesMesh,
    /// The mesh of the armor is too small to hold a single particle.
    #[error(
        "the armor mesh is smaller than one particle at this resolution; check \
         `metres_per_unit` or raise `cells_per_radius`"
    )]
    EmptyMesh,
    /// The solver rejected the model or failed while running.
    #[error(transparent)]
    Solver(#[from] SolverError),
}
