//! The application window: toolbar, editors, particle view and results.

use std::path::{Path, PathBuf};

use eframe::egui;
use glam::DVec2;
use rap_analysis::{ImpactSummary, bounding_box};
use rap_app::{ModelKind, Simulation, presets};
use rap_domain::{Backend, SimulationSettings};
use rap_geometry::TriangleMesh;
use rap_io::{IoError, ScenarioFile, stl};
use rap_solver::{Particle, Symmetry};

use crate::runner::{Runner, Snapshot, Update};
use crate::view::{Camera, Field, Scene};
use crate::view3d::{Camera3, Cloud, Surface};
use crate::{editors, materials, view, view3d, widgets};

/// Number of progress reports over a run; also the number of animation
/// frames at most.
const SLICES: u32 = 240;
/// Margin around the model in the view, in cells.
const VIEW_MARGIN_CELLS: f64 = 4.0;

/// The scenario as assembled for the solver, before any time has passed.
struct Preview {
    simulation: Simulation,
    /// All particles of a solid model, for the three-dimensional view.
    cloud: Option<Cloud>,
    bounds: (DVec2, DVec2),
    particle_size: f64,
}

impl Preview {
    fn new(file: &ScenarioFile, mesh: Option<&TriangleMesh>) -> Result<Self, String> {
        // The preview only shows the initial state, so it never needs the
        // solver chosen for the run.
        let settings = SimulationSettings {
            backend: Backend::Cpu,
            ..file.settings.clone()
        };
        let simulation = Simulation::with_mesh(&file.scenario, &settings, &file.materials, mesh)
            .map_err(|error| error.to_string())?;
        let cell_size = simulation.layout().cell_size;
        let margin = DVec2::splat(VIEW_MARGIN_CELLS * cell_size);
        let (lower, upper) =
            bounding_box(&simulation.particles()).unwrap_or((DVec2::ZERO, DVec2::ONE));
        Ok(Self {
            cloud: Snapshot::of(&simulation).cloud,
            simulation,
            bounds: (lower - margin, upper + margin),
            particle_size: cell_size / f64::from(file.settings.particles_per_cell_edge),
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
enum RunState {
    Running,
    Paused,
    Finished,
    Failed(String),
}

/// A simulation that has been started.
struct Run {
    runner: Runner,
    particles: Vec<Particle>,
    cloud: Option<Cloud>,
    history: Vec<ImpactSummary>,
    state: RunState,
}

/// State of the application window.
pub struct App {
    file: ScenarioFile,
    path: Option<PathBuf>,
    /// The mesh the armor refers to and the file it was read from.
    mesh: Option<(PathBuf, TriangleMesh)>,
    preview: Result<Preview, String>,
    run: Option<Run>,
    field: Field,
    camera: Camera,
    camera3: Camera3,
    /// Whether a solid model is shown in three dimensions rather than as
    /// its slice.
    solid_view: bool,
    /// Whether the three-dimensional view is cut along the shot line.
    section: bool,
    /// Surface of the particles last shown in three dimensions; `None`
    /// when they have changed since.
    surface: Option<Surface>,
    /// Corners of the part of space the three-dimensional view shows.
    framing: Option<(glam::Vec3, glam::Vec3)>,
    /// Outcome of the last file operation or start of a run, shown in the
    /// toolbar.
    message: Option<String>,
}

impl Default for App {
    fn default() -> Self {
        Self::new(presets::apfsds_against_steel())
    }
}

impl App {
    /// Creates the window state for editing `file`.
    #[must_use]
    pub fn new(file: ScenarioFile) -> Self {
        let mut app = Self {
            file,
            path: None,
            mesh: None,
            preview: Err(String::new()),
            run: None,
            field: Field::Material,
            camera: Camera::default(),
            camera3: Camera3::default(),
            solid_view: true,
            section: true,
            surface: None,
            framing: None,
            message: None,
        };
        app.rebuild_preview();
        app
    }

    /// Why the scenario cannot be simulated, if it cannot.
    #[must_use]
    pub fn problem(&self) -> Option<&str> {
        self.preview.as_ref().err().map(String::as_str)
    }

    /// Whether a simulation has been started and not yet reset.
    #[must_use]
    pub fn has_run(&self) -> bool {
        self.run.is_some()
    }

    /// Whether the started simulation has reached its end time.
    #[must_use]
    pub fn is_finished(&self) -> bool {
        self.run
            .as_ref()
            .is_some_and(|run| run.state == RunState::Finished)
    }

    /// Latest reported state of the started simulation.
    #[must_use]
    pub fn latest_summary(&self) -> Option<&ImpactSummary> {
        self.run.as_ref().and_then(|run| run.history.last())
    }

    /// Replaces the scenario being edited and discards any run.
    pub fn set_file(&mut self, file: ScenarioFile) {
        self.run = None;
        self.file = file;
        self.camera = Camera::default();
        self.camera3 = Camera3::default();
        self.rebuild_preview();
    }

    /// Opens a scenario file.
    ///
    /// # Errors
    /// Returns [`IoError`] if the file cannot be read; the current scenario
    /// is then kept.
    pub fn open(&mut self, path: &Path) -> Result<(), IoError> {
        let file = ScenarioFile::load(path)?;
        // A mesh named by the scenario is looked for next to it.
        self.path = Some(path.to_owned());
        self.set_file(file);
        Ok(())
    }

    /// Saves the scenario to `path` and remembers it for later saves.
    ///
    /// # Errors
    /// Returns [`IoError`] if the file cannot be written.
    pub fn save(&mut self, path: &Path) -> Result<(), IoError> {
        self.file.save(path)?;
        self.path = Some(path.to_owned());
        // A mesh named relative to the old place may not be found from here.
        self.rebuild_preview();
        Ok(())
    }

    /// Reads the mesh the armor refers to, unless it has been read already.
    /// A relative file name starts at the folder of the scenario file.
    fn refresh_mesh(&mut self) -> Result<(), String> {
        let Some(solid) = self.file.scenario.armor.solid() else {
            self.mesh = None;
            return Ok(());
        };
        solid.validate().map_err(|error| error.to_string())?;
        let folder = self.path.as_deref().and_then(Path::parent);
        let path = folder.unwrap_or_else(|| Path::new("")).join(&solid.file);
        if self
            .mesh
            .as_ref()
            .is_some_and(|(loaded, _)| *loaded == path)
        {
            return Ok(());
        }
        self.mesh = None;
        let mesh = stl::load(&path).map_err(|error| error.to_string())?;
        self.mesh = Some((path, mesh));
        Ok(())
    }

    fn mesh(&self) -> Option<&TriangleMesh> {
        self.mesh.as_ref().map(|(_, mesh)| mesh)
    }

    fn rebuild_preview(&mut self) {
        self.preview = self
            .refresh_mesh()
            .and_then(|()| Preview::new(&self.file, self.mesh()));
        self.surface = None;
        self.framing = None;
    }

    /// Starts simulating the scenario. Does nothing if it is invalid or a
    /// simulation has already been started. `ctx` is asked to repaint
    /// whenever progress arrives.
    pub fn start(&mut self, ctx: &egui::Context) {
        if self.run.is_some() {
            return;
        }
        let Ok(preview) = &self.preview else {
            return;
        };
        let simulation = match Simulation::with_mesh(
            &self.file.scenario,
            &self.file.settings,
            &self.file.materials,
            self.mesh(),
        ) {
            Ok(simulation) => simulation,
            Err(error) => {
                self.message = Some(error.to_string());
                return;
            }
        };
        self.message = Some(format!("Running on {}", simulation.solver_name()));
        let repaint = ctx.clone();
        let runner = Runner::start(simulation, SLICES, move || {
            repaint.request_repaint();
        });
        self.run = Some(Run {
            runner,
            particles: preview.simulation.particles().into_owned(),
            cloud: preview.cloud.clone(),
            history: vec![preview.simulation.summary()],
            state: RunState::Running,
        });
    }

    /// Pauses a running simulation or resumes a paused one.
    pub fn toggle_pause(&mut self) {
        if let Some(run) = &mut self.run {
            match run.state {
                RunState::Running => {
                    run.runner.pause();
                    run.state = RunState::Paused;
                }
                RunState::Paused => {
                    run.runner.resume();
                    run.state = RunState::Running;
                }
                RunState::Finished | RunState::Failed(_) => {}
            }
        }
    }

    /// Stops and discards the simulation, returning to the editable scenario.
    pub fn reset(&mut self) {
        self.run = None;
        self.surface = None;
    }

    /// Takes in the progress reported since the last frame.
    fn poll(&mut self) {
        let Some(run) = &mut self.run else {
            return;
        };
        let surface = &mut self.surface;
        let mut accept = |snapshot: Snapshot| {
            run.particles = snapshot.particles;
            run.cloud = snapshot.cloud;
            run.history.push(snapshot.summary);
            *surface = None;
        };
        for update in run.runner.poll() {
            match update {
                Update::Progress(snapshot) => accept(snapshot),
                Update::Finished(snapshot) => {
                    accept(snapshot);
                    run.state = RunState::Finished;
                }
                Update::Failed(message) => run.state = RunState::Failed(message),
            }
        }
    }

    fn report(&mut self, action: &str, result: Result<(), IoError>) {
        self.message = Some(match result {
            Ok(()) => {
                let path = self.path.as_deref().map(Path::display);
                path.map_or_else(|| action.to_owned(), |path| format!("{action} {path}"))
            }
            Err(error) => error.to_string(),
        });
    }

    fn file_menu(&mut self, ui: &mut egui::Ui) {
        ui.menu_button("New", |ui| {
            if ui.button("APFSDS against steel").clicked() {
                self.path = None;
                self.set_file(presets::apfsds_against_steel());
                ui.close_menu();
            }
            if ui.button("APHE against a turret cheek").clicked() {
                self.path = None;
                self.set_file(presets::aphe_against_cast_armor());
                ui.close_menu();
            }
            if ui.button("APBC against steel").clicked() {
                self.path = None;
                self.set_file(presets::apbc_against_steel());
                ui.close_menu();
            }
        });
        let dialog = || rfd::FileDialog::new().add_filter("Scenario", &["json"]);
        if ui.button("Open…").clicked() {
            if let Some(path) = dialog().pick_file() {
                let result = self.open(&path);
                self.report("Opened", result);
            }
        }
        let save_as = ui.button("Save as…").clicked();
        let save = ui
            .add_enabled(self.path.is_some(), egui::Button::new("Save"))
            .clicked();
        let target = if save_as {
            dialog().set_file_name("scenario.json").save_file()
        } else if save {
            self.path.clone()
        } else {
            None
        };
        if let Some(path) = target {
            let result = self.save(&path);
            self.report("Saved", result);
        }
    }

    fn run_controls(&mut self, ui: &mut egui::Ui) {
        let state = self.run.as_ref().map(|run| run.state.clone());
        match state {
            None => {
                let ready = self.preview.is_ok();
                if ui.add_enabled(ready, egui::Button::new("▶ Run")).clicked() {
                    self.start(ui.ctx());
                }
            }
            Some(RunState::Running) => {
                if ui.button("⏸ Pause").clicked() {
                    self.toggle_pause();
                }
            }
            Some(RunState::Paused) => {
                if ui.button("▶ Resume").clicked() {
                    self.toggle_pause();
                }
            }
            Some(RunState::Finished | RunState::Failed(_)) => {}
        }
        if ui
            .add_enabled(self.run.is_some(), egui::Button::new("⟲ Reset"))
            .clicked()
        {
            self.reset();
        }
    }

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            self.file_menu(ui);
            ui.separator();
            self.run_controls(ui);
            ui.separator();
            ui.label("Colour by");
            egui::ComboBox::from_id_salt("field")
                .selected_text(self.field.label())
                .show_ui(ui, |ui| {
                    for field in Field::ALL {
                        ui.selectable_value(&mut self.field, field, field.label());
                    }
                });
            if self.cloud().is_some() {
                ui.separator();
                ui.checkbox(&mut self.solid_view, "3D");
                if ui
                    .add_enabled(
                        self.solid_view,
                        egui::Checkbox::new(&mut self.section, "Section"),
                    )
                    .changed()
                {
                    self.surface = None;
                    self.framing = None;
                }
            }
            if let Some(message) = &self.message {
                ui.separator();
                ui.weak(message);
            }
        });
    }

    /// All particles of the solid model being shown, if it is one.
    fn cloud(&self) -> Option<&Cloud> {
        match &self.run {
            Some(run) => run.cloud.as_ref(),
            None => self.preview.as_ref().ok()?.cloud.as_ref(),
        }
    }

    fn editors(&mut self, ui: &mut egui::Ui) {
        if let Some(problem) = self.problem() {
            ui.colored_label(ui.visuals().error_fg_color, problem);
            ui.separator();
        }
        let editable = self.run.is_none();
        if !editable {
            ui.weak("Reset the simulation to edit the scenario.");
        }
        let mut changed = false;
        ui.add_enabled_ui(editable, |ui| {
            let available = materials::available(&self.file.materials);
            egui::CollapsingHeader::new("Projectile")
                .default_open(true)
                .show(ui, |ui| {
                    changed |= editors::projectile(ui, &mut self.file.scenario, &available);
                });
            egui::CollapsingHeader::new("Armor")
                .default_open(true)
                .show(ui, |ui| {
                    changed |= editors::armor(ui, &mut self.file.scenario.armor, &available);
                });
            egui::CollapsingHeader::new("Materials").show(ui, |ui| {
                changed |= materials::editor(ui, &mut self.file.materials);
            });
            egui::CollapsingHeader::new("Simulation").show(ui, |ui| {
                changed |= editors::settings(ui, &mut self.file.settings);
            });
        });
        if changed {
            self.rebuild_preview();
        }
    }

    fn results(&self, ui: &mut egui::Ui) {
        let Ok(preview) = &self.preview else {
            ui.weak("Fix the scenario to see the model.");
            return;
        };
        let layout = preview.simulation.layout();
        egui::Grid::new("model").num_columns(2).show(ui, |ui| {
            let symmetry = match layout.kind {
                ModelKind::Axisymmetric => "axisymmetric",
                ModelKind::PlaneStrain => "plane strain",
                ModelKind::Solid => "3D half model (cross-section shown)",
            };
            let particles = preview.simulation.particle_count();
            for (label, value) in [
                ("Model", symmetry.to_owned()),
                ("Cell size", format!("{:.3} mm", layout.cell_size * 1.0e3)),
                ("Particles", particles.to_string()),
                (
                    "Simulated time",
                    format!("{:.1} µs", layout.end_time * 1.0e6),
                ),
            ] {
                ui.label(label);
                ui.label(value);
                ui.end_row();
            }
        });
        if layout.kind == ModelKind::PlaneStrain {
            ui.weak(
                "An oblique impact is modelled in plane strain: the projectile acts as a slab, \
                 so depths are indicative only.",
            );
        }
        ui.separator();

        let Some(run) = &self.run else {
            ui.weak("Press Run to simulate.");
            return;
        };
        let Some(summary) = run.history.last() else {
            return;
        };
        match &run.state {
            RunState::Running => ui.label("Running…"),
            RunState::Paused => ui.label("Paused"),
            RunState::Finished => ui.strong(if summary.perforated {
                "Finished: armor perforated"
            } else {
                "Finished: armor not perforated"
            }),
            RunState::Failed(message) => ui.colored_label(ui.visuals().error_fg_color, message),
        };
        #[expect(
            clippy::cast_possible_truncation,
            reason = "a fraction between 0 and 1"
        )]
        let progress = (summary.time / layout.end_time) as f32;
        ui.add(egui::ProgressBar::new(progress).show_percentage());

        egui::Grid::new("summary").num_columns(2).show(ui, |ui| {
            for (label, value) in [
                ("Time", format!("{:.1} µs", summary.time * 1.0e6)),
                (
                    "Depth along shot line",
                    format!(
                        "{:.1} of {:.1} mm",
                        summary.penetration_depth * 1.0e3,
                        summary.armor_path_length * 1.0e3
                    ),
                ),
                (
                    "Residual speed",
                    format!("{:.0} m/s", summary.residual_speed),
                ),
                (
                    "Residual mass",
                    format!("{:.0} %", summary.residual_mass_fraction * 100.0),
                ),
                (
                    "Projectile failed",
                    format!("{:.0} %", summary.projectile_failed_fraction * 100.0),
                ),
                (
                    "Armor failed",
                    format!("{:.1} %", summary.armor_failed_fraction * 100.0),
                ),
            ] {
                ui.label(label);
                ui.label(value);
                ui.end_row();
            }
        });
        ui.add_space(6.0);
        let depth: Vec<[f64; 2]> = run
            .history
            .iter()
            .map(|summary| [summary.time * 1.0e6, summary.penetration_depth * 1.0e3])
            .collect();
        widgets::line_chart(ui, &depth, "Depth [mm] over time [µs]", 150.0);
    }

    fn view(&mut self, ui: &mut egui::Ui) {
        if self.solid_view {
            if self.surface.is_none() {
                self.surface = self
                    .cloud()
                    .map(|cloud| Surface::extract(cloud, self.section));
                // Keep showing the part of space the model first filled, so
                // that it does not shift or shrink as debris spreads.
                if let Some(surface) = &mut self.surface {
                    surface.bounds = *self.framing.get_or_insert(surface.bounds);
                }
            }
            if let Some(surface) = &self.surface {
                view3d::show(ui, surface, self.field, &mut self.camera3);
                return;
            }
        }
        let Ok(preview) = &self.preview else {
            ui.centered_and_justified(|ui| ui.weak("Nothing to show."));
            return;
        };
        let initial = preview.simulation.particles();
        let particles = self
            .run
            .as_ref()
            .map_or(&*initial, |run| run.particles.as_slice());
        let layout = preview.simulation.layout();
        let scene = Scene {
            particles,
            bounds: preview.bounds,
            particle_size: preview.particle_size,
            mirror_axis: (layout.symmetry == Symmetry::Axisymmetric).then_some(layout.shot_line),
        };
        view::show(ui, &scene, self.field, &mut self.camera);
    }

    /// Draws one frame of the interface.
    pub fn show(&mut self, ctx: &egui::Context) {
        self.poll();
        egui::TopBottomPanel::top("toolbar").show(ctx, |ui| self.toolbar(ui));
        egui::SidePanel::left("editors")
            .resizable(true)
            .default_width(380.0)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| self.editors(ui));
            });
        egui::SidePanel::right("results")
            .resizable(true)
            .default_width(260.0)
            .show(ctx, |ui| {
                ui.heading("Results");
                egui::ScrollArea::vertical().show(ui, |ui| self.results(ui));
            });
        egui::CentralPanel::default().show(ctx, |ui| self.view(ui));
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.show(ctx);
    }
}

#[cfg(test)]
mod tests {
    use std::thread;
    use std::time::{Duration, Instant};

    use eframe::egui;
    use rap_app::presets;
    use rap_io::ScenarioFile;

    use super::App;

    /// A coarse, short scenario: these tests exercise the interface.
    fn quick_file() -> ScenarioFile {
        let mut file = presets::apfsds_against_steel();
        file.settings.cells_per_radius = 2.0;
        file.settings.armor_extent_radii = 4.0;
        file.settings.end_time = Some(4.0e-5);
        file
    }

    /// Runs one frame of the interface without a window.
    fn frame(app: &mut App, ctx: &egui::Context) {
        let _ = ctx.run(egui::RawInput::default(), |ctx| app.show(ctx));
    }

    #[test]
    fn both_presets_draw_without_a_problem() {
        let ctx = egui::Context::default();
        let mut shell = presets::aphe_against_cast_armor();
        // The preview of a solid model is built in full; keep it small.
        shell.settings.cells_per_radius = 2.0;
        shell.settings.armor_extent_radii = 3.0;
        for file in [
            presets::apfsds_against_steel(),
            presets::apbc_against_steel(),
            shell,
        ] {
            let mut app = App::new(file);
            assert_eq!(app.problem(), None);
            for _ in 0..3 {
                frame(&mut app, &ctx);
            }
            assert!(!app.has_run());
        }
    }

    #[test]
    fn simulation_runs_to_the_end_and_can_be_reset() {
        let ctx = egui::Context::default();
        let mut app = App::new(quick_file());
        frame(&mut app, &ctx);
        app.start(&ctx);
        assert!(app.has_run());
        let deadline = Instant::now() + Duration::from_secs(120);
        while !app.is_finished() {
            assert!(
                Instant::now() < deadline,
                "the simulation did not finish in time"
            );
            frame(&mut app, &ctx);
            thread::sleep(Duration::from_millis(5));
        }
        let summary = app.latest_summary().expect("a finished run has a summary");
        assert!((summary.time - 4.0e-5).abs() < 1e-12);
        assert!(summary.penetration_depth > 0.0);
        frame(&mut app, &ctx);

        app.reset();
        assert!(!app.has_run());
        frame(&mut app, &ctx);
    }

    #[test]
    fn pausing_and_resuming_keeps_the_run_alive() {
        let ctx = egui::Context::default();
        let mut app = App::new(quick_file());
        app.start(&ctx);
        app.toggle_pause();
        frame(&mut app, &ctx);
        assert!(app.has_run() && !app.is_finished());
        app.toggle_pause();
        frame(&mut app, &ctx);
        app.reset();
    }

    #[test]
    fn invalid_scenario_is_explained_and_cannot_be_started() {
        let ctx = egui::Context::default();
        let mut file = quick_file();
        let armor = file.scenario.armor.plates_mut().expect("plates");
        armor.layers[0].thickness = 0.0;
        let mut app = App::new(file);
        let problem = app.problem().expect("a zero-thickness layer is invalid");
        assert!(problem.contains("layer.thickness"), "{problem}");
        frame(&mut app, &ctx);
        app.start(&ctx);
        assert!(!app.has_run());
    }

    #[test]
    fn scenario_can_be_saved_and_opened_again() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let path = directory.path().join("scenario.json");
        let mut app = App::new(quick_file());
        app.save(&path).expect("the scenario is saved");

        let mut other = App::default();
        other.open(&path).expect("the scenario is opened");
        assert_eq!(other.file.settings, quick_file().settings);
        assert_eq!(other.path.as_deref(), Some(path.as_path()));

        let missing = directory.path().join("missing.json");
        assert!(other.open(&missing).is_err());
        // A failed open keeps what was there.
        assert_eq!(other.file.settings, quick_file().settings);
    }

    #[test]
    fn armor_from_a_mesh_file_is_read_from_next_to_the_scenario() {
        use glam::DVec3;
        use rap_domain::{MaterialId, SolidArmor};
        use rap_geometry::TriangleMesh;

        let directory = tempfile::tempdir().expect("a temporary directory");
        let block = TriangleMesh::cuboid(DVec3::new(0.0, -100.0, -100.0), DVec3::splat(100.0));
        std::fs::write(
            directory.path().join("block.stl"),
            rap_io::stl::to_text(&block, "block"),
        )
        .expect("the mesh is written");
        let mut file = quick_file();
        let mut solid = SolidArmor::new("block.stl", MaterialId::new("steel-4340"));
        solid.max_depth = Some(0.02);
        file.scenario.armor = solid.into();
        let path = directory.path().join("scenario.json");
        file.save(&path).expect("the scenario is saved");

        // Without the folder of the scenario the mesh cannot be found.
        let mut app = App::new(file);
        let problem = app.problem().expect("the mesh is not here");
        assert!(problem.contains("block.stl"), "{problem}");

        app.open(&path).expect("the scenario is opened");
        assert_eq!(app.problem(), None);
        let ctx = egui::Context::default();
        for _ in 0..3 {
            frame(&mut app, &ctx);
        }
        // Back to plates, the mesh is forgotten.
        app.set_file(quick_file());
        assert_eq!(app.problem(), None);
        assert!(app.mesh().is_none());
    }
}
