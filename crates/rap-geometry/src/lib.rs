//! Parametric shapes, solid construction and particle sampling.

use glam::DVec2;

/// Fills the axis-aligned rectangle between `lower` and `upper` with a
/// regular lattice of points, one at the centre of every `spacing`-sized
/// square. Each point represents a volume of `spacing²` per unit depth.
///
/// The rectangle is covered by whole squares: its size is rounded to the
/// nearest multiple of `spacing` along each axis.
#[must_use]
pub fn fill_rectangle(lower: DVec2, upper: DVec2, spacing: f64) -> Vec<DVec2> {
    let counts = ((upper - lower) / spacing).round().max(DVec2::ZERO);
    let (columns, rows) = (to_count(counts.x), to_count(counts.y));
    let mut points = Vec::with_capacity(columns * rows);
    let mut y = lower.y + 0.5 * spacing;
    for _ in 0..rows {
        let mut x = lower.x + 0.5 * spacing;
        for _ in 0..columns {
            points.push(DVec2::new(x, y));
            x += spacing;
        }
        y += spacing;
    }
    points
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

    use super::fill_rectangle;

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
}
