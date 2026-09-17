#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod icon;
mod timer;
mod tray;

use eframe::egui;

fn main() -> eframe::Result {
    let icon = icon::app_icon(64, false);

    let viewport = egui::ViewportBuilder::default()
        .with_title("ChronoDesk")
        .with_app_id("chronodesk")
        .with_icon(egui::IconData { rgba: icon.rgba, width: icon.size, height: icon.size })
        .with_transparent(true)
        .with_decorations(false)
        .with_resizable(false)
        .with_always_on_top()
        .with_taskbar(false)
        .with_inner_size([220.0, 90.0])
        .with_position([80.0, 80.0]);

    let options = eframe::NativeOptions {
        viewport,
        persist_window: true,
        ..Default::default()
    };

    eframe::run_native(
        "ChronoDesk",
        options,
        Box::new(|cc| Ok(Box::new(app::ChronoApp::new(cc)?))),
    )
}
