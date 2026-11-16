//! Export of particle snapshots in the legacy VTK format, readable by
//! `ParaView` and `VisIt`.

use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::Path;

use rap_solver::{Particle, Particle3};

use crate::IoError;

/// What is written for one particle, whatever the dimension of its model.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Point {
    position: [f64; 3],
    velocity: [f64; 3],
    body: usize,
    material: usize,
    damage: f64,
    plastic_strain: f64,
    pressure: f64,
    equivalent_stress: f64,
    temperature: f64,
    density: f64,
}

impl From<&Particle> for Point {
    fn from(particle: &Particle) -> Self {
        Self {
            position: [particle.position.x, particle.position.y, 0.0],
            velocity: [particle.velocity.x, particle.velocity.y, 0.0],
            body: particle.body,
            material: particle.material,
            damage: particle.damage,
            plastic_strain: particle.plastic_strain,
            pressure: particle.pressure,
            equivalent_stress: particle.deviator.equivalent_stress(),
            temperature: particle.temperature,
            density: particle.density(),
        }
    }
}

impl From<&Particle3> for Point {
    fn from(particle: &Particle3) -> Self {
        Self {
            position: particle.position.to_array(),
            velocity: particle.velocity.to_array(),
            body: particle.body,
            material: particle.material,
            damage: particle.damage,
            plastic_strain: particle.plastic_strain,
            pressure: particle.pressure,
            equivalent_stress: particle.equivalent_stress(),
            temperature: particle.temperature,
            density: particle.density(),
        }
    }
}

fn vector(writer: &mut impl Write, value: [f64; 3]) -> io::Result<()> {
    writeln!(writer, "{:e} {:e} {:e}", value[0], value[1], value[2])
}

fn integers(
    writer: &mut impl Write,
    name: &str,
    points: &[Point],
    value: impl Fn(&Point) -> usize,
) -> io::Result<()> {
    writeln!(writer, "SCALARS {name} int 1")?;
    writeln!(writer, "LOOKUP_TABLE default")?;
    points
        .iter()
        .try_for_each(|point| writeln!(writer, "{}", value(point)))
}

fn reals(
    writer: &mut impl Write,
    name: &str,
    points: &[Point],
    value: impl Fn(&Point) -> f64,
) -> io::Result<()> {
    writeln!(writer, "SCALARS {name} double 1")?;
    writeln!(writer, "LOOKUP_TABLE default")?;
    points
        .iter()
        .try_for_each(|point| writeln!(writer, "{:e}", value(point)))
}

fn write_points(mut writer: impl Write, points: &[Point]) -> io::Result<()> {
    let count = points.len();
    writeln!(writer, "# vtk DataFile Version 3.0")?;
    writeln!(writer, "Realistic armor penetration particles")?;
    writeln!(writer, "ASCII")?;
    writeln!(writer, "DATASET POLYDATA")?;
    writeln!(writer, "POINTS {count} double")?;
    for point in points {
        vector(&mut writer, point.position)?;
    }
    writeln!(writer, "VERTICES {count} {}", 2 * count)?;
    for index in 0..count {
        writeln!(writer, "1 {index}")?;
    }
    writeln!(writer, "POINT_DATA {count}")?;
    integers(&mut writer, "body", points, |point| point.body)?;
    integers(&mut writer, "material", points, |point| point.material)?;
    reals(&mut writer, "damage", points, |point| point.damage)?;
    reals(&mut writer, "plastic_strain", points, |point| {
        point.plastic_strain
    })?;
    reals(&mut writer, "pressure", points, |point| point.pressure)?;
    reals(&mut writer, "equivalent_stress", points, |point| {
        point.equivalent_stress
    })?;
    reals(&mut writer, "temperature", points, |point| {
        point.temperature
    })?;
    reals(&mut writer, "density", points, |point| point.density)?;
    writeln!(writer, "VECTORS velocity double")?;
    for point in points {
        vector(&mut writer, point.velocity)?;
    }
    writer.flush()
}

fn save_points(path: &Path, points: &[Point]) -> Result<(), IoError> {
    File::create(path)
        .and_then(|file| write_points(BufWriter::new(file), points))
        .map_err(|source| IoError::Write {
            path: path.to_owned(),
            source,
        })
}

/// Writes two-dimensional `particles` as a VTK point cloud in the plane
/// `z = 0`, with their state as point data.
///
/// # Errors
/// Returns any error of the underlying writer.
pub fn write_particles(writer: impl Write, particles: &[Particle]) -> io::Result<()> {
    write_points(
        writer,
        &particles.iter().map(Point::from).collect::<Vec<_>>(),
    )
}

/// Writes two-dimensional `particles` to a VTK file, replacing any existing
/// file.
///
/// # Errors
/// Returns [`IoError::Write`] if the file cannot be written.
pub fn save_particles(path: &Path, particles: &[Particle]) -> Result<(), IoError> {
    save_points(path, &particles.iter().map(Point::from).collect::<Vec<_>>())
}

/// Writes three-dimensional `particles` as a VTK point cloud with their
/// state as point data.
///
/// # Errors
/// Returns any error of the underlying writer.
pub fn write_solid_particles(writer: impl Write, particles: &[Particle3]) -> io::Result<()> {
    write_points(
        writer,
        &particles.iter().map(Point::from).collect::<Vec<_>>(),
    )
}

/// Writes three-dimensional `particles` to a VTK file, replacing any
/// existing file.
///
/// # Errors
/// Returns [`IoError::Write`] if the file cannot be written.
pub fn save_solid_particles(path: &Path, particles: &[Particle3]) -> Result<(), IoError> {
    save_points(path, &particles.iter().map(Point::from).collect::<Vec<_>>())
}

#[cfg(test)]
mod tests {
    use glam::{DMat2, DMat3, DVec2, DVec3};
    use rap_solver::{Deviator, Particle, Particle3};

    use super::{save_particles, write_particles, write_solid_particles};

    fn particle(x: f64, body: usize) -> Particle {
        Particle {
            position: DVec2::new(x, 0.25),
            velocity: DVec2::new(1500.0, -2.0),
            velocity_gradient: DMat2::ZERO,
            mass: 2.0e-3,
            volume: 2.5e-7,
            body,
            material: 1,
            deviator: Deviator {
                xx: 3.0e8,
                yy: -1.5e8,
                xy: 0.0,
            },
            pressure: 1.0e9,
            viscous_pressure: 0.0,
            specific_energy: 0.0,
            plastic_strain: 0.125,
            plastic_strain_rate: 0.0,
            temperature: 294.0,
            damage: 0.5,
            sound_speed: 5800.0,
        }
    }

    fn sections(text: &str) -> Vec<&str> {
        text.lines()
            .filter(|line| line.starts_with(|first: char| first.is_ascii_uppercase()))
            .collect()
    }

    #[test]
    fn output_has_one_entry_per_particle_in_every_section() {
        let particles = [particle(0.5, 0), particle(1.5, 2), particle(2.5, 2)];
        let mut buffer = Vec::new();
        write_particles(&mut buffer, &particles).expect("writing to memory succeeds");
        let text = String::from_utf8(buffer).expect("ASCII output");

        assert!(text.starts_with("# vtk DataFile Version 3.0\n"));
        let sections = sections(&text);
        assert!(sections.contains(&"POINTS 3 double"));
        assert!(sections.contains(&"VERTICES 3 6"));
        assert!(sections.contains(&"POINT_DATA 3"));
        assert!(sections.contains(&"VECTORS velocity double"));
        let scalars = sections
            .iter()
            .filter(|line| line.starts_with("SCALARS"))
            .count();
        assert_eq!(scalars, 8);
        // Header (4) + points (1 + 3) + vertices (1 + 3) + point data (1)
        // + 8 scalars (2 + 3 each) + velocity (1 + 3).
        assert_eq!(text.lines().count(), 4 + 4 + 4 + 1 + 8 * 5 + 4);

        let lines: Vec<_> = text.lines().collect();
        assert_eq!(lines[5], "5e-1 2.5e-1 0e0");
        assert_eq!(lines[lines.len() - 1], "1.5e3 -2e0 0e0");
        let body = lines
            .iter()
            .position(|line| *line == "SCALARS body int 1")
            .expect("body data");
        assert_eq!(lines[body + 2..body + 5], ["0", "2", "2"]);
    }

    #[test]
    fn solid_particles_keep_their_third_coordinate() {
        let solid = Particle3 {
            position: DVec3::new(0.5, 0.25, 0.125),
            velocity: DVec3::new(1.0, 2.0, 4.0),
            velocity_gradient: DMat3::ZERO,
            mass: 2.0e-3,
            volume: 2.5e-7,
            body: 1,
            material: 0,
            deviator: DMat3::from_diagonal(DVec3::new(2.0e8, -1.0e8, -1.0e8)),
            pressure: 0.0,
            viscous_pressure: 0.0,
            specific_energy: 0.0,
            plastic_strain: 0.0,
            plastic_strain_rate: 0.0,
            temperature: 294.0,
            damage: 0.0,
            sound_speed: 5800.0,
        };
        let mut buffer = Vec::new();
        write_solid_particles(&mut buffer, &[solid]).expect("writing to memory succeeds");
        let text = String::from_utf8(buffer).expect("ASCII output");
        let lines: Vec<_> = text.lines().collect();
        assert_eq!(lines[5], "5e-1 2.5e-1 1.25e-1");
        assert_eq!(lines[lines.len() - 1], "1e0 2e0 4e0");
        let stress = lines
            .iter()
            .position(|line| *line == "SCALARS equivalent_stress double 1")
            .expect("stress data");
        assert_eq!(lines[stress + 2], "3e8");
    }

    #[test]
    fn file_is_written_and_unwritable_paths_are_reported() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let path = directory.path().join("particles.vtk");
        save_particles(&path, &[particle(0.0, 0)]).expect("the file is written");
        let text = std::fs::read_to_string(&path).expect("the file exists");
        assert!(text.contains("POINTS 1 double"));

        let missing = directory
            .path()
            .join("no-such-directory")
            .join("particles.vtk");
        let error = save_particles(&missing, &[]).unwrap_err();
        assert!(error.to_string().contains("no-such-directory"));
    }
}
