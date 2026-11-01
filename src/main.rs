#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod format;
mod scan;
mod treemap;

use eframe::egui;

fn main() -> eframe::Result {
    // An optional folder on the command line is scanned straight away.
    let start_path = std::env::args_os().nth(1).map(std::path::PathBuf::from);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Space Detective")
            .with_inner_size([1600.0, 950.0])
            .with_min_inner_size([900.0, 560.0])
            .with_maximized(true)
            .with_icon(eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon-256.png")).expect("valid icon")),
        ..Default::default()
    };
    eframe::run_native(
        "Space Detective",
        options,
        Box::new(move |cc| Ok(Box::new(app::App::new(cc, start_path)))),
    )
}
