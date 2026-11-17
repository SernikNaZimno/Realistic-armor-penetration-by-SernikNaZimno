//! Closed triangle meshes and their conversion to particles.
//!
//! A mesh is turned into particles by casting one line along `x` through
//! every lattice column and keeping the lattice points between an odd and
//! the following even crossing of the surface. A line that passes exactly
//! through an edge or a vertex is treated as if it were displaced by an
//! infinitesimal amount, the same for every triangle, so that it crosses a
//! closed surface an even number of times whatever the mesh looks like.

use glam::{DVec2, DVec3};
use thiserror::Error;

/// Reasons a triangle mesh cannot be used.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum MeshError {
    /// The mesh has no triangles.
    #[error("the mesh has no triangles")]
    Empty,
    /// A vertex coordinate is infinite or not a number.
    #[error("the mesh has a vertex that is not a finite point")]
    NotFinite,
    /// The surface has a hole, so inside and outside cannot be told apart.
    #[error(
        "the mesh is not a closed surface: a line through it crosses the surface an odd number \
         of times"
    )]
    NotClosed,
}

/// A regular set of lines parallel to `x`, one through the centre of every
/// `spacing`-sized square of a rectangle in the `y`–`z` plane.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Columns {
    /// Corner of the rectangle with the smallest `y` and `z`.
    pub lower: DVec2,
    /// Distance between neighbouring lines, in m.
    pub spacing: f64,
    /// Number of lines along `y` and along `z`.
    pub counts: [usize; 2],
}

impl Columns {
    /// The single line through `point` of the `y`–`z` plane.
    #[must_use]
    pub fn single(point: DVec2) -> Self {
        Self {
            lower: point - DVec2::splat(0.5),
            spacing: 1.0,
            counts: [1, 1],
        }
    }

    /// Number of lines.
    #[must_use]
    pub fn len(&self) -> usize {
        self.counts[0] * self.counts[1]
    }

    /// Whether there is no line at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Position in the `y`–`z` plane of line `index` along one axis.
    fn coordinate(&self, axis: usize, index: usize) -> f64 {
        self.lower.to_array()[axis] + (to_f64(index) + 0.5) * self.spacing
    }

    /// Indices along `axis` of the lines between `low` and `high`.
    fn range(&self, axis: usize, low: f64, high: f64) -> std::ops::Range<usize> {
        let origin = self.lower.to_array()[axis];
        let first = ((low - origin) / self.spacing - 0.5).floor().max(0.0);
        let last = ((high - origin) / self.spacing - 0.5).ceil();
        let count = self.counts[axis];
        if last < 0.0 {
            return 0..0;
        }
        to_index(first).min(count)..(to_index(last) + 1).min(count)
    }
}

#[expect(
    clippy::cast_precision_loss,
    reason = "lattice counts are far below 2^53"
)]
fn to_f64(count: usize) -> f64 {
    count as f64
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the value is a non-negative whole number clamped by the caller"
)]
fn to_index(value: f64) -> usize {
    value as usize
}

/// A surface made of triangles.
#[derive(Debug, Clone, PartialEq)]
pub struct TriangleMesh {
    triangles: Vec<[DVec3; 3]>,
}

/// Which side of the line from `start` to `end` the point `point` is on, in
/// the `y`–`z` plane: positive on the left. A point exactly on the line is
/// placed as if it were displaced by an infinitesimal step towards `-y` and
/// a still smaller one towards `+z`. The result is the same, with the sign
/// reversed, when the end points are swapped.
fn side(start: DVec2, end: DVec2, point: DVec2) -> f64 {
    // Always evaluate from the same end so that two triangles sharing the
    // edge agree to the last bit.
    let swapped = (end.x, end.y) < (start.x, start.y);
    let (from, to) = if swapped { (end, start) } else { (start, end) };
    let edge = to - from;
    let mut value = edge.perp_dot(point - from);
    if value == 0.0 {
        value = if edge.y == 0.0 { edge.x } else { edge.y };
    }
    if swapped { -value } else { value }
}

impl TriangleMesh {
    /// A mesh of `triangles`, each given by its three corners.
    ///
    /// # Errors
    /// Returns [`MeshError`] if there is no triangle or a corner is not a
    /// finite point.
    pub fn new(triangles: Vec<[DVec3; 3]>) -> Result<Self, MeshError> {
        if triangles.is_empty() {
            return Err(MeshError::Empty);
        }
        if triangles.iter().flatten().any(|corner| !corner.is_finite()) {
            return Err(MeshError::NotFinite);
        }
        Ok(Self { triangles })
    }

    /// The closed box between the corners `lower` and `upper`.
    #[must_use]
    pub fn cuboid(lower: DVec3, upper: DVec3) -> Self {
        let corner = |index: usize| {
            DVec3::new(
                if index & 1 == 0 { lower.x } else { upper.x },
                if index & 2 == 0 { lower.y } else { upper.y },
                if index & 4 == 0 { lower.z } else { upper.z },
            )
        };
        let faces = [
            [0, 2, 3, 1],
            [4, 5, 7, 6],
            [0, 1, 5, 4],
            [2, 6, 7, 3],
            [0, 4, 6, 2],
            [1, 3, 7, 5],
        ];
        Self {
            triangles: faces
                .into_iter()
                .flat_map(|[a, b, c, d]| {
                    [
                        [corner(a), corner(b), corner(c)],
                        [corner(a), corner(c), corner(d)],
                    ]
                })
                .collect(),
        }
    }

    /// The triangles, each as its three corners.
    #[must_use]
    pub fn triangles(&self) -> &[[DVec3; 3]] {
        &self.triangles
    }

    /// Corners with the smallest and the largest coordinates of the box
    /// that just contains the mesh.
    #[must_use]
    pub fn bounds(&self) -> (DVec3, DVec3) {
        self.triangles.iter().flatten().fold(
            (DVec3::INFINITY, DVec3::NEG_INFINITY),
            |(lower, upper), corner| (lower.min(*corner), upper.max(*corner)),
        )
    }

    /// The mesh with every corner replaced by `map` of it.
    #[must_use]
    pub fn mapped(&self, map: impl Fn(DVec3) -> DVec3) -> Self {
        Self {
            triangles: self
                .triangles
                .iter()
                .map(|triangle| triangle.map(&map))
                .collect(),
        }
    }

    /// For every line of `columns`, the `x` coordinates at which it crosses
    /// the surface, in increasing order. Lines are numbered along `y`
    /// first: line `j + k * counts[0]` is the `j`-th along `y` and the
    /// `k`-th along `z`.
    ///
    /// # Errors
    /// Returns [`MeshError::NotClosed`] if a line crosses the surface an
    /// odd number of times.
    pub fn crossings(&self, columns: &Columns) -> Result<Vec<Vec<f64>>, MeshError> {
        let mut crossings = vec![Vec::new(); columns.len()];
        for triangle in &self.triangles {
            let [a, b, c] = triangle.map(|corner| DVec2::new(corner.y, corner.z));
            let lower = a.min(b).min(c);
            let upper = a.max(b).max(c);
            let normal = (triangle[1] - triangle[0]).cross(triangle[2] - triangle[0]);
            // A triangle seen edge-on is never crossed by a displaced line.
            if normal.x == 0.0 {
                continue;
            }
            let orientation = normal.x.signum();
            for k in columns.range(1, lower.y, upper.y) {
                for j in columns.range(0, lower.x, upper.x) {
                    let point = DVec2::new(columns.coordinate(0, j), columns.coordinate(1, k));
                    let inside = [(a, b), (b, c), (c, a)]
                        .into_iter()
                        .all(|(start, end)| orientation * side(start, end, point) > 0.0);
                    if inside {
                        // The plane of the triangle at the line.
                        let offset = DVec3::new(triangle[0].x, point.x, point.y) - triangle[0];
                        let x = triangle[0].x - offset.dot(normal) / normal.x;
                        crossings[j + k * columns.counts[0]].push(x);
                    }
                }
            }
        }
        for line in &mut crossings {
            if line.len() % 2 != 0 {
                return Err(MeshError::NotClosed);
            }
            line.sort_by(f64::total_cmp);
        }
        Ok(crossings)
    }

    /// Fills the part of the mesh inside the axis-aligned box between
    /// `lower` and `upper` with a regular lattice of points, one at the
    /// centre of every `spacing`-sized cube of the box that lies inside the
    /// surface. Each point represents a volume of `spacing³`. The box size
    /// is rounded to the nearest multiple of `spacing` along each axis.
    ///
    /// # Errors
    /// Returns [`MeshError::NotClosed`] if the surface has a hole.
    pub fn fill(&self, lower: DVec3, upper: DVec3, spacing: f64) -> Result<Vec<DVec3>, MeshError> {
        let counts = ((upper - lower) / spacing).round().max(DVec3::ZERO);
        let [along, across, up] = counts.to_array().map(to_index);
        let columns = Columns {
            lower: DVec2::new(lower.y, lower.z),
            spacing,
            counts: [across, up],
        };
        let crossings = self.crossings(&columns)?;
        let mut points = Vec::new();
        for (line, crossings) in crossings.iter().enumerate() {
            let y = columns.coordinate(0, line % across);
            let z = columns.coordinate(1, line / across);
            for pair in crossings.chunks_exact(2) {
                let first = ((pair[0] - lower.x) / spacing - 0.5).ceil().max(0.0);
                for index in to_index(first)..along {
                    let x = lower.x + (to_f64(index) + 0.5) * spacing;
                    if x >= pair[1] {
                        break;
                    }
                    points.push(DVec3::new(x, y, z));
                }
            }
        }
        Ok(points)
    }
}

#[cfg(test)]
mod tests {
    use glam::{DVec2, DVec3};

    use super::{Columns, MeshError, TriangleMesh};

    /// The triangles of the closed box between `lower` and `upper`.
    fn cuboid(lower: DVec3, upper: DVec3) -> Vec<[DVec3; 3]> {
        TriangleMesh::cuboid(lower, upper).triangles().to_vec()
    }

    /// A closed octahedron with its corners `radius` from `centre`.
    fn octahedron(centre: DVec3, radius: f64) -> Vec<[DVec3; 3]> {
        let mut triangles = Vec::new();
        for x in [-1.0, 1.0] {
            for y in [-1.0, 1.0] {
                for z in [-1.0, 1.0] {
                    triangles.push([
                        centre + DVec3::X * x * radius,
                        centre + DVec3::Y * y * radius,
                        centre + DVec3::Z * z * radius,
                    ]);
                }
            }
        }
        triangles
    }

    #[test]
    fn empty_and_non_finite_meshes_are_rejected() {
        assert_eq!(TriangleMesh::new(Vec::new()), Err(MeshError::Empty));
        let bad = vec![[DVec3::ZERO, DVec3::X, DVec3::new(0.0, f64::NAN, 0.0)]];
        assert_eq!(TriangleMesh::new(bad), Err(MeshError::NotFinite));
    }

    #[test]
    fn bounds_contain_every_corner() {
        let mesh = TriangleMesh::new(cuboid(
            DVec3::new(-1.0, 2.0, 3.0),
            DVec3::new(4.0, 5.0, 6.0),
        ))
        .expect("a valid mesh");
        assert_eq!(
            mesh.bounds(),
            (DVec3::new(-1.0, 2.0, 3.0), DVec3::new(4.0, 5.0, 6.0))
        );
    }

    #[test]
    fn line_through_a_box_crosses_its_two_faces() {
        let mesh = TriangleMesh::new(cuboid(DVec3::ZERO, DVec3::new(2.0, 1.0, 1.0)))
            .expect("a valid mesh");
        let through = mesh
            .crossings(&Columns::single(DVec2::new(0.3, 0.6)))
            .expect("a closed mesh");
        assert_eq!(through, vec![vec![0.0, 2.0]]);
        let beside = mesh
            .crossings(&Columns::single(DVec2::new(1.3, 0.6)))
            .expect("a closed mesh");
        assert_eq!(beside, vec![Vec::<f64>::new()]);
    }

    #[test]
    fn lines_through_edges_and_corners_cross_an_even_number_of_times() {
        // Lattice lines pass exactly through the diagonal of each face, the
        // edges and the corners of the box and of the octahedron.
        let shapes = [
            cuboid(DVec3::ZERO, DVec3::ONE),
            octahedron(DVec3::splat(0.5), 0.5),
        ];
        for triangles in shapes {
            let mesh = TriangleMesh::new(triangles).expect("a valid mesh");
            let columns = Columns {
                lower: DVec2::splat(-0.375),
                spacing: 0.25,
                counts: [7, 7],
            };
            let crossings = mesh.crossings(&columns).expect("a closed mesh");
            assert!(crossings.iter().all(|line| line.len() % 2 == 0));
            assert!(crossings.iter().any(|line| !line.is_empty()));
        }
    }

    #[test]
    fn filling_a_box_gives_its_volume_whatever_the_alignment() {
        let mesh = TriangleMesh::new(cuboid(DVec3::ZERO, DVec3::new(1.0, 0.5, 0.25)))
            .expect("a valid mesh");
        // Lattice faces coincide with the faces of the box.
        let points = mesh
            .fill(DVec3::splat(-0.25), DVec3::splat(1.25), 0.125)
            .expect("a closed mesh");
        assert_eq!(points.len(), 8 * 4 * 2);
        let centroid = points.iter().sum::<DVec3>() / 64.0;
        assert!((centroid - DVec3::new(0.5, 0.25, 0.125)).length() < 1e-12);
        // Lattice points coincide with the faces of the box.
        let shifted = mesh
            .fill(DVec3::splat(-0.3125), DVec3::splat(1.1875), 0.125)
            .expect("a closed mesh");
        assert_eq!(shifted.len(), 8 * 4 * 2);
    }

    #[test]
    fn filling_an_octahedron_gives_its_volume() {
        let mesh = TriangleMesh::new(octahedron(DVec3::ZERO, 1.0)).expect("a valid mesh");
        let spacing = 0.02;
        let points = mesh
            .fill(DVec3::splat(-1.0), DVec3::splat(1.0), spacing)
            .expect("a closed mesh");
        let count = u32::try_from(points.len()).expect("under a million points");
        let volume = spacing.powi(3) * f64::from(count);
        assert!((volume / (4.0 / 3.0) - 1.0).abs() < 0.01, "{volume}");
    }

    #[test]
    fn the_box_limits_the_filled_part() {
        let mesh = TriangleMesh::new(cuboid(DVec3::ZERO, DVec3::ONE)).expect("a valid mesh");
        let points = mesh
            .fill(DVec3::new(0.5, 0.0, 0.0), DVec3::new(2.0, 0.5, 1.0), 0.25)
            .expect("a closed mesh");
        assert_eq!(points.len(), 2 * 2 * 4);
        assert!(points.iter().all(|point| point.x > 0.5 && point.y < 0.5));
    }

    #[test]
    fn a_mesh_with_a_hole_is_reported() {
        let mut triangles = cuboid(DVec3::ZERO, DVec3::ONE);
        triangles.truncate(10);
        let mesh = TriangleMesh::new(triangles).expect("a valid mesh");
        let result = mesh.fill(DVec3::ZERO, DVec3::ONE, 0.25);
        assert_eq!(result, Err(MeshError::NotClosed));
    }

    #[test]
    fn mapping_moves_the_mesh() {
        let mesh = TriangleMesh::new(cuboid(DVec3::ZERO, DVec3::ONE)).expect("a valid mesh");
        let moved = mesh.mapped(|corner| 2.0 * corner + DVec3::X);
        assert_eq!(moved.bounds(), (DVec3::X, DVec3::new(3.0, 2.0, 2.0)));
    }
}
