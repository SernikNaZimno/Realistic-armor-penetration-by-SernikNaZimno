//! Parametric shapes, solid construction and particle sampling.

pub mod nose;

use glam::DVec2;

/// Fills the part of the axis-aligned rectangle between `lower` and `upper`
/// selected by `keep` with a regular lattice of points, one at the centre of
/// every `spacing`-sized square. Each point represents a cross-section area
/// of `spacing²`.
///
/// The rectangle is covered by whole squares: its size is rounded to the
/// nearest multiple of `spacing` along each axis.
#[must_use]
pub fn fill_region(
    lower: DVec2,
    upper: DVec2,
    spacing: f64,
    keep: impl Fn(DVec2) -> bool,
) -> Vec<DVec2> {
    let counts = ((upper - lower) / spacing).round().max(DVec2::ZERO);
    let (columns, rows) = (to_count(counts.x), to_count(counts.y));
    let mut points = Vec::with_capacity(columns * rows);
    let mut y = lower.y + 0.5 * spacing;
    for _ in 0..rows {
        let mut x = lower.x + 0.5 * spacing;
        for _ in 0..columns {
            let point = DVec2::new(x, y);
            if keep(point) {
                points.push(point);
            }
            x += spacing;
        }
        y += spacing;
    }
    points
}

/// Fills the whole rectangle between `lower` and `upper`; see
/// [`fill_region`].
#[must_use]
pub fn fill_rectangle(lower: DVec2, upper: DVec2, spacing: f64) -> Vec<DVec2> {
    fill_region(lower, upper, spacing, |_| true)
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the value is a rounded, non-negative lattice count"
)]
fn to_count(value: f64) -> usize {
    value as usize
}

#[cfg(test)]
mod tests {
    use glam::DVec2;

    use super::{fill_rectangle, fill_region};

    #[test]
    fn rectangle_is_filled_with_cell_centred_points() {
        let points = fill_rectangle(DVec2::new(1.0, 2.0), DVec2::new(2.0, 2.5), 0.25);
        assert_eq!(points.len(), 4 * 2);
        assert!((points[0] - DVec2::new(1.125, 2.125)).length() < 1e-12);
        assert!((points[7] - DVec2::new(1.875, 2.375)).length() < 1e-12);
        let centroid = points.iter().sum::<DVec2>() / 8.0;
        assert!((centroid - DVec2::new(1.5, 2.25)).length() < 1e-12);
    }

    #[test]
    fn degenerate_rectangle_is_empty() {
        assert_eq!(fill_rectangle(DVec2::ONE, DVec2::ONE, 0.1), Vec::new());
        assert_eq!(fill_rectangle(DVec2::ONE, DVec2::ZERO, 0.1), Vec::new());
    }

    #[test]
    fn region_keeps_only_selected_points() {
        let disc = fill_region(DVec2::splat(-1.0), DVec2::splat(1.0), 0.01, |point| {
            point.length() <= 1.0
        });
        let count = u32::try_from(disc.len()).expect("a few thousand points");
        let area = 1.0e-4 * f64::from(count);
        assert!((area / std::f64::consts::PI - 1.0).abs() < 2e-3, "{area}");
    }
}
