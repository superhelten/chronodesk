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
mod update;
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
    // Right after the guard, so a launch or installer that finds it held can
    // already be heard while the window is still being built.
    let inbox = config_path.as_deref().and_then(signal::Inbox::open);

    let icon = icon::app_icon(64, false);
    // Loaded before the window exists so the saved position can be applied up
    // front instead of making the overlay jump after the first frame.
    let loaded = config_path.as_deref().map(config::load).unwrap_or_else(|| {
        eprintln!("ChronoDesk: no config directory; settings will not be saved");
        // Without a file the welcome could never be marked as seen, and
        // would be back at every launch.
        config::Loaded::fallback(Vec::new(), None)
    });
    // Only a first guess: the OS converts points with the scale of whichever
    // monitor the window first comes up on. Once it runs, the placement check
    // puts it on the exact pixel from `window_px`.
    let position = loaded.config.window.or(loaded.config.window_px).map_or([80.0, 80.0], |w| [w.x, w.y]);

    // A test instance belongs to a script, not to whoever is at the keyboard,
    // and must not get in the way of what they are doing.
    let scripted = instrument::requested(&args);

    let viewport = egui::ViewportBuilder::default()
        .with_title("ChronoDesk")
        .with_app_id("chronodesk")
        .with_icon(egui::IconData { rgba: icon.rgba, width: icon.size, height: icon.size })
        .with_transparent(true)
        .with_decorations(false)
        .with_resizable(false)
        // Not a courtesy but a lifeline: with no taskbar button there would be
        // nothing to click to get a minimised overlay back. Without the style
        // bit the system's own minimise commands do not apply to it; a second
        // launch restores it if something minimised it anyway.
        .with_minimize_button(false)
        // An overlay never takes the focus when it starts, whether at login,
        // after an update or from a script: it would pull whoever is in a
        // full-screen game out of it. A click or a second launch focuses it.
        .with_active(false)
        // Nor may it sit on top of their game: a script's overlay stays at the
        // bottom of the pile, where it draws just the same.
        .with_window_level(if scripted { egui::WindowLevel::AlwaysOnBottom } else { egui::WindowLevel::AlwaysOnTop })
        .with_taskbar(false)
        .with_inner_size([220.0, 90.0])
        .with_position(position);

    let options = eframe::NativeOptions { viewport, ..Default::default() };

    eframe::run_native(
        "ChronoDesk",
        options,
        Box::new(move |cc| Ok(Box::new(app::ChronoApp::new(cc, loaded, inbox)?))),
    )
}
