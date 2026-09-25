//! The daily look for a newer release, and installing it from the menu.

use std::sync::mpsc;
use std::time::{Instant, SystemTime};

use eframe::egui;

use crate::update::{self, Version};

use super::ChronoApp;

impl ChronoApp {
    /// Looks for a newer release once a day, deciding on frames that are drawn
    /// anyway: it never wakes the overlay for it. The request runs on a thread
    /// of its own, and its answer costs one frame.
    pub(super) fn check_for_updates(&mut self, ctx: &egui::Context, now: Instant) {
        self.poll_update_download();
        if let Some(pending) = &self.update_check {
            let answer = match pending.try_recv() {
                Ok(answer) => answer,
                Err(mpsc::TryRecvError::Empty) => return,
                Err(mpsc::TryRecvError::Disconnected) => None,
            };
            self.update_check = None;
            match answer {
                Some(latest) => {
                    self.settings.update_checked = epoch_s(SystemTime::now());
                    let offer = (latest > Version::current()).then(|| latest.to_string());
                    if offer != self.settings.update_available {
                        self.update_step = UpdateStep::Offered;
                    }
                    self.settings.update_available = offer;
                }
                None => self.update_retry = Some(now + update::RETRY_AFTER),
            }
        }
        let retry_passed = self.update_retry.is_none_or(|at| now >= at);
        if !self.updates_allowed
            || !self.settings.check_updates
            || !retry_passed
            || !update::due(self.settings.update_checked, epoch_s(SystemTime::now()))
        {
            return;
        }
        self.update_retry = None;
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        let spawned = std::thread::Builder::new().name("update check".to_owned()).spawn(move || {
            let latest = update::latest().inspect_err(|err| eprintln!("ChronoDesk: update check failed: {err}")).ok();
            let _ = tx.send(latest);
            ctx.request_repaint();
        });
        match spawned {
            Ok(_) => self.update_check = Some(rx),
            Err(_) => self.update_retry = Some(now + update::RETRY_AFTER),
        }
    }

    /// The menu's update item was clicked. The installed copy downloads the
    /// release, verifies it and runs its setup, which replaces this exe and
    /// starts the new one; a portable copy, or one whose download did not
    /// verify, gets the release page instead.
    pub(super) fn start_update(&mut self, ctx: &egui::Context) {
        let Some(version) = self.settings.update_available.as_deref().and_then(Version::parse) else { return };
        // A script's overlay, or one in a test, neither downloads nor opens a browser.
        if !self.updates_allowed {
            return;
        }
        match self.update_step {
            UpdateStep::Offered if self.installed => {
                let (tx, rx) = mpsc::channel();
                let ctx = ctx.clone();
                let spawned = std::thread::Builder::new().name("update download".to_owned()).spawn(move || {
                    let _ = tx.send(update::download_setup(version));
                    ctx.request_repaint();
                });
                if spawned.is_ok() {
                    self.update_download = Some(rx);
                    self.update_step = UpdateStep::Downloading;
                } else {
                    update::open_release_page(version);
                }
            }
            UpdateStep::Downloading | UpdateStep::Installing => {}
            _ => update::open_release_page(version),
        }
    }

    /// Hands a verified download to its setup, which asks this overlay to
    /// quit, replaces the exe and starts the new version in its place.
    fn poll_update_download(&mut self) {
        let Some(pending) = &self.update_download else { return };
        let result = match pending.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => Err(update::Refused::Download(std::io::Error::other("stopped"))),
        };
        self.update_download = None;
        self.update_step = match result.map(|setup| std::process::Command::new(&setup).arg("--quiet").spawn()) {
            Ok(Ok(_)) => UpdateStep::Installing,
            Ok(Err(err)) => {
                eprintln!("ChronoDesk: could not start the update: {err}");
                UpdateStep::Failed
            }
            Err(refused) => {
                eprintln!("ChronoDesk: update not installed: {refused}");
                UpdateStep::Failed
            }
        };
    }

    /// The menu's update item: what it says and whether it can be clicked.
    pub(super) fn update_item(&self) -> Option<(String, bool)> {
        let version = self.settings.update_available.as_deref()?;
        Some(match self.update_step {
            UpdateStep::Offered if self.installed => (format!("Update to ChronoDesk {version} now"), true),
            UpdateStep::Offered => (format!("Update available: ChronoDesk {version}…"), true),
            UpdateStep::Downloading => (format!("Downloading ChronoDesk {version}…"), false),
            UpdateStep::Installing => (format!("Installing ChronoDesk {version}…"), false),
            UpdateStep::Failed => (format!("Update failed: download ChronoDesk {version}…"), true),
        })
    }
}

/// How far the update on offer has got.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum UpdateStep {
    /// Found by the daily check; a click starts it.
    Offered,
    Downloading,
    /// The setup runs, and is about to ask this overlay to quit.
    Installing,
    /// The download failed or did not verify; a click opens the release page.
    Failed,
}

fn epoch_s(wall: SystemTime) -> u64 {
    wall.duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Local;
    use crate::config::Config;
    use crate::tray::Command;

    /// Once the running version has caught up, the offer is withdrawn.
    #[test]
    fn an_update_offer_the_running_version_has_reached_is_dropped() {
        let ctx = egui::Context::default();
        let newer = Config { update_available: Some("999.0.0".to_owned()), ..Config::default() };
        assert_eq!(ChronoApp::pinned(&ctx, newer, Local::now()).settings.update_available.as_deref(), Some("999.0.0"));
        let reached = Config { update_available: Some(Version::current().to_string()), ..Config::default() };
        assert_eq!(ChronoApp::pinned(&ctx, reached, Local::now()).settings.update_available, None);
        let garbage = Config { update_available: Some("soon".to_owned()), ..Config::default() };
        assert_eq!(ChronoApp::pinned(&ctx, garbage, Local::now()).settings.update_available, None);
    }

    #[test]
    fn an_update_is_offered_at_the_top_of_the_menu_and_withdrawn() {
        let ctx = egui::Context::default();
        let mut app = ChronoApp::pinned(&ctx, Config::default(), Local::now());
        let now = Instant::now();
        app.tray.sync(app.menu_state(now));
        let (plain, offered) = app.tray.top_level();
        assert!(!offered);

        app.settings.update_available = Some("999.0.0".to_owned());
        app.tray.sync(app.menu_state(now));
        assert_eq!(app.tray.top_level(), (plain + 2, true), "the offer and a separator");

        app.apply(Command::ToggleUpdateCheck, &ctx, now);
        assert!(!app.settings.check_updates && app.settings.update_available.is_none());
        app.tray.sync(app.menu_state(now));
        assert_eq!(app.tray.top_level(), (plain, false));
    }

    #[test]
    fn the_update_item_says_what_a_click_will_do() {
        let ctx = egui::Context::default();
        let offer = Config { update_available: Some("999.0.0".to_owned()), ..Config::default() };
        let mut app = ChronoApp::pinned(&ctx, offer, Local::now());
        assert_eq!(app.update_item(), Some(("Update available: ChronoDesk 999.0.0…".to_owned(), true)), "portable");
        app.installed = true;
        assert_eq!(app.update_item(), Some(("Update to ChronoDesk 999.0.0 now".to_owned(), true)));
        app.update_step = UpdateStep::Downloading;
        assert_eq!(app.update_item().map(|(_, enabled)| enabled), Some(false), "no second download");
        app.update_step = UpdateStep::Failed;
        assert_eq!(app.update_item(), Some(("Update failed: download ChronoDesk 999.0.0…".to_owned(), true)));
        app.settings.update_available = None;
        assert_eq!(app.update_item(), None);
    }

    /// A download that did not verify, or a setup that would not start, ends
    /// in the fallback; nothing is run.
    #[test]
    fn an_update_that_fails_falls_back_to_the_release_page() {
        let ctx = egui::Context::default();
        let offer = Config { update_available: Some("999.0.0".to_owned()), ..Config::default() };
        let mut app = ChronoApp::pinned(&ctx, offer, Local::now());
        for result in [
            Err(update::Refused::Signature),
            Ok(std::env::temp_dir().join("chronodesk-test-no-such-setup.exe")),
        ] {
            let (tx, rx) = mpsc::channel();
            tx.send(result).unwrap();
            (app.update_step, app.update_download) = (UpdateStep::Downloading, Some(rx));
            app.poll_update_download();
            assert_eq!(app.update_step, UpdateStep::Failed);
            assert!(app.update_download.is_none());
        }
    }

    /// Test instances, like a script's, never go out to the network.
    #[test]
    fn an_overlay_rendered_in_a_test_does_not_check_for_updates() {
        let ctx = egui::Context::default();
        let mut app = ChronoApp::pinned(&ctx, Config::default(), Local::now());
        app.check_for_updates(&ctx, Instant::now());
        assert!(app.update_check.is_none());
    }
}
