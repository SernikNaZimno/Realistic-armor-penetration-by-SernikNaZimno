//! Desktop application entry point.

// Do not open a console window next to the application in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() -> eframe::Result {
    rap_ui::run()
}
