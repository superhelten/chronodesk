//! Writing the settings and the counters to disk, and where the window is
//! put at startup.

use std::time::{Duration, Instant, SystemTime};

use eframe::egui::{self, Pos2, Vec2, ViewportCommand, pos2};

use crate::config::{self, Config};
use crate::placement;
use crate::timer;

use super::window::window_position_px;
use super::{ChronoApp, WAKE_SLACK};

/// How long to wait for a window move to take effect before trusting the
/// window's own reported position again.
const PLACEMENT_TIMEOUT: Duration = Duration::from_millis(1500);
/// A move lands on whole pixels, so the window ends up within rounding
/// distance of where it was asked to go.
const ARRIVAL_TOLERANCE_PX: f32 = 2.0;
/// Frames the placement check may wait for the display scale to settle.
pub(super) const PLACEMENT_DEFERRALS: u8 = 10;
/// How long to wait after the last change before writing the config file.
const SAVE_DELAY: Duration = Duration::from_millis(1500);
/// How long after a failed write it is tried once more.
const SAVE_RETRY: Duration = Duration::from_secs(5);

impl ChronoApp {
    /// Notes the current window position (`position`, physical pixels) and
    /// writes the config once it has been unchanged for [`SAVE_DELAY`].
    pub(super) fn persist(&mut self, ctx: &egui::Context, now: Instant, position: Option<Pos2>) {
        if let Some(at) = self.save_retry {
            match at.checked_duration_since(now).filter(|left| !left.is_zero()) {
                Some(left) => ctx.request_repaint_after(left + WAKE_SLACK),
                None => {
                    self.save_retry = None;
                    self.write_config_once();
                }
            }
        }
        // Until the placement check has run the window is wherever the OS put
        // it, and while a move of ours is in flight it still reports the old
        // position: recording either would undo the placement in the file.
        if self.placement_pending.is_none()
            && self.placement_settled(ctx, now, position)
            && let Some(px) = position
        {
            self.record_position(px, ctx.pixels_per_point());
        }

        if self.settings != self.saved && self.dirty_since.is_none() {
            self.dirty_since = Some(now);
        }
        if let Some(since) = self.dirty_since {
            match SAVE_DELAY.checked_sub(now.duration_since(since)).filter(|left| !left.is_zero()) {
                None => self.write_config(),
                // What is left of the delay, not all of it again, and past it
                // by the slack a timed wake-up needs to not land just short.
                Some(left) => ctx.request_repaint_after(left + WAKE_SLACK),
            }
        }
    }

    pub(super) fn write_config(&mut self) {
        let now = Instant::now();
        self.write_config_once();
        if self.save_failed {
            self.save_retry = Some(now + SAVE_RETRY);
        }
    }

    /// One attempt, with no retry of its own.
    fn write_config_once(&mut self) {
        self.dirty_since = None;
        self.saved = self.settings.clone();
        let Some(path) = &self.config_path else { return };
        let result = config::save(path, &self.settings);
        if let Err(err) = &result {
            eprintln!("ChronoDesk: could not save config: {err}");
        }
        self.save_failed = result.is_err();
    }

    /// Both units from one reading, so they always name the same spot.
    fn record_position(&mut self, px: Pos2, ppp: f32) {
        let window_px = Some(config::WindowPos { x: px.x, y: px.y });
        if self.settings.window_px != window_px {
            self.settings.window_px = window_px;
            self.settings.window = Some(config::WindowPos { x: px.x / ppp, y: px.y / ppp });
        }
    }

    /// True once the window sits where we last asked it to, so its reported
    /// position can be trusted again.
    fn placement_settled(&mut self, ctx: &egui::Context, now: Instant, position: Option<Pos2>) -> bool {
        let Some((target, since)) = self.pending_move else {
            return true;
        };
        let arrived = position.is_some_and(|px| (px - target).length() <= ARRIVAL_TOLERANCE_PX);
        // A window manager is free to ignore a move, so this cannot wait forever.
        if arrived || now.duration_since(since) >= PLACEMENT_TIMEOUT {
            self.pending_move = None;
            return true;
        }
        ctx.request_repaint_after(Duration::from_millis(50));
        false
    }

    /// Checks the saved position against the monitors that are actually
    /// attached, and moves the overlay somewhere safe if it is stranded.
    ///
    /// `size` is the window's size in points, which is why this runs after the
    /// layout has been measured rather than in `main`: placing the overlay
    /// against the top-right corner needs its real width, and at window-creation
    /// time that is still a placeholder.
    pub(super) fn check_placement(&mut self, ctx: &egui::Context, frame: &eframe::Frame, size: Vec2, now: Instant) {
        let (ppp, zoom) = (ctx.pixels_per_point(), ctx.zoom_factor());
        // egui's points are physical pixels divided by the scale of the monitor
        // the window is on. Right after startup eframe may not have picked that
        // scale up yet, and converting with the wrong one would misplace the
        // window by the ratio between them, so wait until the two agree.
        if let Some(left) = self.placement_pending.filter(|&n| n > 0)
            && let Some(window) = frame.winit_window()
            && (ppp - window.scale_factor() as f32 * zoom).abs() > 1e-3
        {
            self.placement_pending = Some(left - 1);
            ctx.request_repaint();
            return;
        }
        self.placement_pending = None;

        let Some(saved) = self.saved_position else {
            self.instrument.set_placement_report("decision=no-saved-position".to_owned());
            return;
        };
        let monitors = placement::monitors(frame);
        let saved_px = saved.pixels(ppp);
        let (target, decision) = match placement::evaluate(saved_px, size, zoom, &monitors) {
            placement::Outcome::Keep => (saved_px, "keep".to_owned()),
            placement::Outcome::Move { to, reason } => {
                eprintln!(
                    "ChronoDesk: saved position ({:.0},{:.0}) px is {}; moved to ({:.0},{:.0}) px",
                    saved_px.x,
                    saved_px.y,
                    reason.as_str(),
                    to.x,
                    to.y,
                );
                (to, format!("{} moved_to_px=({:.0},{:.0})", reason.as_str(), to.x, to.y))
            }
        };

        // Even a position that is kept has to be applied: the window was created
        // at it in points, which the OS converted with the scale of whichever
        // monitor the window first came up on. With screens at different
        // scaling that is the wrong one, and only pixels land exactly.
        let actual = window_position_px(ctx, frame);
        let corrected = actual.is_none_or(|px| (px - target).length() > ARRIVAL_TOLERANCE_PX);
        if corrected {
            match frame.winit_window() {
                // Physical, so nothing is scaled on the way.
                Some(window) => window.set_outer_position(winit::dpi::PhysicalPosition::new(
                    target.x.round() as i32,
                    target.y.round() as i32,
                )),
                None => ctx.send_viewport_cmd(ViewportCommand::OuterPosition(pos2(target.x / ppp, target.y / ppp))),
            }
            self.pending_move = Some((target, now));
            ctx.request_repaint();
        }
        self.record_position(target, ppp);
        if target != saved_px {
            // Straight to disk: the point of the exercise is that the next
            // launch starts from a position that exists.
            self.write_config();
        }
        let saved_unit = match saved {
            SavedPosition::Pixels(_) => "px",
            SavedPosition::Points(_) => "pt",
        };
        self.instrument.set_placement_report(format!(
            "decision={decision} corrected={} ppp={ppp:.3} zoom={zoom:.3} size_pt=({:.1},{:.1}) \
             saved_in={saved_unit} saved_px=({:.0},{:.0}) monitors=[{}]",
            u8::from(corrected),
            size.x,
            size.y,
            saved_px.x,
            saved_px.y,
            placement::describe(&monitors),
        ));
    }

    /// Mirrors the two counters into the settings and writes them out at once:
    /// a logout or a power cut gives no notice, and the whole point is that a
    /// running timer survives one. Called after a command that touched a
    /// counter, and when the wall clock was set under a running one; that is
    /// enough, because a running counter is saved as the moment it started,
    /// which does not change while it runs.
    pub(super) fn save_counters(&mut self, now: Instant) {
        let wall = SystemTime::now();
        let counters = (self.stopwatch.save(now, wall), self.countdown.save(now, wall));
        if counters != (self.settings.stopwatch, self.settings.countdown) {
            (self.settings.stopwatch, self.settings.countdown) = counters;
            self.write_config();
        }
    }

    /// A running counter is saved as the wall-clock moment it started, which
    /// only adds up as long as nobody sets the clock. When somebody has (a
    /// time sync, by hand), what was saved would come back after a restart
    /// off by the correction, so it is saved again. Cheap enough per frame:
    /// one clock read and a comparison while something runs, nothing else.
    pub(super) fn follow_clock_changes(&mut self, now: Instant) {
        if !self.stopwatch.is_running() && !self.countdown.is_running(now) {
            return;
        }
        let wall = SystemTime::now();
        let moved = |saved: Option<timer::Saved>, current: Option<timer::Saved>| {
            saved.zip(current).is_some_and(|(saved, current)| saved.drifted(current))
        };
        if moved(self.settings.stopwatch, self.stopwatch.save(now, wall))
            || moved(self.settings.countdown, self.countdown.save(now, wall))
        {
            self.save_counters(now);
        }
    }
}

/// Where `app.ron` puts the overlay.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum SavedPosition {
    /// Physical pixels, which mean the same on every monitor.
    Pixels(Pos2),
    /// Points, from a file written before positions were kept in pixels.
    /// Converted with the scale the window has now, which is right as long as
    /// it came up on the monitor it was saved on.
    Points(Pos2),
}

impl SavedPosition {
    pub(super) fn of(settings: &Config) -> Option<Self> {
        let pos = |w: config::WindowPos| pos2(w.x, w.y);
        settings.window_px.map(|w| Self::Pixels(pos(w))).or_else(|| settings.window.map(|w| Self::Points(pos(w))))
    }

    pub(super) fn pixels(self, ppp: f32) -> Pos2 {
        match self {
            Self::Pixels(px) => px,
            Self::Points(pt) => pos2(pt.x * ppp, pt.y * ppp),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Local;

    /// A file that cannot be written (read-only, locked, a folder in its place)
    /// gets one attempt per change, not one every [`SAVE_DELAY`] for as long as
    /// the app runs, which would keep an idle overlay drawing. Quitting tries
    /// once more.
    #[test]
    fn a_failed_save_waits_for_the_next_change() {
        let dir = std::env::temp_dir().join(format!("chronodesk_test_save_fail_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("app.ron");
        std::fs::create_dir_all(&path).unwrap(); // a folder where the file goes: the rename fails

        let ctx = egui::Context::default();
        let mut app = ChronoApp::pinned(&ctx, Config::default(), Local::now());
        app.config_path = Some(path.clone());
        let t0 = Instant::now();

        app.settings.backdrop = true;
        app.persist(&ctx, t0, None);
        assert!(app.dirty_since.is_some());
        app.persist(&ctx, t0 + SAVE_DELAY, None);
        assert!(app.dirty_since.is_none(), "the attempt was made");
        app.persist(&ctx, t0 + SAVE_DELAY * 2, None);
        assert!(app.dirty_since.is_none(), "and is not repeated without a change");

        app.settings.chroma = true;
        app.persist(&ctx, t0 + SAVE_DELAY * 3, None);
        assert!(app.dirty_since.is_some(), "a new change is tried again");

        // The folder goes away and quitting gets the settings out after all.
        app.persist(&ctx, t0 + SAVE_DELAY * 4, None);
        std::fs::remove_dir(&path).unwrap();
        eframe::App::on_exit(&mut app, None);
        let saved = config::load(&path).config;
        assert!(saved.backdrop && saved.chroma, "written on exit");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A file held for a moment (a scanner at logon) gets one more try a few
    /// seconds later, without waiting for another change.
    #[test]
    fn a_failed_save_is_tried_once_more() {
        let dir = std::env::temp_dir().join(format!("chronodesk_test_save_retry_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("app.ron");
        std::fs::create_dir_all(&path).unwrap();

        let ctx = egui::Context::default();
        let mut app = ChronoApp::pinned(&ctx, Config::default(), Local::now());
        app.config_path = Some(path.clone());
        app.settings.backdrop = true;
        app.write_config();
        assert!(app.save_failed && app.save_retry.is_some());

        std::fs::remove_dir(&path).unwrap();
        app.persist(&ctx, Instant::now() + SAVE_RETRY + WAKE_SLACK, None);
        assert!(!app.save_failed && app.save_retry.is_none());
        assert!(config::load(&path).config.backdrop, "the retry wrote it");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// No path to write to (a file that could not be read, or no config
    /// directory at all): a change is noted once and then left be.
    #[test]
    fn without_a_file_to_write_a_change_does_not_keep_the_app_awake() {
        let ctx = egui::Context::default();
        let mut app = ChronoApp::pinned(&ctx, Config::default(), Local::now());
        let t0 = Instant::now();
        app.settings.backdrop = true;
        app.persist(&ctx, t0, None);
        app.persist(&ctx, t0 + SAVE_DELAY, None);
        app.persist(&ctx, t0 + SAVE_DELAY * 2, None);
        assert!(app.dirty_since.is_none());
        assert!(!app.save_failed, "nothing was tried");
    }

    #[test]
    fn a_position_in_pixels_wins_and_one_in_points_scales_with_the_window() {
        let at = |x, y| Some(config::WindowPos { x, y });
        let both = Config { window_px: at(3000.0, 150.0), window: at(2000.0, 100.0), ..Config::default() };
        assert_eq!(SavedPosition::of(&both).map(|s| s.pixels(1.5)), Some(pos2(3000.0, 150.0)));
        let points = Config { window: at(2000.0, 100.0), ..Config::default() };
        assert_eq!(SavedPosition::of(&points).map(|s| s.pixels(1.5)), Some(pos2(3000.0, 150.0)));
        assert_eq!(SavedPosition::of(&Config::default()), None);
    }

    /// Dragged to a 150 % screen: the file gets the exact pixel, and the
    /// points that go with it for the next window to be created at.
    #[test]
    fn a_recorded_position_keeps_pixels_and_points_together() {
        let ctx = egui::Context::default();
        let mut app = ChronoApp::pinned(&ctx, Config::default(), Local::now());
        app.placement_pending = None;
        app.persist(&ctx, Instant::now(), Some(pos2(3000.0, 150.0)));
        assert_eq!(app.settings.window_px, Some(config::WindowPos { x: 3000.0, y: 150.0 }));
        assert_eq!(app.settings.window, Some(config::WindowPos { x: 3000.0 / ctx.pixels_per_point(), y: 150.0 / ctx.pixels_per_point() }));
    }

    /// Before the placement check has run, the window is wherever the OS put
    /// it, and that must not reach the file.
    #[test]
    fn nothing_is_recorded_before_the_placement_check() {
        let ctx = egui::Context::default();
        let saved = Config { window_px: Some(config::WindowPos { x: 3000.0, y: 150.0 }), ..Config::default() };
        let mut app = ChronoApp::pinned(&ctx, saved.clone(), Local::now());
        assert!(app.placement_pending.is_some());
        app.persist(&ctx, Instant::now(), Some(pos2(2000.0, 100.0)));
        assert_eq!(app.settings.window_px, saved.window_px);
    }
}
