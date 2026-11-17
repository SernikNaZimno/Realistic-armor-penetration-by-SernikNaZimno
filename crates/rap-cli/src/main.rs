//! Headless command-line front end.

mod pictures;

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use rap_app::{ModelKind, Simulation, load_mesh, presets};
use rap_domain::{APP_NAME, Backend};
use rap_io::{ScenarioFile, vtk};
use rap_materials::{EquationOfState, library};
use serde_json::json;

use crate::pictures::{Colouring, Pair, PictureOptions};

/// Command-line arguments.
#[derive(Debug, Parser)]
#[command(name = "rap", version, about = APP_NAME)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Write an example scenario file to edit.
    Preset {
        /// Kind of projectile in the example.
        #[arg(value_enum)]
        projectile: PresetKind,
        /// File to write; standard output if omitted.
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// List the built-in materials.
    Materials,
    /// Simulate a scenario file.
    Run {
        /// Scenario file to simulate.
        scenario: PathBuf,
        /// Directory for the results; `<scenario name>-results` next to the
        /// scenario file if omitted.
        #[arg(short, long)]
        output_dir: Option<PathBuf>,
        /// Do not write particle snapshots, only the summary.
        #[arg(long)]
        no_snapshots: bool,
        /// Run on the graphics card, whatever the scenario file says.
        #[arg(long)]
        gpu: bool,
        /// Write a three-dimensional picture of the model at every snapshot,
        /// for assembling into a film.
        #[arg(long)]
        pictures: bool,
        /// What the colours of the pictures show.
        #[arg(long, value_enum, default_value = "material")]
        colour_by: Colouring,
        /// Viewing direction of the pictures: rotation about the vertical
        /// axis and elevation, in degrees, such as `0,0` for a side view.
        #[arg(long, value_name = "YAW,PITCH", allow_hyphen_values = true)]
        view: Option<Pair<f32>>,
        /// Size of the pictures, in pixels.
        #[arg(long, value_name = "WIDTHxHEIGHT", default_value = "1280x720")]
        picture_size: Pair<usize>,
        /// Draw the whole model in the pictures instead of cutting it along
        /// the shot line.
        #[arg(long)]
        whole: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum PresetKind {
    /// Full-caliber capped shot against a steel plate.
    Apbc,
    /// Long-rod penetrator against thick steel.
    Apfsds,
    /// Full-caliber shell with a bursting charge against thick steel at a
    /// compound angle.
    Aphe,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    execute(cli.command, &mut io::stdout().lock())
}

/// Runs one command, writing its report to `output`.
fn execute(command: Command, output: &mut impl Write) -> Result<()> {
    match command {
        Command::Preset {
            projectile,
            output: path,
        } => preset(projectile, path.as_deref(), output),
        Command::Materials => materials(output),
        Command::Run {
            scenario,
            output_dir,
            no_snapshots,
            gpu,
            pictures,
            colour_by,
            view,
            picture_size,
            whole,
        } => {
            let directory = output_dir.unwrap_or_else(|| default_output_directory(&scenario));
            let options = RunOptions {
                snapshots: !no_snapshots,
                force_gpu: gpu,
                pictures: pictures
                    .then(|| PictureOptions::new(colour_by, view, picture_size, whole))
                    .transpose()?,
            };
            run(&scenario, &directory, options, output)
        }
    }
}

fn preset(kind: PresetKind, path: Option<&Path>, output: &mut impl Write) -> Result<()> {
    let file = match kind {
        PresetKind::Apbc => presets::apbc_against_steel(),
        PresetKind::Apfsds => presets::apfsds_against_steel(),
        PresetKind::Aphe => presets::aphe_against_cast_armor(),
    };
    match path {
        Some(path) => {
            file.save(path)?;
            writeln!(output, "Wrote {}", path.display())?;
        }
        None => writeln!(output, "{}", file.to_json()?)?,
    }
    Ok(())
}

fn materials(output: &mut impl Write) -> Result<()> {
    writeln!(
        output,
        "{:<24} {:<24} {:>16}",
        "identifier", "name", "density [kg/m³]"
    )?;
    for material in library::builtin() {
        writeln!(
            output,
            "{:<24} {:<24} {:>16.0}",
            material.id.as_str(),
            material.name,
            material.eos.reference_density()
        )?;
    }
    Ok(())
}

/// `<scenario name>-results` next to the scenario file.
fn default_output_directory(scenario: &Path) -> PathBuf {
    let mut directory = scenario
        .file_stem()
        .map_or_else(|| "scenario".into(), std::ffi::OsStr::to_os_string);
    directory.push("-results");
    scenario.with_file_name(directory)
}

/// How a scenario is run.
#[derive(Debug, Clone, Copy)]
struct RunOptions {
    /// Write particle snapshots.
    snapshots: bool,
    /// Use the GPU solver regardless of the scenario's settings.
    force_gpu: bool,
    /// Write pictures, drawn like this.
    pictures: Option<PictureOptions>,
}

/// Prints what is about to be simulated and the heading of the progress
/// table.
fn announce(
    output: &mut impl Write,
    simulation: &Simulation,
    symmetry: &str,
    pictures: bool,
) -> Result<()> {
    let layout = simulation.layout();
    writeln!(output, "{APP_NAME}")?;
    writeln!(output, "Solver: {}", simulation.solver_name())?;
    writeln!(
        output,
        "Model: {symmetry}, cell {:.3} mm, {} particles, {:.1} µs to simulate",
        layout.cell_size * 1.0e3,
        simulation.particle_count(),
        layout.end_time * 1.0e6
    )?;
    if layout.kind == ModelKind::PlaneStrain {
        writeln!(
            output,
            "Note: an oblique impact is modelled in plane strain; the projectile acts as a \
             slab, not a body of revolution, so depths are indicative only."
        )?;
    }
    if pictures && layout.kind == ModelKind::PlaneStrain {
        writeln!(
            output,
            "Note: a model in plane strain has no third dimension to draw; no pictures will \
             be written."
        )?;
    }
    writeln!(
        output,
        "{:>10} {:>12} {:>14} {:>10}",
        "time [µs]", "depth [mm]", "residual [m/s]", "failed"
    )?;
    Ok(())
}

/// The outcome of a run in one sentence.
fn verdict(last: &rap_analysis::ImpactSummary) -> String {
    if last.perforated {
        format!(
            "Perforated: {:.0} % of the projectile mass left the armor at {:.0} m/s.",
            last.residual_mass_fraction * 100.0,
            last.residual_speed
        )
    } else {
        format!(
            "Not perforated: the projectile reached {:.1} mm of {:.1} mm along the shot line.",
            last.penetration_depth * 1.0e3,
            last.armor_path_length * 1.0e3
        )
    }
}

fn run(
    scenario: &Path,
    directory: &Path,
    options: RunOptions,
    output: &mut impl Write,
) -> Result<()> {
    let mut file = ScenarioFile::load(scenario)?;
    if options.force_gpu {
        file.settings.backend = Backend::Gpu;
    }
    // A mesh file named by the scenario is looked for next to it.
    let folder = scenario.parent().unwrap_or_else(|| Path::new(""));
    let mesh = load_mesh(&file.scenario, folder)
        .with_context(|| format!("cannot simulate {}", scenario.display()))?;
    let mut simulation = Simulation::with_mesh(
        &file.scenario,
        &file.settings,
        &file.materials,
        mesh.as_ref(),
    )
    .with_context(|| format!("cannot simulate {}", scenario.display()))?;
    fs::create_dir_all(directory)
        .with_context(|| format!("cannot create {}", directory.display()))?;

    let layout = *simulation.layout();
    let symmetry = match layout.kind {
        ModelKind::Axisymmetric => "axisymmetric",
        ModelKind::PlaneStrain => "plane strain",
        ModelKind::Solid if mesh.is_some() => "three-dimensional (whole model)",
        ModelKind::Solid => "three-dimensional (half model)",
    };
    announce(output, &simulation, symmetry, options.pictures.is_some())?;

    let mut history = Vec::new();
    let mut frames = Vec::new();
    let mut framing = pictures::Framing::default();
    let count = file.settings.snapshots;
    for index in 0..=count {
        let time = layout.end_time * f64::from(index) / f64::from(count);
        simulation.advance_to(time)?;
        let summary = simulation.summary();
        writeln!(
            output,
            "{:>10.1} {:>12.2} {:>14.1} {:>9.1}%",
            summary.time * 1.0e6,
            summary.penetration_depth * 1.0e3,
            summary.residual_speed,
            summary.armor_failed_fraction * 100.0
        )?;
        if options.snapshots {
            let path = directory.join(format!("particles_{index:04}.vtk"));
            match simulation.solid_particles() {
                Some(particles) => vtk::save_solid_particles(&path, &particles)?,
                None => vtk::save_particles(&path, &simulation.particles())?,
            }
        }
        if let Some(pictures) = &options.pictures {
            let path = directory.join(format!("picture_{index:04}.png"));
            if let Some(range) = pictures::save(&simulation, pictures, &mut framing, &path)? {
                frames.push(json!({
                    "file": path.file_name().map(|name| name.to_string_lossy()),
                    "time": summary.time,
                    "lowest": range.0,
                    "highest": range.1,
                }));
            }
        }
        history.push(summary);
    }

    let last = simulation.summary();
    let verdict = verdict(&last);
    writeln!(output, "{verdict}")?;

    let report = json!({
        "scenario": scenario.display().to_string(),
        "solver": simulation.solver_name(),
        "model": {
            "symmetry": symmetry,
            "cell_size": layout.cell_size,
            "particles": simulation.particle_count(),
            "end_time": layout.end_time,
        },
        "result": last,
        "history": history,
        "pictures": options.pictures.map(|pictures| json!({
            "colours": pictures.field.label(),
            "frames": frames,
        })),
    });
    let summary_path = directory.join("summary.json");
    fs::write(&summary_path, serde_json::to_string_pretty(&report)?)
        .with_context(|| format!("cannot write {}", summary_path.display()))?;
    writeln!(output, "Results written to {}", directory.display())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use clap::{CommandFactory, Parser};
    use rap_io::ScenarioFile;

    use super::{Cli, Colouring, Command, Pair, PresetKind, default_output_directory, execute};

    fn report(command: Command) -> String {
        let mut output = Vec::new();
        execute(command, &mut output).expect("the command succeeds");
        String::from_utf8(output).expect("text output")
    }

    #[test]
    fn command_line_definition_is_consistent() {
        Cli::command().debug_assert();
    }

    #[test]
    fn arguments_are_parsed() {
        let cli = Cli::parse_from([
            "rap",
            "run",
            "shot.json",
            "--no-snapshots",
            "-o",
            "out",
            "--pictures",
            "--view",
            "-20,35",
            "--colour-by",
            "speed",
        ]);
        let Command::Run {
            scenario,
            output_dir,
            no_snapshots,
            gpu,
            pictures,
            colour_by,
            view,
            picture_size,
            whole,
        } = cli.command
        else {
            panic!("expected the run command");
        };
        assert_eq!(scenario, Path::new("shot.json"));
        assert_eq!(output_dir.as_deref(), Some(Path::new("out")));
        assert!(no_snapshots);
        assert!(pictures && !whole);
        assert_eq!(colour_by, Colouring::Speed);
        assert_eq!(view, Some(Pair(-20.0, 35.0)));
        assert_eq!(picture_size, Pair(1280, 720));
        assert!(!gpu);
        assert!(Cli::try_parse_from(["rap", "preset", "heat"]).is_err());
    }

    #[test]
    fn results_go_next_to_the_scenario_by_default() {
        let directory = default_output_directory(Path::new("shots/thick plate.json"));
        assert_eq!(directory, Path::new("shots/thick plate-results"));
    }

    #[test]
    fn presets_are_valid_scenario_files() {
        for kind in [PresetKind::Apbc, PresetKind::Apfsds, PresetKind::Aphe] {
            let text = report(Command::Preset {
                projectile: kind,
                output: None,
            });
            let file = ScenarioFile::from_json(&text).expect("a valid scenario file");
            assert_eq!(file.scenario.validate(), Ok(()));
        }
    }

    #[test]
    fn materials_are_listed() {
        let text = report(Command::Materials);
        assert!(
            text.contains("steel-4340") && text.contains("tungsten-heavy-alloy"),
            "{text}"
        );
    }

    #[test]
    fn run_writes_summary_and_snapshots() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let scenario = directory.path().join("shot.json");
        let mut file = rap_app::presets::apfsds_against_steel();
        // Coarse and short: this checks the plumbing, not the physics.
        file.settings.cells_per_radius = 2.0;
        file.settings.armor_extent_radii = 4.0;
        file.settings.end_time = Some(2.0e-5);
        file.settings.snapshots = 2;
        file.save(&scenario).expect("the scenario is written");

        let text = report(Command::Run {
            scenario: scenario.clone(),
            output_dir: None,
            no_snapshots: false,
            gpu: false,
            pictures: false,
            colour_by: Colouring::Material,
            view: None,
            picture_size: Pair(64, 48),
            whole: false,
        });
        assert!(text.contains("axisymmetric"), "{text}");
        assert!(text.contains("Solver: CPU"), "{text}");
        assert!(text.contains("Not perforated"), "{text}");

        let results = directory.path().join("shot-results");
        for name in [
            "particles_0000.vtk",
            "particles_0001.vtk",
            "particles_0002.vtk",
        ] {
            assert!(results.join(name).is_file(), "{name} is missing");
        }
        let summary = std::fs::read_to_string(results.join("summary.json")).expect("a summary");
        let summary: serde_json::Value = serde_json::from_str(&summary).expect("valid JSON");
        assert_eq!(summary["history"].as_array().map(Vec::len), Some(3));
        assert_eq!(summary["result"]["perforated"], false);
        assert!(
            summary["result"]["penetration_depth"]
                .as_f64()
                .expect("a number")
                > 0.0
        );
    }

    #[test]
    fn pictures_are_written_for_a_solid_model() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let scenario = directory.path().join("oblique.json");
        let mut file = rap_app::presets::apfsds_against_steel();
        file.settings.cells_per_radius = 2.0;
        file.settings.armor_extent_radii = 3.0;
        file.settings.end_time = Some(2.0e-6);
        file.settings.snapshots = 1;
        let armor = file.scenario.armor.plates_mut().expect("plates");
        armor.obliquity = 0.5;
        armor.layers[0].thickness = 0.02;
        file.save(&scenario).expect("the scenario is written");

        let text = report(Command::Run {
            scenario: scenario.clone(),
            output_dir: None,
            no_snapshots: true,
            gpu: false,
            pictures: true,
            colour_by: Colouring::Material,
            view: None,
            picture_size: Pair(64, 48),
            whole: false,
        });
        assert!(text.contains("three-dimensional"), "{text}");
        let results = directory.path().join("oblique-results");
        for name in ["picture_0000.png", "picture_0001.png"] {
            let bytes = std::fs::read(results.join(name)).expect("a picture");
            assert_eq!(&bytes[1..4], b"PNG", "{name}");
        }
        assert!(!results.join("particles_0000.vtk").exists());
        let summary = std::fs::read_to_string(results.join("summary.json")).expect("a summary");
        let summary: serde_json::Value = serde_json::from_str(&summary).expect("valid JSON");
        assert_eq!(summary["pictures"]["colours"], "Material");
        assert_eq!(summary["pictures"]["frames"][1]["file"], "picture_0001.png");
    }

    #[test]
    fn missing_scenario_is_reported() {
        let mut output = Vec::new();
        let command = Command::Run {
            scenario: "does-not-exist.json".into(),
            output_dir: None,
            no_snapshots: true,
            gpu: false,
            pictures: false,
            colour_by: Colouring::Material,
            view: None,
            picture_size: Pair(64, 48),
            whole: false,
        };
        let error = execute(command, &mut output).unwrap_err();
        assert!(error.to_string().contains("does-not-exist.json"));
    }
}
