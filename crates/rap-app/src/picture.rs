//! What a simulation looks like in three dimensions, for the pictures of
//! both front ends.

use rap_render::Cloud;
use rap_solver::Symmetry;

use crate::assemble::ModelKind;
use crate::simulation::Simulation;

/// The particles of `simulation` as a cloud to draw in three dimensions:
/// those of a solid model as they are, those of an axisymmetric model swept
/// half-way round the axis with at most about `budget` points. `None` for a
/// model in plane strain, which has no third dimension to show.
#[must_use]
pub fn cloud(simulation: &Simulation, budget: usize) -> Option<Cloud> {
    let layout = simulation.layout();
    if let Some(particles) = simulation.solid_particles() {
        return Some(Cloud::new(
            &particles,
            layout.symmetry_plane,
            layout.half,
            layout.armor_body,
        ));
    }
    (layout.kind == ModelKind::Axisymmetric && layout.symmetry == Symmetry::Axisymmetric).then(
        || {
            Cloud::revolved(
                &simulation.particles(),
                layout.shot_line,
                layout.armor_body,
                budget,
            )
        },
    )
}
