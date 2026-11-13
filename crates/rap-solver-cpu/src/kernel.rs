//! Quadratic B-spline interpolation between particles and grid nodes.

use glam::DVec2;

/// Offsets of the three stencil nodes along one axis, in cells.
const OFFSETS: [f64; 3] = [0.0, 1.0, 2.0];

/// The 3 x 3 block of grid nodes a particle interacts with.
pub(crate) struct Stencil {
    base: [usize; 2],
    weights: [[f64; 3]; 2],
    /// Particle position relative to the base node, in cells; each
    /// component lies in `[0.5, 1.5)`.
    fraction: DVec2,
}

impl Stencil {
    /// Builds the stencil of a particle. The position must lie at least
    /// half a cell inside the grid, which the solver checks every step.
    pub(crate) fn new(position: DVec2, inverse_cell_size: f64) -> Self {
        let scaled = position * inverse_cell_size;
        let base = (scaled - 0.5).floor();
        let fraction = scaled - base;
        Self {
            base: [to_index(base.x), to_index(base.y)],
            weights: [weights(fraction.x), weights(fraction.y)],
            fraction,
        }
    }

    /// Calls `visit(node, weight, arm)` for every stencil node, where `node`
    /// is the flat node index and `arm` the vector from the particle to the
    /// node, in m.
    pub(crate) fn for_each(
        &self,
        row_length: usize,
        cell_size: f64,
        mut visit: impl FnMut(usize, f64, DVec2),
    ) {
        for (j, (weight_y, offset_y)) in self.weights[1].into_iter().zip(OFFSETS).enumerate() {
            let row = (self.base[1] + j) * row_length;
            for (i, (weight_x, offset_x)) in self.weights[0].into_iter().zip(OFFSETS).enumerate() {
                let arm = (DVec2::new(offset_x, offset_y) - self.fraction) * cell_size;
                visit(row + self.base[0] + i, weight_x * weight_y, arm);
            }
        }
    }
}

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

#[cfg(test)]
mod tests {
    use glam::{DMat2, DVec2};
    use proptest::prelude::*;
    use rap_solver::model::count_to_f64;

    use super::Stencil;

    const CELL: f64 = 2.0e-3;
    const ROW: usize = 16;

    proptest! {
        #[test]
        fn weights_reproduce_constant_and_linear_fields(x in 1.0e-3..2.9e-2_f64, y in 1.0e-3..2.9e-2_f64) {
            let position = DVec2::new(x, y);
            let stencil = Stencil::new(position, 1.0 / CELL);
            let mut total = 0.0;
            let mut smallest = f64::MAX;
            let mut first_moment = DVec2::ZERO;
            let mut second_moment = DMat2::ZERO;
            let mut reconstructed = DVec2::ZERO;
            stencil.for_each(ROW, CELL, |node, weight, arm| {
                smallest = smallest.min(weight);
                total += weight;
                first_moment += weight * arm;
                second_moment += DMat2::from_cols(arm * arm.x, arm * arm.y) * weight;
                let node_position = DVec2::new(count_to_f64(node % ROW), count_to_f64(node / ROW)) * CELL;
                reconstructed += weight * node_position;
            });
            prop_assert!(smallest >= 0.0);
            prop_assert!((total - 1.0).abs() < 1e-12);
            prop_assert!(first_moment.length() < 1e-15);
            prop_assert!((reconstructed - position).length() < 1e-14);
            // The MLS-MPM force and velocity-gradient formulas rely on
            // Σ w·(x_i − x_p)(x_i − x_p)ᵀ = Δx²/4 · I.
            let expected = DMat2::IDENTITY * (CELL * CELL / 4.0);
            prop_assert!((second_moment - expected).abs_diff_eq(DMat2::ZERO, 1e-18));
        }
    }
}
