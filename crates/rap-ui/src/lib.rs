//! Desktop user interface: projectile editor, armor editor and simulation view.

mod app;
mod editors;
mod materials;
mod runner;
mod view;
mod view3d;
mod widgets;

use eframe::egui;
use rap_domain::APP_NAME;

pub use app::App;

/// Opens the application window and runs until it is closed.
///
/// # Errors
/// Returns an [`eframe::Error`] if the window or its graphics context
/// cannot be created.
pub fn run() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(APP_NAME)
            .with_inner_size([1360.0, 820.0])
            .with_min_inner_size([900.0, 560.0]),
        ..Default::default()
    };
    eframe::run_native(
        APP_NAME,
        options,
        Box::new(|_context| Ok(Box::new(App::default()))),
    )
}
