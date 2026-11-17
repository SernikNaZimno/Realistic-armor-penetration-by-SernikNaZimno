//! When two bodies sharing a grid node actually touch.
//!
//! Each body has its own velocity field on the grid, and a node carries
//! mass of a body as soon as one of its particles is within a cell and a
//! half. Treating every shared node as contact therefore makes bodies
//! interact across a gap of up to three cells, which thickens a penetrator
//! by an amount that shrinks only linearly with the cell size.
//!
//! The test here uses what a node already knows: the mass of each body and
//! where its centre of mass, weighted by the interpolation kernel, lies.
//! For two bodies that fill the space on either side of a plane, the
//! distance between those centres along the normal depends only on how the
//! node's mass is shared between them. If the centres are further apart
//! than that, there is a gap, and the bodies do not yet touch.

/// Share of a node's mass below which a body is treated as having this
/// share; keeps the formulae finite.
const MIN_SHARE: f64 = 1.0e-9;
/// Share at which the plane between the bodies is half a cell from the
/// node, where the kernel changes from its outer to its inner piece.
const OUTER_SHARE: f64 = 1.0 / 6.0;
/// First moment of half of the kernel about its centre, in cells.
const HALF_MOMENT: f64 = 0.203_125;
/// Newton steps that find where the plane is from the share; the starting
/// guess is good to a few percent.
const NEWTON_STEPS: usize = 4;

/// How much further apart than touching, in cells, the centres of two
/// bodies may be and still count as in contact. Contact a little early is
/// harmless; contact late lets bodies overlap.
pub const CONTACT_MARGIN: f64 = 0.02;

/// Distance, in cells, between the kernel-weighted centres of mass of two
/// bodies that touch along a plane, when the smaller body holds `share` of
/// the mass at the node (at most one half).
///
/// The kernel is the quadratic B-spline of the solvers. Along a direction
/// that is not a grid axis the result differs by about two percent.
#[must_use]
pub fn touching_separation(share: f64) -> f64 {
    let share = if share.is_nan() {
        MIN_SHARE
    } else {
        share.clamp(MIN_SHARE, 0.5)
    };
    // First moment, about the node, of the part of the kernel beyond the
    // plane; the part on the near side has the opposite moment.
    let moment = if share <= OUTER_SHARE {
        // The plane is `1.5 − depth` cells from the node.
        let depth = (6.0 * share).cbrt();
        0.25 * depth.powi(3) - 0.125 * depth.powi(4)
    } else {
        // Distance of the plane from the node: s³/3 − 3s/4 + (1/2 − share) = 0.
        let missing = 0.5 - share;
        let mut distance = missing / 0.75;
        for _ in 0..NEWTON_STEPS {
            distance -=
                (distance.powi(3) / 3.0 - 0.75 * distance + missing) / (distance * distance - 0.75);
        }
        HALF_MOMENT - 0.375 * distance * distance + 0.25 * distance.powi(4)
    };
    moment / (share * (1.0 - share))
}

/// Whether two bodies touch at a node, given the distance between their
/// kernel-weighted centres of mass along the contact normal, in cells, and
/// the share of the node's mass that the smaller of them holds.
#[must_use]
pub fn touches(separation: f64, share: f64) -> bool {
    separation <= touching_separation(share) + CONTACT_MARGIN
}

#[cfg(test)]
mod tests {
    use super::{touches, touching_separation};

    /// The solvers' kernel at `x` cells from the node.
    fn kernel(x: f64) -> f64 {
        let x = x.abs();
        if x < 0.5 {
            0.75 - x * x
        } else if x < 1.5 {
            0.5 * (1.5 - x).powi(2)
        } else {
            0.0
        }
    }

    /// Share of the smaller body and distance between the centres of two
    /// bodies whose surfaces are `gap` apart about a plane `plane` cells
    /// from the node, by numerical integration.
    fn measured(plane: f64, gap: f64) -> (f64, f64) {
        let steps = 300_000;
        let (mut near, mut far) = ((0.0, 0.0), (0.0, 0.0));
        for step in 0..steps {
            let x = -1.5 + 3.0 * (f64::from(step) + 0.5) / f64::from(steps);
            let weight = kernel(x);
            if x < plane - gap / 2.0 {
                near = (near.0 + weight, near.1 + weight * x);
            } else if x > plane + gap / 2.0 {
                far = (far.0 + weight, far.1 + weight * x);
            }
        }
        let share = near.0.min(far.0) / (near.0 + far.0);
        (share, far.1 / far.0 - near.1 / near.0)
    }

    #[test]
    fn touching_bodies_are_as_far_apart_as_the_formula_says() {
        for plane in [0.0, 0.1, 0.3, 0.5, 0.7, 1.0, 1.3] {
            let (share, separation) = measured(plane, 0.0);
            let expected = touching_separation(share);
            assert!(
                (separation - expected).abs() < 1e-4,
                "plane at {plane}: {separation} against {expected}"
            );
            assert!(touches(separation, share));
        }
        // Equal shares: twice the mean distance of half a kernel.
        assert!((touching_separation(0.5) - 0.8125).abs() < 1e-12);
        assert!((touching_separation(1.0 / 6.0) - 0.9).abs() < 1e-12);
    }

    #[test]
    fn a_gap_of_a_tenth_of_a_cell_is_told_from_contact() {
        // Further from the node than this the smaller body holds under two
        // percent of the mass and the gap no longer shows.
        for plane in [0.0, 0.25, 0.5, 0.8] {
            let (share, separation) = measured(plane, 0.1);
            assert!(!touches(separation, share), "plane at {plane}");
            // Overlapping bodies are in contact all the more.
            let (share, separation) = measured(plane, -0.2);
            assert!(touches(separation, share), "plane at {plane}");
        }
    }

    #[test]
    fn extreme_shares_stay_finite() {
        for share in [0.0, 1.0e-12, 1.0e-6, 0.5, 0.7, f64::NAN] {
            let separation = touching_separation(share);
            assert!(separation.is_finite() && separation > 0.8 && separation <= 1.5 + 1e-9);
        }
    }
}
