//! Export of particle snapshots in the legacy VTK format, readable by
//! `ParaView` and `VisIt`.

use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::Path;

use rap_solver::Particle;

use crate::IoError;

/// Writes `particles` as a VTK point cloud with their state as point data.
///
/// # Errors
/// Returns any error of the underlying writer.
pub fn write_particles(mut writer: impl Write, particles: &[Particle]) -> io::Result<()> {
    let count = particles.len();
    writeln!(writer, "# vtk DataFile Version 3.0")?;
    writeln!(writer, "Realistic armor penetration particles")?;
    writeln!(writer, "ASCII")?;
    writeln!(writer, "DATASET POLYDATA")?;
    writeln!(writer, "POINTS {count} double")?;
    for particle in particles {
        writeln!(
            writer,
            "{:e} {:e} 0",
            particle.position.x, particle.position.y
        )?;
    }
    writeln!(writer, "VERTICES {count} {}", 2 * count)?;
    for index in 0..count {
        writeln!(writer, "1 {index}")?;
    }
    writeln!(writer, "POINT_DATA {count}")?;
    integers(&mut writer, "body", particles, |particle| particle.body)?;
    integers(&mut writer, "material", particles, |particle| {
        particle.material
    })?;
    reals(&mut writer, "damage", particles, |particle| particle.damage)?;
    reals(&mut writer, "plastic_strain", particles, |particle| {
        particle.plastic_strain
    })?;
    reals(&mut writer, "pressure", particles, |particle| {
        particle.pressure
    })?;
    reals(&mut writer, "equivalent_stress", particles, |particle| {
        particle.deviator.equivalent_stress()
    })?;
    reals(&mut writer, "temperature", particles, |particle| {
        particle.temperature
    })?;
    reals(&mut writer, "density", particles, Particle::density)?;
    writeln!(writer, "VECTORS velocity double")?;
    for particle in particles {
        writeln!(
            writer,
            "{:e} {:e} 0",
            particle.velocity.x, particle.velocity.y
        )?;
    }
    writer.flush()
}

/// Writes `particles` to a VTK file, replacing any existing file.
///
/// # Errors
/// Returns [`IoError::Write`] if the file cannot be written.
pub fn save_particles(path: &Path, particles: &[Particle]) -> Result<(), IoError> {
    File::create(path)
        .and_then(|file| write_particles(BufWriter::new(file), particles))
        .map_err(|source| IoError::Write {
            path: path.to_owned(),
            source,
        })
}

fn integers(
    writer: &mut impl Write,
    name: &str,
    particles: &[Particle],
    value: impl Fn(&Particle) -> usize,
) -> io::Result<()> {
    writeln!(writer, "SCALARS {name} int 1")?;
    writeln!(writer, "LOOKUP_TABLE default")?;
    particles
        .iter()
        .try_for_each(|particle| writeln!(writer, "{}", value(particle)))
}

fn reals(
    writer: &mut impl Write,
    name: &str,
    particles: &[Particle],
    value: impl Fn(&Particle) -> f64,
) -> io::Result<()> {
    writeln!(writer, "SCALARS {name} double 1")?;
    writeln!(writer, "LOOKUP_TABLE default")?;
    particles
        .iter()
        .try_for_each(|particle| writeln!(writer, "{:e}", value(particle)))
}

#[cfg(test)]
mod tests {
    use glam::{DMat2, DVec2};
    use rap_solver::{Deviator, Particle};

    use super::{save_particles, write_particles};

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
        assert_eq!(lines[5], "5e-1 2.5e-1 0");
        assert_eq!(lines[lines.len() - 1], "1.5e3 -2e0 0");
        let body = lines
            .iter()
            .position(|line| *line == "SCALARS body int 1")
            .expect("body data");
        assert_eq!(lines[body + 2..body + 5], ["0", "2", "2"]);
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
