use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant, SystemTime};

use chrono::{DateTime, Local, Timelike as _};
use eframe::egui::{
    self, Color32, FontId, PointerButton, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2,
    ViewportCommand, WindowLevel, pos2, vec2,
};
use serde::{Deserialize, Serialize};

use crate::autostart::Autostart;
use crate::board;
use crate::chime;
use crate::clock::{ClockStyle, clock_readout};
use crate::ring;
use crate::config::{self, Config};
use crate::digital;
use crate::instrument::{Cause, Instrument};
use crate::layout::{DerivedLayout, Font, LayoutKey, Metrics};
use crate::market::{self, BoardStyle, Market};
use crate::matrix;
use crate::night::{self, Schedule};
use crate::placement;
use crate::signal::{self, Signal};
use crate::text::{display_family, install_display_font, label_family, measure_glyphs, paint_galley, paint_readout, spaced};
use crate::theme::Theme;
use crate::timer::{self, Alarm, Countdown, Stopwatch};
use crate::tray::{Command, MenuState, Tray};
use crate::welcome;

/// How long a finished timer blinks before settling on a steady colour.
const BLINK_FOR: Duration = Duration::from_secs(30);
/// Timed wake-ups can fire more than one OS timer tick (~15.6 ms on Windows)
/// early. Waking just before a boundary would find "1 ms left" and spin at vsync
/// until it passes, so aim well past it; a 25 ms display lag is invisible.
const WAKE_SLACK: Duration = Duration::from_millis(25);
/// How long to wait for a window move to take effect before trusting the
/// window's own reported position again.
const PLACEMENT_TIMEOUT: Duration = Duration::from_millis(1500);
/// A move is expressed in points and applied in whole pixels, so the window
/// lands within rounding distance of where it was asked to go.
const ARRIVAL_TOLERANCE_PX: f32 = 2.0;
/// Frames the placement check may wait for the display scale to settle.
const PLACEMENT_DEFERRALS: u8 = 10;
/// How long the overlay stays outlined after another launch asked for it.
const ATTENTION_FOR: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    #[default]
    Clock,
    Stopwatch,
    Timer,
    /// The market board: one row per exchange, in its local time.
    Market,
}

impl Mode {
    pub const ALL: [Self; 4] = [Self::Clock, Self::Stopwatch, Self::Timer, Self::Market];

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.id().eq_ignore_ascii_case(id))
    }

    pub fn id(self) -> &'static str {
        match self {
            Self::Clock => "clock",
            Self::Stopwatch => "stopwatch",
            Self::Timer => "timer",
            Self::Market => "market",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Clock => "Clock",
            Self::Stopwatch => "Stopwatch",
            Self::Timer => "Timer",
            Self::Market => "Markets",
        }
    }

    /// Whether start/pause and reset mean anything: only the two modes
    /// that run something have hover controls and menu actions.
    pub fn has_controls(self) -> bool {
        matches!(self, Self::Stopwatch | Self::Timer)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Size {
    Small,
    #[default]
    Medium,
    Large,
}

impl Size {
    pub const ALL: [Self; 3] = [Self::Small, Self::Medium, Self::Large];

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.id().eq_ignore_ascii_case(id))
    }

    pub fn id(self) -> &'static str {
        match self {
            Self::Small => "small",
            Self::Medium => "medium",
            Self::Large => "large",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Small => "Small",
            Self::Medium => "Medium",
            Self::Large => "Large",
        }
    }

    pub fn font_size(self) -> f32 {
        match self {
            Self::Small => 28.0,
            Self::Medium => 44.0,
            Self::Large => 68.0,
        }
    }
}

// Stored as their lowercase ids rather than variant names: RON drops the name
// of a unit variant when a value is read untyped, which is exactly what the
// field-by-field config loader does.
macro_rules! serde_by_id {
    ($ty:ty, $what:literal) => {
        impl Serialize for $ty {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_str(self.id())
            }
        }

        impl<'de> Deserialize<'de> for $ty {
            fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let id = String::deserialize(deserializer)?;
                Self::from_id(&id)
                    .ok_or_else(|| serde::de::Error::custom(format!("unknown {} '{id}'", $what)))
            }
        }
    };
}
pub(crate) use serde_by_id;
serde_by_id!(Mode, "mode");
serde_by_id!(Size, "size");

/// Runtime state. `locked` is deliberately never persisted: the app always
/// starts interactive so it can never come up unreachable.
pub struct ChronoApp {
    settings: Config,
    /// Last state written to disk, so saving only happens on real changes.
    saved: Config,
    config_path: Option<PathBuf>,
    /// Set when `settings` differs from `saved`; writes are delayed so dragging
    /// the window doesn't hit the disk on every frame.
    dirty_since: Option<Instant>,
    locked: bool,
    stopwatch: Stopwatch,
    countdown: Countdown,
    alarm: Alarm,
    /// Chimes actually played, for the instrumentation.
    chimes: u32,
    tray: Tray,
    instrument: Instrument,
    /// The theme drawn with this frame: palette plus any night dimming.
    /// Rebuilt every frame from a few colour multiplies; nothing caches on it.
    theme: Theme,
    /// Cached layout: rebuilt on size or scale changes, not per frame.
    layout: DerivedLayout,
    /// Last size requested from the OS, to avoid resize commands every frame.
    window_size: Option<Vec2>,
    window_styled: bool,
    /// The position as it came out of `app.ron`, kept apart from `settings`
    /// because `persist` overwrites that one with wherever the window actually is.
    saved_position: Option<config::WindowPos>,
    /// Frames the placement check may still be put off while the display scale
    /// settles; `None` once it has run. Counted down rather than waited on, so
    /// a scale that never agrees costs a few frames instead of looping forever.
    placement_pending: Option<u8>,
    /// A position we asked the OS for, in **physical pixels**, until the window
    /// reports that it got there. Physical because a move to a screen with
    /// another scaling factor changes what a point is worth, and a point-space
    /// comparison would then never match.
    pending_move: Option<(Pos2, Instant)>,
    wheel: f32,
    autostart: Autostart,
    /// This exe, as the `Run` key would name it; `None` leaves the menu item disabled.
    exe: Option<PathBuf>,
    /// The registry's answer and when it was read.
    autostart_on: (bool, Instant),
    /// The welcome card is up instead of the readout.
    welcome: bool,
    /// Word from later launches: show yourself, or quit for the installer.
    signals: Receiver<Signal>,
    /// Since when the overlay is outlined because another launch asked for it.
    attention: Option<Instant>,
}

/// How long to wait after the last change before writing the config file.
/// How stale the menu's autostart check mark may get. The registry can change
/// behind the app's back (Settings, Task Manager); it is re-read on frames that
/// are drawn anyway, never by waking up for it.
const AUTOSTART_RECHECK: Duration = Duration::from_secs(10);

const SAVE_DELAY: Duration = Duration::from_millis(1500);

impl ChronoApp {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        loaded: config::Loaded,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        for warning in &loaded.warnings {
            eprintln!("ChronoDesk: config: {warning}");
        }
        if let Some(backup) = &loaded.quarantined {
            eprintln!("ChronoDesk: unusable config quarantined at {}", backup.display());
        }
        // Anything the loader had to repair is written back, so a bad value is
        // cleaned up instead of being re-reported on every launch.
        let repaired = !loaded.warnings.is_empty() || loaded.quarantined.is_some() || loaded.migrated;
        let settings = loaded.config;

        install_display_font(&cc.egui_ctx);
        // A counter that was under way picks up where the wall clock says it is.
        let (now, wall) = (Instant::now(), SystemTime::now());
        let duration = minutes(settings.timer_minutes);
        let stopwatch = settings.stopwatch.map_or_else(Stopwatch::default, |saved| Stopwatch::restore(saved, now, wall));
        let countdown = settings
            .countdown
            .map_or_else(|| Countdown::new(duration), |saved| Countdown::restore(duration, saved, now, wall));

        let autostart = Autostart::system();
        let exe = std::env::current_exe().ok().filter(|_| Autostart::available());
        let autostart_on = (exe.as_deref().is_some_and(|exe| autostart.enabled(exe)), Instant::now());
        if !settings.always_on_top {
            cc.egui_ctx.send_viewport_cmd(ViewportCommand::WindowLevel(WindowLevel::Normal));
        }

        let config_path = config::config_path();
        let (tx, signals) = mpsc::channel();
        if let Some(path) = &config_path {
            let ctx = cc.egui_ctx.clone();
            let window = native_window(cc);
            signal::listen(path, move |signal| {
                // A minimised window draws no frames, so nothing sent to the
                // app would be looked at: it is brought back from here first.
                if signal == Signal::Show {
                    restore_if_minimised(window);
                }
                let _ = tx.send(signal);
                ctx.request_repaint();
            });
        }
        let instrument = Instrument::start(&cc.egui_ctx);

        Ok(Self {
            welcome: settings.first_run,
            signals,
            attention: None,
            countdown,
            saved: settings.clone(),
            saved_position: settings.window,
            placement_pending: Some(PLACEMENT_DEFERRALS),
            pending_move: None,
            settings,
            config_path,
            dirty_since: repaired.then(Instant::now),
            locked: false,
            stopwatch,
            alarm: Alarm::default(),
            chimes: 0,
            tray: Tray::new(&cc.egui_ctx, !instrument.active()),
            instrument,
            layout: DerivedLayout::new(&Theme::default()),
            theme: Theme::default(),
            window_size: None,
            window_styled: false,
            wheel: 0.0,
            autostart,
            exe,
            autostart_on,
        })
    }

    /// Notes the current window position and writes the config once it has been
    /// unchanged for [`SAVE_DELAY`].
    fn persist(&mut self, ctx: &egui::Context, now: Instant) {
        // While a move of ours is in flight the window still reports the old
        // position, and recording it would undo the move in the file.
        if self.placement_settled(ctx, now)
            && let Some(rect) = ctx.input(|i| i.viewport().outer_rect)
        {
            let pos = config::WindowPos { x: rect.min.x, y: rect.min.y };
            if self.settings.window != Some(pos) {
                self.settings.window = Some(pos);
            }
        }

        if self.settings != self.saved && self.dirty_since.is_none() {
            self.dirty_since = Some(now);
        }
        if let Some(since) = self.dirty_since {
            if now.duration_since(since) >= SAVE_DELAY {
                self.write_config();
            } else {
                ctx.request_repaint_after(SAVE_DELAY);
            }
        }
    }

    fn write_config(&mut self) {
        self.dirty_since = None;
        let Some(path) = &self.config_path else { return };
        match config::save(path, &self.settings) {
            Ok(()) => self.saved = self.settings.clone(),
            Err(err) => eprintln!("ChronoDesk: could not save config: {err}"),
        }
    }

    /// True once the window sits where we last asked it to, so its reported
    /// position can be trusted again.
    fn placement_settled(&mut self, ctx: &egui::Context, now: Instant) -> bool {
        let Some((target, since)) = self.pending_move else {
            return true;
        };
        let ppp = ctx.pixels_per_point();
        let arrived = ctx.input(|i| i.viewport().outer_rect).is_some_and(|rect| {
            (rect.min.to_vec2() * ppp - target.to_vec2()).length() <= ARRIVAL_TOLERANCE_PX
        });
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
    fn check_placement(&mut self, ctx: &egui::Context, frame: &eframe::Frame, size: Vec2, now: Instant) {
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
        let saved_px = pos2(saved.x * ppp, saved.y * ppp);
        let outcome = placement::evaluate(saved_px, size, zoom, &monitors);

        let decision = match outcome {
            placement::Outcome::Keep => "keep".to_owned(),
            placement::Outcome::Move { to, reason } => {
                let to_pt = pos2(to.x / ppp, to.y / ppp);
                ctx.send_viewport_cmd(ViewportCommand::OuterPosition(to_pt));
                self.settings.window = Some(config::WindowPos { x: to_pt.x, y: to_pt.y });
                self.pending_move = Some((to, now));
                // Straight to disk: the point of the exercise is that the next
                // launch starts from a position that exists.
                self.write_config();
                eprintln!(
                    "ChronoDesk: saved position ({:.0},{:.0}) is {}; moved to ({:.0},{:.0})",
                    saved.x,
                    saved.y,
                    reason.as_str(),
                    to_pt.x,
                    to_pt.y,
                );
                ctx.request_repaint();
                format!("{} moved_to_pt=({:.1},{:.1}) moved_to_px=({:.0},{:.0})", reason.as_str(), to_pt.x, to_pt.y, to.x, to.y)
            }
        };
        self.instrument.set_placement_report(format!(
            "decision={decision} ppp={ppp:.3} zoom={zoom:.3} size_pt=({:.1},{:.1}) \
             saved_pt=({:.1},{:.1}) saved_px=({:.0},{:.0}) monitors=[{}]",
            size.x,
            size.y,
            saved.x,
            saved.y,
            saved_px.x,
            saved_px.y,
            placement::describe(&monitors),
        ));
    }

    /// Mirrors the two counters into the settings and writes them out at once:
    /// a logout or a power cut gives no notice, and the whole point is that a
    /// running timer survives one. Called only after a command that touched a
    /// counter, never per frame. That is enough, because a running counter is
    /// saved as the moment it started, which does not change while it runs.
    fn save_counters(&mut self, now: Instant) {
        let wall = SystemTime::now();
        let counters = (self.stopwatch.save(now, wall), self.countdown.save(now, wall));
        if counters != (self.settings.stopwatch, self.settings.countdown) {
            (self.settings.stopwatch, self.settings.countdown) = counters;
            self.write_config();
        }
    }

    /// The card has done its job the moment the user does anything at all, and
    /// that is written down at once so it is never shown twice.
    fn dismiss_welcome(&mut self) {
        if self.welcome {
            self.welcome = false;
            if self.settings.first_run {
                self.settings.first_run = false;
                self.write_config();
            }
        }
    }

    /// Another launch found this one running. An overlay has no taskbar button
    /// to flash, so it takes the focus, wears an outline for a few seconds, and
    /// has its position checked again, which brings it back if the screen it
    /// was on has gone.
    fn surface(&mut self, ctx: &egui::Context, now: Instant) {
        ctx.send_viewport_cmd(ViewportCommand::Focus);
        self.attention = Some(now);
        self.saved_position = self.settings.window;
        self.placement_pending = Some(PLACEMENT_DEFERRALS);
    }

    fn apply(&mut self, cmd: Command, ctx: &egui::Context, now: Instant) {
        // Whoever reaches for the menu or a key has found their way in.
        if cmd != Command::ShowWelcome {
            self.dismiss_welcome();
        }
        let counters_touched = matches!(cmd, Command::StartPause | Command::Reset | Command::SetTimerMinutes(_));
        let s = &mut self.settings;
        match cmd {
            Command::ToggleLock => {
                self.locked = !self.locked && self.tray.lock_available();
                ctx.send_viewport_cmd(ViewportCommand::MousePassthrough(self.locked));
            }
            Command::SetMode(mode) => s.mode = mode,
            Command::SetTimerMinutes(min) => {
                s.timer_minutes = min.clamp(1, 24 * 60);
                s.mode = Mode::Timer;
                self.countdown.set_duration(minutes(s.timer_minutes));
            }
            Command::StartPause => match s.mode {
                Mode::Clock | Mode::Market => {}
                Mode::Stopwatch => self.stopwatch.toggle(now),
                Mode::Timer => self.countdown.toggle(now),
            },
            Command::Reset => match s.mode {
                Mode::Clock | Mode::Market => {}
                Mode::Stopwatch => self.stopwatch.reset(),
                Mode::Timer => self.countdown.reset(),
            },
            Command::ToggleMarket(market) => toggle_market(&mut s.markets, market),
            Command::ToggleBoardLayout => s.board_layout = s.board_layout.toggled(),
            Command::ToggleBoardLabels => s.board_labels = s.board_labels.toggled(),
            Command::ToggleRing => s.seconds_ring = !s.seconds_ring,
            Command::ToggleTimerSound => s.timer_sound = !s.timer_sound,
            Command::SetSize(size) => s.size = size,
            Command::ToggleFont => s.font = s.font.toggled(),
            Command::SetFont(font) => s.font = font,
            Command::ToggleBackdrop => s.backdrop = !s.backdrop,
            Command::ToggleOutline => s.text_outline = !s.text_outline,
            Command::ToggleChroma => s.chroma = !s.chroma,
            Command::ToggleSeconds => s.show_seconds = !s.show_seconds,
            Command::ToggleClockFormat => s.clock_format = s.clock_format.toggled(),
            Command::ToggleDate => s.show_date = !s.show_date,
            Command::SetPalette(palette) => s.palette = palette,
            Command::SetNight(night) => s.night = night,
            Command::ToggleOnTop => {
                s.always_on_top = !s.always_on_top;
                let level = if s.always_on_top { WindowLevel::AlwaysOnTop } else { WindowLevel::Normal };
                ctx.send_viewport_cmd(ViewportCommand::WindowLevel(level));
            }
            Command::ToggleAutostart => self.toggle_autostart(now),
            Command::ShowWelcome => self.welcome = true,
            Command::Quit => ctx.send_viewport_cmd(ViewportCommand::Close),
        }
        if counters_touched {
            self.save_counters(now);
        }
        // The OS flips check items on click; re-assert our state.
        self.tray.invalidate();
        // Commands can arrive after this frame's readout was taken.
        ctx.request_repaint();
    }

    fn keyboard_commands(&self, ctx: &egui::Context) -> Vec<Command> {
        use egui::Key;
        ctx.input(|i| {
            [
                (Key::Space, Command::StartPause),
                (Key::R, Command::Reset),
                (Key::Num1, Command::SetMode(Mode::Clock)),
                (Key::Num2, Command::SetMode(Mode::Stopwatch)),
                (Key::Num3, Command::SetMode(Mode::Timer)),
                (Key::Num4, Command::SetMode(Mode::Market)),
            ]
            .into_iter()
            .filter(|(key, _)| i.key_pressed(*key))
            .map(|(_, cmd)| cmd)
            .collect()
        })
    }

    /// Scrolling over an idle timer nudges its duration by a minute per notch.
    fn scroll_timer(&mut self, ctx: &egui::Context) -> Option<Command> {
        if self.settings.mode != Mode::Timer || !self.countdown.is_idle() {
            self.wheel = 0.0;
            return None;
        }
        self.wheel += ctx.input(|i| {
            i.events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::MouseWheel { unit, delta, .. } => Some(match unit {
                        egui::MouseWheelUnit::Point => delta.y / 50.0,
                        egui::MouseWheelUnit::Line | egui::MouseWheelUnit::Page => delta.y,
                    }),
                    _ => None,
                })
                .sum::<f32>()
        });
        let steps = self.wheel.trunc();
        if steps == 0.0 {
            return None;
        }
        self.wheel -= steps;
        let min = (self.settings.timer_minutes as i64 + steps as i64).clamp(1, 24 * 60);
        Some(Command::SetTimerMinutes(min as u64))
    }

    fn menu_state(&self, now: Instant) -> MenuState {
        let s = &self.settings;
        let start_label = match s.mode {
            Mode::Clock => "Start",
            Mode::Stopwatch if self.stopwatch.is_running() => "Pause",
            Mode::Timer if self.countdown.is_finished(now) => "Restart",
            Mode::Timer if self.countdown.is_running(now) => "Pause",
            _ => "Start",
        };
        MenuState {
            locked: self.locked,
            lock_available: self.tray.lock_available(),
            mode: s.mode,
            timer_minutes: s.timer_minutes,
            start_label,
            size: s.size,
            font: s.font,
            backdrop: s.backdrop,
            text_outline: s.text_outline,
            chroma: s.chroma,
            show_seconds: s.show_seconds,
            clock_format: s.clock_format,
            show_date: s.show_date,
            palette: s.palette,
            night: s.night,
            markets: s.markets.clone(),
            board_layout: s.board_layout,
            board_labels: s.board_labels,
            seconds_ring: s.seconds_ring,
            timer_sound: s.timer_sound,
            always_on_top: s.always_on_top,
            autostart: self.autostart_on.0,
            autostart_available: self.exe.is_some(),
        }
    }

    /// Flips from the check mark the user was looking at, not from a fresh read:
    /// the tray menu opens without a frame, so after hours asleep the mark can be
    /// stale, and a click must still do what it appeared to offer. The registry
    /// is read back afterwards, so the mark ends up telling the truth either way.
    fn toggle_autostart(&mut self, now: Instant) {
        let Some(exe) = self.exe.as_deref() else { return };
        let on = !self.autostart_on.0;
        if let Err(err) = self.autostart.set(exe, on) {
            eprintln!("ChronoDesk: could not update the startup entry: {err}");
        }
        self.autostart_on = (self.autostart.enabled(exe), now);
    }

    fn recheck_autostart(&mut self, now: Instant) {
        if let Some(exe) = self.exe.as_deref()
            && now.duration_since(self.autostart_on.1) >= AUTOSTART_RECHECK
        {
            self.autostart_on = (self.autostart.enabled(exe), now);
        }
    }

    /// Resolves the theme for this frame and returns how long night mode can
    /// be left alone before it has to be looked at again (`Auto` only).
    fn apply_night(&mut self, local: DateTime<Local>) -> Option<Duration> {
        let s = &self.settings;
        let schedule = Schedule { from: s.night_from, to: s.night_to };
        let (dim, wake) = night::resolve(s.night, schedule, s.night_dim, local.time());
        self.theme = Theme::resolve(s.palette, dim, s.chroma);
        wake
    }

    /// What to draw above the caption line, the caption, the seconds ring's
    /// position, and when the display next changes.
    ///
    /// The clock is drawn in the primary colour, the counters in the
    /// secondary one; they only differ in a dual-colour preset.
    fn readout(&self, now: Instant, local: DateTime<Local>) -> Readout {
        if self.welcome {
            // Nothing on the card changes, so nothing is scheduled: it costs
            // no frames while it waits to be read.
            return Readout { scene: Scene::Card, caption: welcome::DISMISS.to_owned(), next_change: None, ring: None };
        }
        let s = &self.settings;
        let c = &self.theme.color;
        let ring = s.seconds_ring;
        let line = |main: String, caption: String, color: Color32, next_change: Option<Duration>, lit: usize| Readout {
            scene: Scene::Line { main, color },
            caption,
            next_change,
            ring: ring.then_some(lit),
        };
        match s.mode {
            Mode::Clock => {
                let style = ClockStyle { format: s.clock_format, show_seconds: s.show_seconds, show_date: s.show_date };
                let clock = clock_readout(local, style);
                // The ring moves every second even when the digits do not.
                let mut next = clock.until_change;
                if ring {
                    let into_second = u64::from(local.nanosecond() % 1_000_000_000);
                    next = next.min(Duration::from_nanos(1_000_000_000 - into_second));
                }
                line(clock.main, clock.caption, c.text, Some(next), ring::lit(local.second()))
            }
            Mode::Market => {
                let style = BoardStyle { format: s.clock_format, show_seconds: s.show_seconds, labels: s.board_labels };
                let board = market::board(local.naive_utc(), &s.markets, style);
                Readout {
                    scene: Scene::Board(board.rows),
                    caption: board.caption,
                    next_change: Some(board.until_change),
                    ring: None,
                }
            }
            Mode::Stopwatch => {
                let elapsed = self.stopwatch.elapsed(now);
                let running = self.stopwatch.is_running();
                line(
                    timer::format_stopwatch(elapsed),
                    if running || elapsed.is_zero() { "STOPWATCH" } else { "PAUSED" }.into(),
                    c.secondary,
                    running.then(|| timer::next_stopwatch_tick(elapsed)),
                    ring::lit((elapsed.as_secs() % 60) as u32),
                )
            }
            Mode::Timer => {
                let cd = &self.countdown;
                let remaining = cd.remaining(now);
                if let Some(over) = cd.overtime(now) {
                    let blinking = over < BLINK_FOR;
                    let lit = !blinking || over.as_millis() % 1000 < 600;
                    let into = (over.as_millis() % 1000) as u64;
                    let next = if into < 600 { 600 - into } else { 1000 - into };
                    line(
                        timer::format_countdown(remaining),
                        "TIME'S UP".into(),
                        if lit { c.alert } else { c.alert.gamma_multiply(0.25) },
                        blinking.then(|| Duration::from_millis(next)),
                        0,
                    )
                } else {
                    let running = cd.is_running(now);
                    let caption = if running {
                        "TIMER".to_owned()
                    } else if cd.is_idle() {
                        format!("TIMER · {} MIN", cd.duration().as_secs() / 60)
                    } else {
                        "PAUSED".to_owned()
                    };
                    // The ring drains with the digits: the seconds left in
                    // the current minute, rounded up like the readout.
                    let seconds_left = remaining.as_nanos().div_ceil(1_000_000_000) as u64;
                    line(
                        timer::format_countdown(remaining),
                        caption,
                        c.secondary,
                        running.then(|| timer::next_countdown_tick(remaining)),
                        ring::lit_remaining(seconds_left),
                    )
                }
            }
        }
    }
}

/// What sits above the caption line.
enum Scene {
    /// One big readout line.
    Line { main: String, color: Color32 },
    /// The market board, one row per exchange.
    Board(Vec<market::Row>),
    /// The welcome card.
    Card,
}

struct Readout {
    scene: Scene,
    caption: String,
    next_change: Option<Duration>,
    /// How many LEDs of the studio ring are lit, when it is on.
    ring: Option<usize>,
}

/// Adds or removes a market, keeping the list in catalogue order around any
/// order the user set by hand, and never emptying it: a board with no rows
/// would be a mode that shows nothing.
fn toggle_market(markets: &mut Vec<Market>, market: Market) {
    if let Some(i) = markets.iter().position(|&m| m == market) {
        if markets.len() > 1 {
            markets.remove(i);
        }
        return;
    }
    // After the last row that precedes it in the catalogue: a list in
    // catalogue order stays in catalogue order, and a hand-set order is
    // disturbed as little as possible.
    let rank = |m: Market| Market::ALL.iter().position(|&x| x == m).unwrap_or(usize::MAX);
    let at = markets.iter().rposition(|&m| rank(m) < rank(market)).map_or(0, |i| i + 1);
    markets.insert(at, market);
}

impl eframe::App for ChronoApp {
    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        // Synthetic pointer events go in here so hit-testing, hovering and
        // clicking take exactly the same path as a real mouse.
        self.instrument.inject(raw_input);
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let frame_start = Instant::now();
        let ctx = ui.ctx().clone();
        let now = frame_start;
        if !self.window_styled {
            self.window_styled = true;
            strip_window_chrome(frame);
        }

        let mut commands: Vec<Command> = self.tray.commands().collect();
        commands.extend(self.instrument.commands());
        if !self.locked {
            commands.extend(self.keyboard_commands(&ctx));
        }
        let had_commands = !commands.is_empty();
        for cmd in commands {
            self.apply(cmd, &ctx, now);
        }
        for signal in self.signals.try_iter().collect::<Vec<_>>() {
            match signal {
                Signal::Show => self.surface(&ctx, now),
                Signal::Quit => ctx.send_viewport_cmd(ViewportCommand::Close),
            }
        }

        let rect = ui.max_rect();
        let background = ui.allocate_rect(rect, Sense::click_and_drag());
        // Any key will do, not only the ones that mean something.
        let key_pressed = !self.locked && ctx.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Key { pressed: true, .. })));
        if background.clicked() || key_pressed {
            self.dismiss_welcome();
        }
        let hovered = !self.locked && ctx.input(|i| i.pointer.hover_pos()).is_some_and(|p| rect.contains(p));
        if hovered && let Some(cmd) = self.scroll_timer(&ctx) {
            self.apply(cmd, &ctx, now);
        }

        let mut s = self.settings.clone();
        // The card is text to be read, whatever is behind the overlay.
        s.backdrop |= self.welcome;
        // One reading of the wall clock per frame, shared by the readout and
        // the night schedule so they can never disagree about the time.
        let local = Local::now();
        let night_wake = self.apply_night(local);
        let readout = self.readout(now, local);
        let painter = ui.painter().clone();

        // Rebuilt only when the size, face or display scale changes; every
        // frame in between reads cached numbers. Only the typeface touches
        // the font atlas; the digital face is pure geometry.
        let theme = self.theme;
        let key = LayoutKey::new(s.size, ctx.pixels_per_point(), s.font);
        self.layout.ensure(key, &theme, |font| match s.font {
            Font::Sans => measure_glyphs(&ctx, font),
            Font::Digital => digital::glyphs(font, &theme.segments),
            Font::Matrix => matrix::glyphs(font, &theme.matrix),
        });
        let m = *self.layout.metrics();
        // A backdrop already separates the text from whatever is behind it,
        // and under a chroma key a dark rim would only leave a fringe once
        // the green is keyed out (requirement T3).
        let halo = (s.text_outline && !s.backdrop && !s.chroma).then_some(m.halo_width);

        // Every small label — the caption, city names, AM/PM — is set the
        // same way: caption size, letter-spaced, in the typeface.
        let caption_font = FontId::new(m.caption_font, display_family());
        let lay = |text: &str| painter.layout_no_wrap(spaced(text), caption_font.clone(), theme.color.text);
        // The board's printed labels: bold, larger, not letter-spaced.
        let label_font = FontId::new(m.label_font, label_family());
        let lay_label = |text: &str| painter.layout_no_wrap(text.to_owned(), label_font.clone(), theme.color.label);
        let faces = board::Faces { caption: &lay, board: &lay_label };

        // An empty caption (clock without date) gives its line back to the
        // window; the hover controls only exist outside clock mode, so
        // nothing else needs that line.
        let caption_galley = (!readout.caption.is_empty())
            .then(|| self.layout.caption_galley(&readout.caption, m.caption_font, || lay(&readout.caption)));
        let caption_width = caption_galley.as_ref().map_or(0.0, |g| g.size().x);
        let caption_height = if caption_galley.is_some() { m.caption_height } else { 0.0 };

        // The board reserves the caption line for the longest countdown it
        // can show, so its window is sized once rather than every minute.
        let (scene_size, geometry) = match &readout.scene {
            Scene::Line { main, .. } => (vec2(self.layout.width_of(main), self.layout.glyphs().height), None),
            Scene::Board(rows) => {
                let geo = board::measure(&mut self.layout, rows, s.board_layout, s.board_labels, &faces);
                (vec2(geo.size.x.max(geo.caption_min_width), geo.size.y), Some(geo))
            }
            Scene::Card => (Vec2::ZERO, None),
        };
        let card = matches!(readout.scene, Scene::Card).then(|| Card::lay_out(&painter, &theme));
        let scene_size = card.as_ref().map_or(scene_size, |card| card.size);
        let theme = &theme;

        // The seconds ring runs just inside the window edge, so the content
        // moves in by the band it needs.
        let inset = if readout.ring.is_some() { m.ring_band } else { 0.0 };
        let content = vec2(scene_size.x.max(caption_width), scene_size.y + caption_height);
        let window_size = (content + (m.pad + Vec2::splat(inset)) * 2.0).ceil();
        self.fit_window(&ctx, window_size);

        // Now that the real size is known, the saved position can be judged
        // against the monitors that are actually attached.
        if let Some(synthetic) = self.instrument.take_place_request() {
            self.saved_position = Some(config::WindowPos { x: synthetic.x, y: synthetic.y });
            self.placement_pending = Some(PLACEMENT_DEFERRALS);
        }
        if self.placement_pending.is_some() {
            self.check_placement(&ctx, frame, window_size, now);
        }

        if s.backdrop {
            painter.rect_filled(rect, m.corner, theme.color.backdrop);
        }
        if self.welcome {
            // The usual backdrop lets the desktop through, which suits four
            // digits and not eight lines of prose.
            painter.rect_filled(rect, m.corner, Color32::from_black_alpha(190));
        }
        if hovered && !s.backdrop {
            let stroke = Stroke::new(1.0, theme.color.hover_frame);
            painter.rect_stroke(rect.shrink(0.5), m.corner, stroke, StrokeKind::Inside);
        }
        // "Here I am", for a few seconds after another launch asked.
        let attention_wake = self.attention.and_then(|since| ATTENTION_FOR.checked_sub(now.duration_since(since)));
        if attention_wake.is_some() {
            painter.rect_stroke(rect.shrink(1.0), m.corner, Stroke::new(2.0, theme.color.text), StrokeKind::Inside);
        } else {
            self.attention = None;
        }

        // Dimming over a halo would eat the contrast the halo just bought, so
        // captions and closed rows are only faded when there is none; and a
        // chroma key wants opaque text, since anything translucent keys as a
        // green tint.
        let dim_captions = halo.is_none() && !s.chroma;
        // Translucent hardware dressing — unlit diodes, sockets, hairlines —
        // keys as a tint, so none of it is drawn over a chroma key.
        let dressing = !s.chroma;
        // Unlit segments under the digital face, at the ghost alpha whatever
        // the readout colour has been dimmed to.
        let ghost = (dressing && s.font.has_ghost()).then(|| theme.ghost(theme.color.text));
        if let Some(lit) = readout.ring {
            paint_ring(&painter, rect, &m, theme, lit, halo, dressing);
        }
        let inner = rect.shrink(inset);
        let scene_top = inner.top() + m.pad.y;
        let caption_color = match &readout.scene {
            Scene::Line { main, color } => {
                let origin = pos2(inner.center().x - scene_size.x / 2.0, scene_top);
                paint_readout(&painter, origin, main, self.layout.glyphs(), *color, halo, ghost, theme);
                // A plain readout colour dims with its caption; an alarm does not.
                let plain = *color == theme.color.text || *color == theme.color.secondary;
                if plain && dim_captions { theme.dim_caption(*color) } else { *color }
            }
            Scene::Board(rows) => {
                let geo = geometry.expect("measured with the board");
                let origin = pos2(inner.center().x - geo.size.x / 2.0, scene_top);
                let style = board::Style { halo, dim_closed: dim_captions, dressing, ghost };
                board::paint(&painter, origin, rows, &geo, &mut self.layout, theme, style, &faces);
                if dim_captions { theme.dim_caption(theme.color.text) } else { theme.color.text }
            }
            Scene::Card => {
                let card = card.expect("laid out with the card");
                card.paint(&painter, pos2(inner.center().x - card.size.x / 2.0, scene_top));
                theme.color.text
            }
        };

        let caption_rect = Rect::from_min_size(
            pos2(inner.left(), scene_top + scene_size.y),
            vec2(inner.width(), caption_height),
        );
        let show_controls = hovered && s.mode.has_controls() && !self.welcome;
        let buttons = show_controls.then(|| control_rects(caption_rect, &m));
        let mut pending = None;
        if show_controls {
            pending = controls(ui, caption_rect, &m, theme, self.menu_state(now).start_label);
        } else if let Some(caption_galley) = caption_galley {
            let pos = caption_rect.center() - caption_galley.size() / 2.0;
            // The caption is far smaller, so it gets the thinnest ring that
            // still separates it from the background.
            paint_galley(&painter, pos, caption_galley, caption_color, halo.map(|_| 1.0), theme);
        }
        if let Some(cmd) = pending {
            self.apply(cmd, &ctx, now);
        }

        if background.drag_started_by(PointerButton::Primary) {
            ctx.send_viewport_cmd(ViewportCommand::StartDrag);
        }

        self.recheck_autostart(now);
        let state = self.menu_state(now);
        self.tray.sync(state);
        self.persist(&ctx, now);

        // Schedule from the readout actually drawn: sampling the clock again here
        // could straddle a boundary and leave a stale value up for a full period.
        // A scheduled night boundary is folded in the same way, so an idle
        // stopwatch still sleeps until the next thing that changes the picture.
        // The countdown is heard in whatever mode is showing, so its finish is a
        // wake of its own: behind a paused stopwatch nothing else would draw then.
        // The bookkeeping runs with the sound off too, so switching it on halfway
        // through a finish does not replay what was skipped; switched on after the
        // blink window, `due` owes nothing at all.
        let chime = self.alarm.due(self.countdown.overtime(now)) && self.settings.timer_sound;
        if chime {
            chime::play();
            self.chimes += 1;
        }
        let alarm_wake = self.settings.timer_sound.then(|| self.alarm.next_in(&self.countdown, now)).flatten();
        let wake = [readout.next_change, night_wake, alarm_wake, attention_wake].into_iter().flatten().min();
        if let Some(next) = wake {
            ctx.request_repaint_after(next + WAKE_SLACK);
        }

        if self.instrument.click_pending() {
            // Keep drawing until the synthetic release has been delivered.
            ctx.request_repaint();
        }
        let cause = if had_commands {
            Cause::Config
        } else if ctx.input(|i| !i.events.is_empty()) {
            Cause::Input
        } else if readout.next_change.is_some() {
            Cause::Tick
        } else {
            Cause::Other
        };
        self.instrument.record_frame(frame_start.elapsed(), cause, self.layout.rebuilds(), self.chimes);
        self.instrument.report_state(|| {
            // Where the hover buttons are, in points, so a test clicks what is
            // drawn instead of guessing at the layout.
            let buttons = buttons.map_or_else(
                || "controls=0".to_owned(),
                |[(start, _), (reset, _)]| {
                    let (a, b) = (start.center(), reset.center());
                    format!("controls=1 start_x={:.1} start_y={:.1} reset_x={:.1} reset_y={:.1}", a.x, a.y, b.x, b.y)
                },
            );
            format!(
                "mode={} welcome={} attention={} win_w={:.1} win_h={:.1} sw_running={} sw_ms={} cd_running={} cd_finished={} cd_remaining_ms={} {buttons}",
                self.settings.mode.id(),
                u8::from(self.welcome),
                u8::from(self.attention.is_some()),
                rect.width(),
                rect.height(),
                u8::from(self.stopwatch.is_running()),
                self.stopwatch.elapsed(now).as_millis(),
                u8::from(self.countdown.is_running(now)),
                u8::from(self.countdown.is_finished(now)),
                self.countdown.remaining(now).as_millis(),
            )
        });

        // Last: this blocks in a native modal loop until the menu closes.
        if background.secondary_clicked() {
            self.tray.show_context_menu(frame);
        }
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        if self.settings.chroma {
            self.theme.color.chroma.to_normalized_gamma_f32()
        } else {
            [0.0; 4]
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if self.settings != self.saved {
            self.write_config();
        }
    }

    fn persist_egui_memory(&self) -> bool {
        false
    }
}

impl ChronoApp {
    fn fit_window(&mut self, ctx: &egui::Context, size: Vec2) {
        if self.window_size != Some(size) {
            self.window_size = Some(size);
            ctx.send_viewport_cmd(ViewportCommand::InnerSize(size));
        }
    }
}

/// The studio seconds ring: sixty discrete LEDs along the window's outline,
/// the first `lit` of them on, clockwise from the top.
///
/// Every LED sits in a dark socket, so the unlit ones read as a faint dark
/// glow with a whisper of the LED colour, the way an off diode does. The one
/// that lit last blooms a little brighter than the rest, and the four
/// quarter positions carry marker LEDs in their own colour whether lit or
/// not. With `dressing` off (chroma key) only the lit LEDs are drawn.
fn paint_ring(painter: &egui::Painter, rect: Rect, m: &Metrics, theme: &Theme, lit: usize, halo: Option<f32>, dressing: bool) {
    let track = rect.shrink(m.ring_band / 2.0);
    let corner = (m.corner - m.ring_band / 2.0).max(0.0);
    let c = &theme.color;
    let r = m.led_radius;
    let (on, marker) = (c.secondary, c.ring_marker);
    let socket = Color32::from_black_alpha(c.ring_socket);
    let shade = Color32::from_black_alpha(c.halo_stroke);
    for (i, centre) in ring::dots(track, corner).into_iter().enumerate() {
        let is_marker = ring::is_marker(i);
        let colour = if is_marker { marker } else { on };
        let radius = if is_marker { r * 1.25 } else { r };
        if dressing {
            painter.circle_filled(centre, radius * 1.6, socket);
        } else if let Some(h) = halo {
            painter.circle_filled(centre, radius + h, shade);
        }
        if i < lit || is_marker {
            // The bloom stays within the socket, so it never smears onto the desktop.
            if i + 1 == lit {
                painter.circle_filled(centre, radius * 1.6, colour.gamma_multiply(f32::from(c.ring_bloom) / 255.0));
            }
            painter.circle_filled(centre, radius, colour);
        } else if dressing {
            painter.circle_filled(centre, radius, colour.gamma_multiply(f32::from(c.ring_unlit) / 255.0));
        }
    }
}

/// The welcome card, laid out: a title, then one row per tip with the key or
/// place in bold and what it does beside it. A fixed, readable type size
/// whatever size the clock is set to, since this is prose, not a readout.
struct Card {
    title: Arc<egui::Galley>,
    rows: Vec<(Arc<egui::Galley>, Arc<egui::Galley>)>,
    key_width: f32,
    size: Vec2,
}

impl Card {
    const TYPE: f32 = 13.0;
    const ROW: f32 = 21.0;
    const TITLE_ROW: f32 = 30.0;
    const GUTTER: f32 = 14.0;

    fn lay_out(painter: &egui::Painter, theme: &Theme) -> Self {
        let c = &theme.color;
        let title = painter.layout_no_wrap(spaced(welcome::TITLE), FontId::new(Self::TYPE - 1.0, display_family()), c.secondary);
        let rows: Vec<_> = welcome::ROWS
            .iter()
            .map(|(key, what)| {
                (
                    painter.layout_no_wrap((*key).to_owned(), FontId::new(Self::TYPE, label_family()), c.label),
                    painter.layout_no_wrap((*what).to_owned(), FontId::new(Self::TYPE, display_family()), c.text.gamma_multiply(0.85)),
                )
            })
            .collect();
        let key_width = rows.iter().map(|(key, _)| key.size().x).fold(0.0, f32::max);
        let text_width = rows.iter().map(|(_, what)| what.size().x).fold(0.0, f32::max);
        let width = (key_width + Self::GUTTER + text_width).max(title.size().x);
        let size = vec2(width, Self::TITLE_ROW + Self::ROW * rows.len() as f32);
        Self { title, rows, key_width, size }
    }

    fn paint(&self, painter: &egui::Painter, origin: Pos2) {
        painter.galley(origin, Arc::clone(&self.title), Color32::PLACEHOLDER);
        for (i, (key, what)) in self.rows.iter().enumerate() {
            let y = origin.y + Self::TITLE_ROW + Self::ROW * i as f32;
            // Keys right-aligned against the gutter, so the explanations line up.
            painter.galley(pos2(origin.x + self.key_width - key.size().x, y), Arc::clone(key), Color32::PLACEHOLDER);
            painter.galley(pos2(origin.x + self.key_width + Self::GUTTER, y), Arc::clone(what), Color32::PLACEHOLDER);
        }
    }
}

/// Where the two hover buttons sit within the caption line.
fn control_rects(area: Rect, metrics: &Metrics) -> [(Rect, Command); 2] {
    let (size, gap) = (metrics.control, metrics.control_gap);
    let center = area.center();
    [
        (Rect::from_center_size(center - vec2((size + gap) / 2.0, 0.0), Vec2::splat(size)), Command::StartPause),
        (Rect::from_center_size(center + vec2((size + gap) / 2.0, 0.0), Vec2::splat(size)), Command::Reset),
    ]
}

/// Start/pause and reset buttons, drawn as shapes so no icon font is needed.
fn controls(
    ui: &mut egui::Ui,
    area: Rect,
    metrics: &Metrics,
    theme: &Theme,
    start_label: &str,
) -> Option<Command> {
    let size = metrics.control;
    let mut clicked = None;
    for (rect, cmd) in control_rects(area, metrics) {
        // A stable id per button: `format!` here would allocate every frame.
        let id = ui.id().with(match cmd {
            Command::StartPause => "start_pause",
            _ => "reset",
        });
        let response = ui.interact(rect, id, Sense::click());
        let alpha = if response.hovered() {
            theme.color.control_glyph_hover
        } else {
            theme.color.control_glyph
        };
        let color = Color32::from_white_alpha(alpha);
        let painter = ui.painter();
        let disc = size * theme.ratio.control_disc;
        // Dark base keeps the glyphs legible over busy windows behind the overlay.
        painter.circle_filled(rect.center(), disc, theme.color.control_base);
        if response.hovered() {
            painter.circle_filled(rect.center(), disc, theme.color.control_hover);
        }
        let r = rect.shrink(size * theme.ratio.control_inset);
        match cmd {
            Command::StartPause if start_label == "Pause" => {
                let bar = vec2(r.width() * 0.3, r.height());
                painter.rect_filled(Rect::from_min_size(r.left_top(), bar), 1.0, color);
                painter.rect_filled(Rect::from_min_size(r.right_top() - vec2(bar.x, 0.0), bar), 1.0, color);
            }
            Command::StartPause => {
                let pts = vec![r.left_top() + vec2(r.width() * 0.1, 0.0), r.right_center(), r.left_bottom() + vec2(r.width() * 0.1, 0.0)];
                painter.add(egui::Shape::convex_polygon(pts, color, Stroke::NONE));
            }
            _ => {
                painter.rect_filled(r.shrink(r.width() * 0.08), 1.5, color);
            }
        }
        if response.clicked() {
            clicked = Some(cmd);
        }
    }
    clicked
}

/// The window's handle as a plain number, so another thread may hold it.
fn native_window(cc: &eframe::CreationContext<'_>) -> isize {
    use raw_window_handle::{HasWindowHandle as _, RawWindowHandle};
    match cc.window_handle().map(|handle| handle.as_raw()) {
        Ok(RawWindowHandle::Win32(handle)) => handle.hwnd.get(),
        _ => 0,
    }
}

/// Safe from any thread: `ShowWindow` posts to the window's own thread.
fn restore_if_minimised(window: isize) {
    #[cfg(windows)]
    if window != 0 {
        #[link(name = "user32")]
        unsafe extern "system" {
            fn IsIconic(window: isize) -> i32;
            fn ShowWindow(window: isize, command: i32) -> i32;
        }
        const SW_RESTORE: i32 = 9;
        // SAFETY: both take a window handle by value and tolerate a stale one.
        unsafe {
            if IsIconic(window) != 0 {
                ShowWindow(window, SW_RESTORE);
            }
        }
    }
    #[cfg(not(windows))]
    let _ = window;
}

/// Windows 11 draws a 1px border and rounds corners even on undecorated
/// windows, which shows up as a faint box around a transparent overlay.
#[cfg(windows)]
fn strip_window_chrome(frame: &eframe::Frame) {
    use winit::platform::windows::{CornerPreference, WindowExtWindows as _};
    if let Some(window) = frame.winit_window() {
        window.set_border_color(None);
        window.set_undecorated_shadow(false);
        window.set_corner_preference(CornerPreference::DoNotRound);
    }
}

#[cfg(not(windows))]
fn strip_window_chrome(_frame: &eframe::Frame) {}

fn minutes(min: u64) -> Duration {
    Duration::from_secs(min * 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mode_ids_round_trip() {
        for mode in Mode::ALL {
            assert_eq!(Mode::from_id(mode.id()), Some(mode));
        }
        assert_eq!(Mode::from_id("market"), Some(Mode::Market));
    }

    /// A menu toggle slots the exchange in at its catalogue position among
    /// whatever order the user set by hand, and removes it in place.
    #[test]
    fn toggling_a_market_keeps_the_users_order_around_it() {
        let mut markets = vec![Market::Tokyo, Market::NewYork, Market::Sydney];
        toggle_market(&mut markets, Market::London);
        assert_eq!(markets, vec![Market::Tokyo, Market::NewYork, Market::London, Market::Sydney]);
        toggle_market(&mut markets, Market::NewYork);
        assert_eq!(markets, vec![Market::Tokyo, Market::London, Market::Sydney]);
        toggle_market(&mut markets, Market::Oslo);
        assert_eq!(markets, vec![Market::Tokyo, Market::London, Market::Oslo, Market::Sydney]);
    }

    #[test]
    fn the_last_market_cannot_be_removed() {
        let mut markets = vec![Market::Oslo];
        toggle_market(&mut markets, Market::Oslo);
        assert_eq!(markets, vec![Market::Oslo]);
    }

    #[test]
    fn toggling_from_the_default_board_appends_in_catalogue_order() {
        let mut markets = Market::DEFAULT.to_vec();
        toggle_market(&mut markets, Market::HongKong);
        assert_eq!(
            markets,
            vec![Market::NewYork, Market::London, Market::Oslo, Market::HongKong, Market::Tokyo, Market::Sydney]
        );
    }
}
