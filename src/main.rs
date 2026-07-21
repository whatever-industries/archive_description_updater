// Hide the console window that Windows would otherwise open alongside the GUI.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod ia;
mod replace;

/// Shown in the title bar, the window heading, and the macOS menu bar.
pub const APP_NAME: &str = "Archive.org Redump Link Updater";

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([900.0, 640.0])
            .with_min_inner_size([620.0, 420.0])
            .with_title(APP_NAME),
        ..Default::default()
    };

    eframe::run_native(
        APP_NAME,
        options,
        Box::new(|cc| Ok(Box::new(app::App::new(cc)))),
    )
}
