#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod autostart;
mod board;
mod chime;
mod clock;
mod config;
mod digital;
mod icon;
mod instance;
mod instrument;
mod layout;
mod market;
mod matrix;
mod night;
mod placement;
mod ring;
mod text;
mod theme;
mod timer;
mod tray;
mod tz;

use eframe::egui;

fn main() -> eframe::Result {
    // Before anything is read or shown: a second launch against the same config
    // file would fight the first over it, so it steps aside.
    let config_path = config::config_path();
    let _instance = match config_path.as_deref().map(instance::acquire) {
        Some(None) => {
            eprintln!("ChronoDesk: already running with this config file");
            return Ok(());
        }
        guard => guard.flatten(),
    };

    let icon = icon::app_icon(64, false);
    // Loaded before the window exists so the saved position can be applied up
    // front instead of making the overlay jump after the first frame.
    let loaded = config_path.as_deref().map(config::load).unwrap_or_else(|| {
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
