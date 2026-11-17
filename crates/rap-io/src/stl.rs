//! Reading triangle meshes from STL files, binary or text, and writing
//! them as text.
//!
//! Only the triangles are read. Facet normals, the name of the solid and
//! the attribute bytes of the binary format are ignored; which side of the
//! surface is inside follows from the surface being closed.

use std::fmt::Write as _;
use std::fs;
use std::path::Path;

use glam::DVec3;
use rap_geometry::{MeshError, TriangleMesh};
use thiserror::Error;

use crate::IoError;

/// Length of the free-form header of a binary STL file, in bytes.
const HEADER_BYTES: usize = 80;
/// Length of the triangle count that follows the header, in bytes.
const COUNT_BYTES: usize = 4;
/// Length of one triangle record of a binary STL file, in bytes.
const TRIANGLE_BYTES: usize = 50;
/// Length of one coordinate of a binary STL file, in bytes.
const FLOAT_BYTES: usize = 4;

/// Reasons a file is not a usable STL mesh.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum StlError {
    /// The file is neither a binary STL file of consistent length nor text.
    #[error("the file is neither a binary nor a text STL file")]
    NotStl,
    /// A coordinate of a text STL file is not a number.
    #[error("`{0}` is not a coordinate")]
    BadCoordinate(String),
    /// A text STL file ends in the middle of a triangle.
    #[error("the file ends in the middle of a triangle")]
    Truncated,
    /// The triangles do not form a usable mesh.
    #[error(transparent)]
    Mesh(#[from] MeshError),
}

/// The triangles of a binary STL file, or `None` if the length of `bytes`
/// does not match the number of triangles it declares.
fn parse_binary(bytes: &[u8]) -> Option<Vec<[DVec3; 3]>> {
    let (count, records) = bytes
        .get(HEADER_BYTES..)?
        .split_first_chunk::<COUNT_BYTES>()?;
    let count = usize::try_from(u32::from_le_bytes(*count)).ok()?;
    if records.len() != count.checked_mul(TRIANGLE_BYTES)? {
        return None;
    }
    let triangles = records
        .chunks_exact(TRIANGLE_BYTES)
        .map(|record| {
            let coordinate = |index: usize| {
                let start = index * FLOAT_BYTES;
                let bytes = [
                    record[start],
                    record[start + 1],
                    record[start + 2],
                    record[start + 3],
                ];
                f64::from(f32::from_le_bytes(bytes))
            };
            // The first three numbers are the normal.
            [1, 2, 3].map(|corner| {
                DVec3::new(
                    coordinate(3 * corner),
                    coordinate(3 * corner + 1),
                    coordinate(3 * corner + 2),
                )
            })
        })
        .collect();
    Some(triangles)
}

/// The triangles of a text STL file.
fn parse_text(text: &str) -> Result<Vec<[DVec3; 3]>, StlError> {
    let mut words = text.split_ascii_whitespace();
    let mut corners = Vec::new();
    while let Some(word) = words.next() {
        if !word.eq_ignore_ascii_case("vertex") {
            continue;
        }
        let mut coordinate = || {
            let word = words.next().ok_or(StlError::Truncated)?;
            word.parse::<f64>()
                .map_err(|_| StlError::BadCoordinate(word.to_owned()))
        };
        corners.push(DVec3::new(coordinate()?, coordinate()?, coordinate()?));
    }
    if corners.len() % 3 != 0 {
        return Err(StlError::Truncated);
    }
    Ok(corners
        .chunks_exact(3)
        .map(|corners| [corners[0], corners[1], corners[2]])
        .collect())
}

/// Reads the mesh in the contents of an STL file.
///
/// # Errors
/// Returns [`StlError`] if `bytes` is not an STL file or holds no usable
/// mesh.
pub fn parse(bytes: &[u8]) -> Result<TriangleMesh, StlError> {
    // A binary file may begin with the word that opens a text file, so the
    // length decides.
    let triangles = if let Some(triangles) = parse_binary(bytes) {
        triangles
    } else {
        let text = std::str::from_utf8(bytes).map_err(|_| StlError::NotStl)?;
        if !text.trim_start().starts_with("solid") {
            return Err(StlError::NotStl);
        }
        parse_text(text)?
    };
    Ok(TriangleMesh::new(triangles)?)
}

/// The mesh as the contents of a text STL file whose solid is called
/// `name`. Coordinates are written so that reading them back gives the
/// same numbers.
#[must_use]
pub fn to_text(mesh: &TriangleMesh, name: &str) -> String {
    let mut text = format!("solid {name}\n");
    for triangle in mesh.triangles() {
        let normal = (triangle[1] - triangle[0])
            .cross(triangle[2] - triangle[0])
            .normalize_or_zero();
        // Writing to a string cannot fail.
        let _ = writeln!(
            text,
            "  facet normal {:e} {:e} {:e}\n    outer loop",
            normal.x, normal.y, normal.z
        );
        for corner in triangle {
            let _ = writeln!(
                text,
                "      vertex {:e} {:e} {:e}",
                corner.x, corner.y, corner.z
            );
        }
        text.push_str("    endloop\n  endfacet\n");
    }
    let _ = writeln!(text, "endsolid {name}");
    text
}

/// Reads the mesh in an STL file.
///
/// # Errors
/// Returns [`IoError`] if the file cannot be read or is not a usable STL
/// mesh.
pub fn load(path: &Path) -> Result<TriangleMesh, IoError> {
    let bytes = fs::read(path).map_err(|source| IoError::Read {
        path: path.to_owned(),
        source,
    })?;
    parse(&bytes).map_err(|source| IoError::Stl {
        path: path.to_owned(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use glam::DVec3;
    use rap_geometry::MeshError;

    use rap_geometry::TriangleMesh;

    use super::{StlError, load, parse, to_text};
    use crate::IoError;

    const TEXT: &str = "solid wedge
  facet normal 0 0 -1
    outer loop
      vertex 0 0 0
      vertex 0 1 0
      vertex 1.5e0 0 0
    endloop
  endfacet
  facet normal 0 0 1
    outer loop
      vertex 0 0 2
      vertex 1 0 2
      vertex 0 -1 2
    endloop
  endfacet
endsolid wedge
";

    /// A binary STL file with the given triangles.
    #[expect(
        clippy::cast_possible_truncation,
        reason = "the format stores single precision"
    )]
    fn binary(triangles: &[[DVec3; 3]]) -> Vec<u8> {
        // Binary files written by some programs start like a text file.
        let mut bytes = b"solid binary".to_vec();
        bytes.resize(80, b' ');
        let count = u32::try_from(triangles.len()).expect("a few triangles");
        bytes.extend(count.to_le_bytes());
        for triangle in triangles {
            bytes.extend([0_u8; 12]);
            for corner in triangle {
                for value in corner.to_array() {
                    bytes.extend((value as f32).to_le_bytes());
                }
            }
            bytes.extend([0_u8; 2]);
        }
        bytes
    }

    #[test]
    fn text_file_is_read() {
        let mesh = parse(TEXT.as_bytes()).expect("a valid file");
        assert_eq!(mesh.triangles().len(), 2);
        assert_eq!(mesh.triangles()[0][2], DVec3::new(1.5, 0.0, 0.0));
        assert_eq!(mesh.triangles()[1][2], DVec3::new(0.0, -1.0, 2.0));
    }

    #[test]
    fn binary_file_is_read() {
        let triangles = [
            [DVec3::ZERO, DVec3::X, DVec3::Y],
            [DVec3::Z, DVec3::new(0.5, -2.0, 4.0), DVec3::ONE],
        ];
        let mesh = parse(&binary(&triangles)).expect("a valid file");
        assert_eq!(mesh.triangles(), triangles);
    }

    #[test]
    fn written_text_reads_back_unchanged() {
        let mesh = TriangleMesh::cuboid(DVec3::new(-0.1, 1.0 / 3.0, 2.5e-7), DVec3::splat(12.0));
        let text = to_text(&mesh, "box");
        assert!(text.starts_with("solid box\n") && text.ends_with("endsolid box\n"));
        assert_eq!(parse(text.as_bytes()), Ok(mesh));
    }

    #[test]
    fn broken_files_are_reported() {
        assert_eq!(parse(b"not an stl file"), Err(StlError::NotStl));
        assert_eq!(parse(&[0xff; 100]), Err(StlError::NotStl));
        assert_eq!(
            parse(b"solid empty\nendsolid empty\n"),
            Err(StlError::Mesh(MeshError::Empty))
        );
        assert_eq!(
            parse(b"solid bad\nvertex 0 0 zero\n"),
            Err(StlError::BadCoordinate("zero".to_owned()))
        );
        assert_eq!(
            parse(b"solid short\nvertex 0 0 0\nvertex 1 0 0\n"),
            Err(StlError::Truncated)
        );
        let mut cut = binary(&[[DVec3::ZERO, DVec3::X, DVec3::Y]]);
        cut.pop();
        assert_eq!(parse(&cut), Err(StlError::NotStl));
    }

    #[test]
    fn file_on_disk_is_read_and_a_missing_one_is_named() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let path = directory.path().join("wedge.stl");
        std::fs::write(&path, TEXT).expect("the file is written");
        assert_eq!(load(&path).expect("a valid file").triangles().len(), 2);

        let missing = directory.path().join("missing.stl");
        let error = load(&missing).unwrap_err();
        assert!(matches!(error, IoError::Read { .. }));
        std::fs::write(&missing, "nothing").expect("the file is written");
        let error = load(&missing).unwrap_err();
        assert!(error.to_string().contains("missing.stl"), "{error}");
        assert!(matches!(error, IoError::Stl { .. }));
    }
}
