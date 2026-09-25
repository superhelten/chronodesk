use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant, SystemTime};

use chrono::{DateTime, Local};
use eframe::egui::{
    self, Color32, FontId, PointerButton, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2,
    ViewportCommand, WindowLevel, pos2, vec2,
};

use crate::alarm;
use crate::autostart::Autostart;
use crate::board;
use crate::chime;
use crate::clock::ClockFormat;
use crate::config::{self, Config};
use crate::digital;
use crate::install;
use crate::instrument::{self, Cause, Instrument, Milestone};
use crate::layout::{DerivedLayout, Font, LayoutKey};
use crate::matrix;
use crate::pomodoro;
use crate::signal::{self, Signal};
use crate::text::{display_family, install_display_font, label_family, measure_glyphs, paint_galley, paint_readout, spaced};
use crate::theme::Theme;
use crate::timer::{Alarm, Countdown, Stopwatch};
use crate::tray::{Command, Tray};
use crate::update::{self, Version};

mod commands;
mod mode;
mod paint;
mod persist;
mod readout;
mod updates;
mod window;

pub use mode::{Mode, Size};
pub(crate) use mode::serde_by_id;
use paint::{Card, LineShape, WidthFloor, control_rects, controls, paint_ring};
use persist::{PLACEMENT_DEFERRALS, SavedPosition};
use readout::Scene;
use updates::UpdateStep;
use window::{native_window, restore_if_minimised, strip_window_chrome, window_position_px};

/// Timed wake-ups can fire more than one OS timer tick (~15.6 ms on Windows)
/// early. Waking just before a boundary would find "1 ms left" and spin at vsync
/// until it passes, so aim well past it; a 25 ms display lag is invisible.
const WAKE_SLACK: Duration = Duration::from_millis(25);
/// How long the overlay stays outlined after another launch asked for it.
const ATTENTION_FOR: Duration = Duration::from_secs(3);

/// Runtime state. `locked` is deliberately never persisted: the app always
/// starts interactive so it can never come up unreachable.
pub struct ChronoApp {
    settings: Config,
    /// Last state written to disk, or that failed to be, so saving only
    /// happens on real changes: a file that cannot be written would otherwise
    /// be retried on every wake-up, and keep an idle overlay drawing.
    saved: Config,
    /// The last write failed; quitting tries once more.
    save_failed: bool,
    /// When a failed write is tried once more. A scanner holding the file
    /// for a moment should not cost a counter that was just started; a file
    /// that stays unwritable gets this one retry and then waits for the next
    /// change, so it cannot keep the overlay awake.
    save_retry: Option<Instant>,
    config_path: Option<PathBuf>,
    /// Set when `settings` differs from `saved`; writes are delayed so dragging
    /// the window doesn't hit the disk on every frame.
    dirty_since: Option<Instant>,
    locked: bool,
    stopwatch: Stopwatch,
    countdown: Countdown,
    alarm: Alarm,
    /// The same three chimes for the alarm set in the menu.
    alarm_chimes: Alarm,
    /// Chimes due, for the instrumentation; played unless a script runs this.
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
    line_width: WidthFloor<LineShape>,
    window_styled: bool,
    /// The position as it came out of `app.ron`, kept apart from `settings`
    /// because `persist` overwrites that one with wherever the window actually is.
    saved_position: Option<SavedPosition>,
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
    /// A check for a newer release on its own thread, until it answers:
    /// the latest version, or `None` when it could not be found out.
    update_check: Option<Receiver<Option<Version>>>,
    /// After a failed check, when the next may go out.
    update_retry: Option<Instant>,
    /// Only the real overlay asks GitHub: not a script's, and not one being
    /// rendered off screen in a test.
    updates_allowed: bool,
    /// Where the update on offer stands; see [`UpdateStep`].
    update_step: UpdateStep,
    /// A download under way: the verified setup, or why there is none.
    update_download: Option<Receiver<Result<PathBuf, update::Refused>>>,
    /// This exe is the installed copy, which an update may replace.
    installed: bool,
    /// The welcome card is up instead of the readout.
    welcome: bool,
    /// Word from later launches: show yourself, or quit for the installer.
    signals: Receiver<Signal>,
    /// Since when the overlay is outlined because another launch asked for it.
    attention: Option<Instant>,
    /// A fixed wall clock for off-screen rendering; see [`Self::pinned`].
    #[cfg(test)]
    pinned: Option<DateTime<Local>>,
}

impl ChronoApp {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        loaded: config::Loaded,
        inbox: Option<signal::Inbox>,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        instrument::reached(Milestone::App);
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

        let config_path = config::config_path();
        // The file is there and holds the user's settings; it just could not be
        // read. This session's defaults must not be written over it.
        let writable_path = config_path.clone().filter(|_| !loaded.unreadable);
        let (tx, signals) = mpsc::channel();
        if let Some(inbox) = inbox {
            let ctx = cc.egui_ctx.clone();
            let window = native_window(cc);
            inbox.listen(move |signal| {
                // Someone is looking for the overlay, and a minimised one is
                // nowhere to be seen: it is brought back from here first.
                if signal == Signal::Show {
                    restore_if_minimised(window);
                }
                let _ = tx.send(signal);
                ctx.request_repaint();
            });
        }
        let instrument = Instrument::start(&cc.egui_ctx);
        let tray = Tray::new(&cc.egui_ctx, !instrument.active());
        let exe = std::env::current_exe().ok().filter(|_| Autostart::available());
        let updates_allowed = !instrument.active();
        if updates_allowed {
            // Setups earlier updates downloaded; off the UI thread, since the
            // temp folder can hold thousands of files.
            let _ = std::thread::Builder::new().name("update clean-up".to_owned()).spawn(update::clean_downloads);
        }
        let installed = std::env::current_exe().is_ok_and(|exe| install::is_installed(&exe));
        let mut app = Self::assemble(&cc.egui_ctx, settings, writable_path, signals, tray, instrument, exe);
        app.updates_allowed = updates_allowed;
        app.installed = installed;
        app.dirty_since = repaired.then(Instant::now);
        Ok(app)
    }

    /// Everything [`Self::new`] builds that does not reach outside the process:
    /// the config path, signal listener, tray icon and exe are decided by the caller.
    fn assemble(
        ctx: &egui::Context,
        mut settings: Config,
        config_path: Option<PathBuf>,
        signals: Receiver<Signal>,
        tray: Tray,
        instrument: Instrument,
        exe: Option<PathBuf>,
    ) -> Self {
        install_display_font(ctx);
        // An offer the running version has caught up with is withdrawn: the
        // user has updated since it was found.
        settings.update_available =
            settings.update_available.take().filter(|v| Version::parse(v).is_some_and(|v| v > Version::current()));
        // A counter that was under way picks up where the wall clock says it is.
        let (now, wall) = (Instant::now(), SystemTime::now());
        let duration = countdown_length(&settings);
        let stopwatch = settings.stopwatch.map_or_else(Stopwatch::default, |saved| Stopwatch::restore(saved, now, wall));
        let countdown = settings
            .countdown
            .map_or_else(|| Countdown::new(duration), |saved| Countdown::restore(duration, saved, now, wall));

        let autostart = Autostart::system();
        let autostart_on = (exe.as_deref().is_some_and(|exe| autostart.enabled(exe)), Instant::now());
        if !settings.always_on_top && !instrument.active() {
            ctx.send_viewport_cmd(ViewportCommand::WindowLevel(WindowLevel::Normal));
        }

        Self {
            welcome: settings.first_run,
            signals,
            attention: None,
            countdown,
            saved: settings.clone(),
            saved_position: SavedPosition::of(&settings),
            placement_pending: Some(PLACEMENT_DEFERRALS),
            pending_move: None,
            settings,
            config_path,
            save_failed: false,
            save_retry: None,
            update_check: None,
            update_retry: None,
            updates_allowed: false,
            update_step: UpdateStep::Offered,
            update_download: None,
            installed: false,
            dirty_since: None,
            locked: false,
            stopwatch,
            alarm: Alarm::default(),
            alarm_chimes: Alarm::default(),
            chimes: 0,
            tray,
            instrument,
            layout: DerivedLayout::new(&Theme::default()),
            theme: Theme::default(),
            window_size: None,
            line_width: WidthFloor::default(),
            window_styled: false,
            wheel: 0.0,
            autostart,
            exe,
            autostart_on,
            #[cfg(test)]
            pinned: None,
        }
    }

    /// An overlay for rendering off screen: no config file, no signal listener,
    /// no tray icon, and the wall clock pinned to `at`. See `shots.rs`.
    #[cfg(test)]
    pub fn pinned(ctx: &egui::Context, settings: Config, at: DateTime<Local>) -> Self {
        let signals = mpsc::channel().1;
        let tray = Tray::new(ctx, false);
        let mut app = Self::assemble(ctx, settings, None, signals, tray, Instrument::start(ctx), None);
        app.pinned = Some(at);
        app
    }

    /// The size the overlay last asked the OS for, in points.
    #[cfg(test)]
    pub fn requested_size(&self) -> Option<Vec2> {
        self.window_size
    }

    /// The wall clock, or the pinned one when rendering off screen.
    fn wall_clock(&self) -> DateTime<Local> {
        #[cfg(test)]
        if let Some(at) = self.pinned {
            return at;
        }
        Local::now()
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

    /// How far into its ringing the armed alarm is at `local`, if it rings.
    fn alarm_ringing(&self, local: &DateTime<Local>) -> Option<Duration> {
        let over = alarm::overtime(self.settings.alarm_due?, local.timestamp_millis())?;
        (over < alarm::RING_FOR).then_some(over)
    }

    /// A click, a key or a command stops an alarm that is ringing, and it is
    /// written down at once: it has done its job.
    fn silence_alarm(&mut self) {
        if self.alarm_ringing(&self.wall_clock()).is_some() {
            self.settings.alarm_due = None;
            self.write_config();
        }
    }

    /// Another launch found this one running. An overlay has no taskbar button
    /// to flash, so it takes the focus, wears an outline for a few seconds, and
    /// has its position checked again, which brings it back if the screen it
    /// was on has gone. A locked overlay is unlocked, as it would be after a
    /// restart: whoever launched it is looking for it, and wants to grab it.
    fn surface(&mut self, ctx: &egui::Context, now: Instant) {
        if self.locked {
            self.locked = false;
            ctx.send_viewport_cmd(ViewportCommand::MousePassthrough(false));
        }
        // Not for a script's overlay: the focus belongs to whoever is at the
        // keyboard, and pulling it would drop them out of a full-screen game.
        if !self.instrument.active() {
            ctx.send_viewport_cmd(ViewportCommand::Focus);
        }
        self.attention = Some(now);
        self.saved_position = SavedPosition::of(&self.settings);
        self.placement_pending = Some(PLACEMENT_DEFERRALS);
    }

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
            self.silence_alarm();
        }
        let hovered = !self.locked && ctx.input(|i| i.pointer.hover_pos()).is_some_and(|p| rect.contains(p));
        if hovered && let Some(cmd) = self.scroll_timer(&ctx) {
            self.apply(cmd, &ctx, now);
        }

        let mut s = self.settings.clone();
        // The card is text to be read, whatever is behind the overlay. A
        // chroma key is behind nothing, and a translucent backdrop over it
        // would key out as a dark tint.
        s.backdrop |= self.welcome;
        s.backdrop &= !s.chroma;
        // One reading of the wall clock per frame, shared by the readout and
        // the night schedule so they can never disagree about the time.
        let local = self.wall_clock();
        let ringing = self.alarm_ringing(&local);
        if ringing.is_none() && self.settings.alarm_due.is_some_and(|due| alarm::until(due, local.timestamp_millis()).is_none()) {
            self.settings.alarm_due = None;
            self.write_config();
        }
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
        let mut content_width = scene_size.x.max(caption_width);
        if let Scene::Line { main, .. } = &readout.scene {
            // 12-hour time has one hour digit until ten and two from ten to
            // one: measured with the second one, the window is as wide at
            // 9:59 as at 10:00 and does not grow past a screen edge.
            let glyphs = self.layout.glyphs();
            if s.mode == Mode::Clock && s.clock_format == ClockFormat::H12 && main.find(':') == Some(1) {
                let digit = ('0'..='9').map(|d| glyphs.cell_width(d)).fold(0.0, f32::max);
                content_width = content_width.max(glyphs.width_of(main) + digit);
            }
            let shape = LineShape {
                layout: key,
                mode: s.mode,
                clock_format: s.clock_format,
                show_seconds: s.show_seconds,
                show_date: s.show_date,
                second_zone: s.second_zone,
                timer_minutes: s.timer_minutes,
                pomodoro: s.pomodoro,
            };
            content_width = self.line_width.hold(shape, content_width);
        }
        let content = vec2(content_width, scene_size.y + caption_height);
        let window_size = (content + (m.pad + Vec2::splat(inset)) * 2.0).ceil();
        self.fit_window(&ctx, window_size);

        // Now that the real size is known, the saved position can be judged
        // against the monitors that are actually attached.
        if let Some(synthetic) = self.instrument.take_place_request() {
            self.saved_position = Some(SavedPosition::Pixels(synthetic));
            self.placement_pending = Some(PLACEMENT_DEFERRALS);
        }
        if self.placement_pending.is_some() {
            self.check_placement(&ctx, frame, window_size, now);
        }

        if s.backdrop {
            painter.rect_filled(rect, m.corner, theme.color.backdrop);
        }
        if self.welcome && !s.chroma {
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
        // A ringing alarm blinks the same frame in the alarm colour, in any
        // mode: it changes nothing in the layout, so the window keeps its size.
        if let Some(over) = ringing
            && over.as_millis() % 1000 < 600
        {
            painter.rect_stroke(rect.shrink(1.0), m.corner, Stroke::new(2.0, theme.color.alert), StrokeKind::Inside);
        }

        // Dimming over a halo would eat the contrast the halo just bought, so
        // captions and closed rows are only faded when there is none; and a
        // chroma key wants opaque text, since anything translucent keys as a
        // green tint.
        let dim_captions = halo.is_none() && !s.chroma;
        // Translucent hardware dressing — unlit diodes, sockets, hairlines —
        // keys as a tint, so none of it is drawn over a chroma key.
        let dressing = !s.chroma;
        // Unlit segments under the digital face, in the colour of the digits
        // drawn over them (a red stopwatch has red diodes, not green ones) and
        // at the ghost alpha whatever that colour has been dimmed to.
        let ghost_of = |color: Color32| (dressing && s.font.has_ghost()).then(|| theme.ghost(color));
        if let Some(lit) = readout.ring {
            paint_ring(&painter, rect, &m, theme, lit, halo, dressing);
        }
        let inner = rect.shrink(inset);
        let scene_top = inner.top() + m.pad.y;
        let caption_color = match &readout.scene {
            Scene::Line { main, color } => {
                let origin = pos2(inner.center().x - scene_size.x / 2.0, scene_top);
                paint_readout(&painter, origin, main, self.layout.glyphs(), *color, halo, ghost_of(*color), theme);
                // A plain readout colour dims with its caption; an alarm does not.
                let plain = *color == theme.color.text || *color == theme.color.secondary;
                if plain && dim_captions { theme.dim_caption(*color) } else { *color }
            }
            Scene::Board(rows) => {
                let geo = geometry.expect("measured with the board");
                let origin = pos2(inner.center().x - geo.size.x / 2.0, scene_top);
                let style = board::Style { halo, dim_closed: dim_captions, dressing, ghost: ghost_of(theme.color.text) };
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
        self.follow_clock_changes(now);
        self.check_for_updates(&ctx, now);
        let state = self.menu_state(now);
        self.tray.sync(state);
        self.persist(&ctx, now, window_position_px(&ctx, frame));

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
            // Counted either way, so a script can check the schedule without
            // beeping at whoever is using the machine meanwhile.
            if !self.instrument.active() {
                chime::play();
            }
            self.chimes += 1;
        }
        let alarm_wake = self.settings.timer_sound.then(|| self.alarm.next_in(&self.countdown, now)).flatten();
        // The alarm was set on purpose, so it is heard whatever the timer's
        // sound is set to. Its own wake gets a sleeping overlay there on time:
        // the time it is due, then every blink and chime while it rings.
        if self.alarm_chimes.due(ringing) {
            if !self.instrument.active() {
                chime::play();
            }
            self.chimes += 1;
        }
        let ring_wake = match ringing {
            Some(over) => {
                let into = (over.as_millis() % 1000) as u64;
                let blink = Duration::from_millis(if into < 600 { 600 - into } else { 1000 - into });
                Some(self.alarm_chimes.next_after(over).map_or(blink, |chime| chime.min(blink)))
            }
            None => self.settings.alarm_due.and_then(|due| alarm::until(due, local.timestamp_millis())),
        };
        let wake = [readout.next_change, night_wake, alarm_wake, attention_wake, ring_wake].into_iter().flatten().min();
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
                "mode={} welcome={} attention={} alarm_due={} ringing={} pomodoro={} phase={} win_w={:.1} win_h={:.1} sw_running={} sw_ms={} cd_running={} cd_finished={} cd_remaining_ms={} {buttons}",
                self.settings.mode.id(),
                u8::from(self.welcome),
                u8::from(self.attention.is_some()),
                self.settings.alarm_due.unwrap_or(0),
                u8::from(ringing.is_some()),
                u8::from(self.settings.pomodoro),
                self.settings.pomodoro_phase,
                rect.width(),
                rect.height(),
                u8::from(self.stopwatch.is_running()),
                self.stopwatch.elapsed(now).as_millis(),
                u8::from(self.countdown.is_running(now)),
                u8::from(self.countdown.is_finished(now)),
                self.countdown.remaining(now).as_millis(),
            )
        });

        instrument::reached(Milestone::FirstFrame);

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
        if self.settings != self.saved || self.save_failed {
            self.write_config();
        }
    }

    fn persist_egui_memory(&self) -> bool {
        false
    }
}

/// What the countdown runs for: the timer's duration, or the length of the
/// Pomodoro period it has got to.
fn countdown_length(settings: &Config) -> Duration {
    if settings.pomodoro {
        pomodoro::duration(settings.pomodoro_phase, settings.timer_minutes)
    } else {
        Duration::from_secs(settings.timer_minutes * 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_second_launch_unlocks_a_locked_overlay() {
        let ctx = egui::Context::default();
        let mut app = ChronoApp::pinned(&ctx, Config::default(), Local::now());
        app.locked = true;
        app.surface(&ctx, Instant::now());
        assert!(!app.locked);
        assert!(app.attention.is_some());
    }

}
