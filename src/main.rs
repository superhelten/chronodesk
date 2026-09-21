#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod autostart;
mod board;
mod chime;
mod clock;
mod config;
mod digital;
mod icon;
mod install;
mod instance;
mod instrument;
mod layout;
mod market;
mod matrix;
mod night;
mod placement;
mod registry;
mod ring;
mod signal;
mod text;
mod theme;
mod timer;
mod tray;
mod tz;
mod welcome;

#[cfg(test)]
mod shots;

use eframe::egui;

fn main() -> eframe::Result {
    // Before anything is read or shown: a second launch against the same config
    // file would fight the first over it, so it steps aside.
    let config_path = config::config_path();

    // The same exe is its own installer; see `install`.
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(action) = install::requested(&args, std::env::current_exe().ok().as_deref()) {
        std::process::exit(install::run(action, &args, config_path.as_deref()));
    }

    let _instance = match config_path.as_deref().map(instance::acquire) {
        Some(None) => {
            // Stepping aside silently looks like a launch that did nothing, so
            // the overlay that is already there is asked to show itself.
            let heard = config_path.as_deref().is_some_and(|path| signal::send(path, signal::Signal::Show));
            eprintln!("ChronoDesk: already running with this config file{}", if heard { "; asked it to show itself" } else { "" });
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
            // Without a file the welcome could never be marked as seen, and
            // would be back at every launch.
            config: config::Config::returning(),
            warnings: Vec::new(),
            quarantined: None,
            migrated: false,
        }
    });
    let position = loaded.config.window.map_or([80.0, 80.0], |w| [w.x, w.y]);

    // A test instance belongs to a script, not to whoever is at the keyboard:
    // taking the focus would hand it their keystrokes (Space, R and 1-4 mean
    // something here) and interrupt what they were typing.
    let scripted = instrument::requested(&args);

    let viewport = egui::ViewportBuilder::default()
        .with_title("ChronoDesk")
        .with_app_id("chronodesk")
        .with_icon(egui::IconData { rgba: icon.rgba, width: icon.size, height: icon.size })
        .with_transparent(true)
        .with_decorations(false)
        .with_resizable(false)
        // Not a courtesy but a lifeline: a minimised window is not drawn at all
        // (no ticks, no chime), and with no taskbar button there is nothing to
        // click to get it back. Without the style bit the system's own
        // minimise commands do not apply to it; a second launch restores it
        // if something minimised it anyway.
        .with_minimize_button(false)
        .with_active(!scripted)
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
