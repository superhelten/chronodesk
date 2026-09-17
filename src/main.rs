#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod config;
mod icon;
mod layout;
mod theme;
mod timer;
mod tray;

use eframe::egui;

fn main() -> eframe::Result {
    let icon = icon::app_icon(64, false);
    // Loaded before the window exists so the saved position can be applied up
    // front instead of making the overlay jump after the first frame.
    let loaded = config::config_path().map(|path| config::load(&path)).unwrap_or_else(|| {
        eprintln!("ChronoDesk: no config directory; settings will not be saved");
        config::Loaded {
            config: config::Config::default(),
            warnings: Vec::new(),
            quarantined: None,
            migrated: false,
        }
    });
    let position = loaded.config.window.map_or([80.0, 80.0], |w| [w.x, w.y]);

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
        .with_position(position);

    let options = eframe::NativeOptions { viewport, ..Default::default() };

    eframe::run_native(
        "ChronoDesk",
        options,
        Box::new(move |cc| Ok(Box::new(app::ChronoApp::new(cc, loaded)?))),
    )
}
