//! Headless command-line front end.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use rap_app::{Simulation, presets};
use rap_domain::APP_NAME;
use rap_io::{ScenarioFile, vtk};
use rap_materials::{EquationOfState, library};
use rap_solver::Symmetry;
use serde_json::json;

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
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum PresetKind {
    /// Full-caliber capped shot against a steel plate.
    Apbc,
    /// Long-rod penetrator against thick steel.
    Apfsds,
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
        } => {
            let directory = output_dir.unwrap_or_else(|| default_output_directory(&scenario));
            run(&scenario, &directory, !no_snapshots, output)
        }
    }
}

fn preset(kind: PresetKind, path: Option<&Path>, output: &mut impl Write) -> Result<()> {
    let file = match kind {
        PresetKind::Apbc => presets::apbc_against_steel(),
        PresetKind::Apfsds => presets::apfsds_against_steel(),
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

fn run(scenario: &Path, directory: &Path, snapshots: bool, output: &mut impl Write) -> Result<()> {
    let file = ScenarioFile::load(scenario)?;
    let mut simulation = Simulation::new(&file.scenario, &file.settings, &file.materials)
        .with_context(|| format!("cannot simulate {}", scenario.display()))?;
    fs::create_dir_all(directory)
        .with_context(|| format!("cannot create {}", directory.display()))?;

    let layout = *simulation.layout();
    let symmetry = match layout.symmetry {
        Symmetry::Axisymmetric => "axisymmetric",
        Symmetry::Planar => "plane strain",
    };
    writeln!(output, "{APP_NAME}")?;
    writeln!(
        output,
        "Model: {symmetry}, cell {:.3} mm, {} particles, {:.1} µs to simulate",
        layout.cell_size * 1.0e3,
        simulation.particles().len(),
        layout.end_time * 1.0e6
    )?;
    if layout.symmetry == Symmetry::Planar {
        writeln!(
            output,
            "Note: an oblique impact is modelled in plane strain; the projectile acts as a \
             slab, not a body of revolution, so depths are indicative only."
        )?;
    }
    writeln!(
        output,
        "{:>10} {:>12} {:>14} {:>10}",
        "time [µs]", "depth [mm]", "residual [m/s]", "failed"
    )?;

    let mut history = Vec::new();
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
        if snapshots {
            let path = directory.join(format!("particles_{index:04}.vtk"));
            vtk::save_particles(&path, &simulation.particles())?;
        }
        history.push(summary);
    }

    let last = simulation.summary();
    let verdict = if last.perforated {
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
    };
    writeln!(output, "{verdict}")?;

    let report = json!({
        "scenario": scenario.display().to_string(),
        "model": {
            "symmetry": symmetry,
            "cell_size": layout.cell_size,
            "particles": simulation.particles().len(),
            "end_time": layout.end_time,
        },
        "result": last,
        "history": history,
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

    use super::{Cli, Command, PresetKind, default_output_directory, execute};

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
        let cli = Cli::parse_from(["rap", "run", "shot.json", "--no-snapshots", "-o", "out"]);
        let Command::Run {
            scenario,
            output_dir,
            no_snapshots,
        } = cli.command
        else {
            panic!("expected the run command");
        };
        assert_eq!(scenario, Path::new("shot.json"));
        assert_eq!(output_dir.as_deref(), Some(Path::new("out")));
        assert!(no_snapshots);
        assert!(Cli::try_parse_from(["rap", "preset", "heat"]).is_err());
    }

    #[test]
    fn results_go_next_to_the_scenario_by_default() {
        let directory = default_output_directory(Path::new("shots/thick plate.json"));
        assert_eq!(directory, Path::new("shots/thick plate-results"));
    }

    #[test]
    fn presets_are_valid_scenario_files() {
        for kind in [PresetKind::Apbc, PresetKind::Apfsds] {
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
        });
        assert!(text.contains("axisymmetric"), "{text}");
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
    fn missing_scenario_is_reported() {
        let mut output = Vec::new();
        let command = Command::Run {
            scenario: "does-not-exist.json".into(),
            output_dir: None,
            no_snapshots: true,
        };
        let error = execute(command, &mut output).unwrap_err();
        assert!(error.to_string().contains("does-not-exist.json"));
    }
}
