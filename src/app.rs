use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{Local, Timelike as _};
use eframe::egui::{
    self, Color32, FontFamily, FontId, Galley, PointerButton, Pos2, Rect, Sense, Stroke,
    StrokeKind, Vec2, ViewportCommand, WindowLevel, pos2, vec2,
};
use serde::{Deserialize, Serialize};

use crate::config::{self, Config};
use crate::instrument::{Cause, Instrument};
use crate::layout::{self, DerivedLayout, Glyphs, LayoutKey, Metrics};
use crate::theme::Theme;
use crate::timer::{self, Countdown, Stopwatch};
use crate::tray::{Command, MenuState, Tray};

const DISPLAY_FONT: &str = "display";
/// How long a finished timer blinks before settling on a steady colour.
const BLINK_FOR: Duration = Duration::from_secs(30);
/// Timed wake-ups can fire more than one OS timer tick (~15.6 ms on Windows)
/// early. Waking just before a boundary would find "1 ms left" and spin at vsync
/// until it passes, so aim well past it; a 25 ms display lag is invisible.
const WAKE_SLACK: Duration = Duration::from_millis(25);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    #[default]
    Clock,
    Stopwatch,
    Timer,
}

impl Mode {
    pub const ALL: [Self; 3] = [Self::Clock, Self::Stopwatch, Self::Timer];

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.id().eq_ignore_ascii_case(id))
    }

    pub fn id(self) -> &'static str {
        match self {
            Self::Clock => "clock",
            Self::Stopwatch => "stopwatch",
            Self::Timer => "timer",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Clock => "Clock",
            Self::Stopwatch => "Stopwatch",
            Self::Timer => "Timer",
        }
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
    tray: Tray,
    instrument: Instrument,
    theme: Theme,
    /// Cached layout: rebuilt on size or scale changes, not per frame.
    layout: DerivedLayout,
    /// Last size requested from the OS, to avoid resize commands every frame.
    window_size: Option<Vec2>,
    window_styled: bool,
    wheel: f32,
}

/// How long to wait after the last change before writing the config file.
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
        if !settings.always_on_top {
            cc.egui_ctx.send_viewport_cmd(ViewportCommand::WindowLevel(WindowLevel::Normal));
        }

        Ok(Self {
            countdown: Countdown::new(minutes(settings.timer_minutes)),
            saved: settings.clone(),
            settings,
            config_path: config::config_path(),
            dirty_since: repaired.then(Instant::now),
            locked: false,
            stopwatch: Stopwatch::default(),
            tray: Tray::new(&cc.egui_ctx),
            instrument: Instrument::start(&cc.egui_ctx),
            layout: DerivedLayout::new(&Theme::default()),
            theme: Theme::default(),
            window_size: None,
            window_styled: false,
            wheel: 0.0,
        })
    }

    /// Notes the current window position and writes the config once it has been
    /// unchanged for [`SAVE_DELAY`].
    fn persist(&mut self, ctx: &egui::Context, now: Instant) {
        if let Some(rect) = ctx.input(|i| i.viewport().outer_rect) {
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

    fn apply(&mut self, cmd: Command, ctx: &egui::Context, now: Instant) {
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
                Mode::Clock => {}
                Mode::Stopwatch => self.stopwatch.toggle(now),
                Mode::Timer => self.countdown.toggle(now),
            },
            Command::Reset => match s.mode {
                Mode::Clock => {}
                Mode::Stopwatch => self.stopwatch.reset(),
                Mode::Timer => self.countdown.reset(),
            },
            Command::SetSize(size) => s.size = size,
            Command::ToggleBackdrop => s.backdrop = !s.backdrop,
            Command::ToggleOutline => s.text_outline = !s.text_outline,
            Command::ToggleChroma => s.chroma = !s.chroma,
            Command::ToggleSeconds => s.show_seconds = !s.show_seconds,
            Command::ToggleOnTop => {
                s.always_on_top = !s.always_on_top;
                let level = if s.always_on_top { WindowLevel::AlwaysOnTop } else { WindowLevel::Normal };
                ctx.send_viewport_cmd(ViewportCommand::WindowLevel(level));
            }
            Command::Quit => ctx.send_viewport_cmd(ViewportCommand::Close),
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
            backdrop: s.backdrop,
            text_outline: s.text_outline,
            chroma: s.chroma,
            show_seconds: s.show_seconds,
            always_on_top: s.always_on_top,
        }
    }

    /// Big readout, small caption line, colour, and when the display next changes.
    fn readout(&self, now: Instant) -> Readout {
        let s = &self.settings;
        let c = &self.theme.color;
        match s.mode {
            Mode::Clock => {
                let t = Local::now();
                let (main, until_change) = if s.show_seconds {
                    (t.format("%H:%M:%S").to_string(), 1_000_000_000 - u64::from(t.nanosecond() % 1_000_000_000))
                } else {
                    let into_minute = u64::from(t.second()) * 1_000_000_000 + u64::from(t.nanosecond() % 1_000_000_000);
                    (t.format("%H:%M").to_string(), 60_000_000_000 - into_minute)
                };
                Readout {
                    main,
                    caption: t.format("%a %-d %b").to_string().to_uppercase(),
                    color: c.text,
                    next_change: Some(Duration::from_nanos(until_change)),
                }
            }
            Mode::Stopwatch => {
                let elapsed = self.stopwatch.elapsed(now);
                let running = self.stopwatch.is_running();
                Readout {
                    main: timer::format_stopwatch(elapsed),
                    caption: if running || elapsed.is_zero() { "STOPWATCH" } else { "PAUSED" }.into(),
                    color: c.text,
                    next_change: running.then(|| timer::next_stopwatch_tick(elapsed)),
                }
            }
            Mode::Timer => {
                let cd = &self.countdown;
                let remaining = cd.remaining(now);
                if let Some(over) = cd.overtime(now) {
                    let blinking = over < BLINK_FOR;
                    let lit = !blinking || over.as_millis() % 1000 < 600;
                    let into = (over.as_millis() % 1000) as u64;
                    let next = if into < 600 { 600 - into } else { 1000 - into };
                    Readout {
                        main: timer::format_countdown(remaining),
                        caption: "TIME'S UP".into(),
                        color: if lit { c.alert } else { c.alert.gamma_multiply(0.25) },
                        next_change: blinking.then(|| Duration::from_millis(next)),
                    }
                } else {
                    let running = cd.is_running(now);
                    let caption = if running {
                        "TIMER".to_owned()
                    } else if cd.is_idle() {
                        format!("TIMER · {} MIN", cd.duration().as_secs() / 60)
                    } else {
                        "PAUSED".to_owned()
                    };
                    Readout {
                        main: timer::format_countdown(remaining),
                        caption,
                        color: c.text,
                        next_change: running.then(|| timer::next_countdown_tick(remaining)),
                    }
                }
            }
        }
    }
}

struct Readout {
    main: String,
    caption: String,
    color: Color32,
    next_change: Option<Duration>,
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

        let rect = ui.max_rect();
        let background = ui.allocate_rect(rect, Sense::click_and_drag());
        let hovered = !self.locked && ctx.input(|i| i.pointer.hover_pos()).is_some_and(|p| rect.contains(p));
        if hovered && let Some(cmd) = self.scroll_timer(&ctx) {
            self.apply(cmd, &ctx, now);
        }

        let s = self.settings.clone();
        let readout = self.readout(now);
        let painter = ui.painter().clone();

        // Rebuilt only when the size or the display scale changes; every frame
        // in between reads cached numbers.
        let theme = self.theme;
        let key = LayoutKey::new(s.size, ctx.pixels_per_point());
        self.layout.ensure(key, &theme, |metrics| measure_glyphs(&ctx, metrics.font));
        let m = *self.layout.metrics();
        // A backdrop already separates the text from whatever is behind it.
        let halo = (s.text_outline && !s.backdrop).then_some(m.halo_width);

        let main_width = self.layout.width_of(&readout.main);
        let main_height = self.layout.glyphs().height;
        let caption_galley = {
            let font = FontId::new(m.caption_font, display_family());
            let text = spaced(&readout.caption);
            let painter = painter.clone();
            self.layout.caption_galley(&readout.caption, m.caption_font, move || {
                painter.layout_no_wrap(text, font, theme.color.text)
            })
        };
        let theme = &theme;

        let content = vec2(main_width.max(caption_galley.size().x), main_height + m.caption_height);
        self.fit_window(&ctx, (content + m.pad * 2.0).ceil());

        if s.backdrop {
            painter.rect_filled(rect, m.corner, theme.color.backdrop);
        }
        if hovered && !s.backdrop {
            let stroke = Stroke::new(1.0, theme.color.hover_frame);
            painter.rect_stroke(rect.shrink(0.5), m.corner, stroke, StrokeKind::Inside);
        }

        let main_origin = pos2(rect.center().x - main_width / 2.0, rect.top() + m.pad.y);
        let mut x = main_origin.x;
        for c in readout.main.chars() {
            let cell = self.layout.cell_width(c);
            if let Some(galley) = self.layout.galley(c) {
                let pos = pos2(x + (cell - galley.size().x) / 2.0, main_origin.y);
                paint_galley(&painter, pos, galley.clone(), readout.color, halo, theme);
            }
            x += cell;
        }

        let caption_rect = Rect::from_min_size(
            pos2(rect.left(), main_origin.y + main_height),
            vec2(rect.width(), m.caption_height),
        );
        let show_controls = hovered && s.mode != Mode::Clock;
        let mut pending = None;
        if show_controls {
            pending = controls(ui, caption_rect, &m, theme, self.menu_state(now).start_label);
        } else {
            let pos = caption_rect.center() - caption_galley.size() / 2.0;
            // Dimming the caption over a halo would eat the contrast the halo
            // just bought, so only dim it when there is a backdrop.
            let color = match (readout.color == theme.color.text, halo) {
                (true, None) => theme.color.text.gamma_multiply(theme.color.caption_dim),
                _ => readout.color,
            };
            // The caption is far smaller, so it gets the thinnest ring that
            // still separates it from the background.
            paint_galley(&painter, pos, caption_galley, color, halo.map(|_| 1.0), theme);
        }
        if let Some(cmd) = pending {
            self.apply(cmd, &ctx, now);
        }

        if background.drag_started_by(PointerButton::Primary) {
            ctx.send_viewport_cmd(ViewportCommand::StartDrag);
        }

        let state = self.menu_state(now);
        self.tray.sync(state);
        self.persist(&ctx, now);

        // Schedule from the readout actually drawn: sampling the clock again here
        // could straddle a boundary and leave a stale value up for a full period.
        if let Some(next) = readout.next_change {
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
        self.instrument.record_frame(frame_start.elapsed(), cause);

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

/// Start/pause and reset buttons, drawn as shapes so no icon font is needed.
fn controls(
    ui: &mut egui::Ui,
    area: Rect,
    metrics: &Metrics,
    theme: &Theme,
    start_label: &str,
) -> Option<Command> {
    let (size, gap) = (metrics.control, metrics.control_gap);
    let center = area.center();
    let buttons = [
        (Rect::from_center_size(center - vec2((size + gap) / 2.0, 0.0), Vec2::splat(size)), Command::StartPause),
        (Rect::from_center_size(center + vec2((size + gap) / 2.0, 0.0), Vec2::splat(size)), Command::Reset),
    ];
    let mut clicked = None;
    for (rect, cmd) in buttons {
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

/// Lays out every character a readout can contain, once per font size. Digits
/// share the widest digit's cell so the readout doesn't jitter as it ticks,
/// whatever the font's own digit metrics are.
fn measure_glyphs(ctx: &egui::Context, font_size: f32) -> Glyphs {
    let font = FontId::new(font_size, display_family());
    ctx.fonts_mut(|fonts| {
        let mut layout = |c: char| fonts.layout_no_wrap(c.to_string(), font.clone(), Color32::WHITE);
        let digit_width = ('0'..='9').map(|d| layout(d).size().x).fold(0.0, f32::max);
        let mut glyphs = Glyphs { digit_width, ..Default::default() };
        for c in layout::READOUT_CHARS.chars() {
            let galley = layout(c);
            glyphs.height = glyphs.height.max(galley.size().y);
            if !c.is_ascii_digit() {
                glyphs.widths.insert(c, galley.size().x);
            }
            glyphs.galleys.insert(c, galley);
        }
        glyphs
    })
}

/// Draws `galley`, optionally haloed by a dark edge `width` points thick.
///
/// egui has no outlined text, so the halo is offset copies of the glyph. Two
/// rings of eight, the outer one fainter, read as a soft dark edge; a single
/// hard ring at a width thin strokes can stand turns the readout into a hollow
/// outline font instead.
fn paint_galley(
    painter: &egui::Painter,
    pos: Pos2,
    galley: Arc<Galley>,
    color: Color32,
    outline: Option<f32>,
    theme: &Theme,
) {
    if let Some(width) = outline {
        for (radius, alpha) in [(width, theme.color.halo[0]), (width * 2.1, theme.color.halo[1])] {
            let diagonal = radius * std::f32::consts::FRAC_1_SQRT_2;
            let ring = [
                vec2(radius, 0.0),
                vec2(-radius, 0.0),
                vec2(0.0, radius),
                vec2(0.0, -radius),
                vec2(diagonal, diagonal),
                vec2(diagonal, -diagonal),
                vec2(-diagonal, diagonal),
                vec2(-diagonal, -diagonal),
            ];
            let shade = Color32::from_black_alpha(alpha);
            for offset in ring {
                painter.galley_with_override_text_color(pos + offset, galley.clone(), shade);
            }
        }
    }
    painter.galley_with_override_text_color(pos, galley, color);
}

/// Letter-spaced caption ("T I M E R"-lite): thin spaces read cleaner at small sizes.
fn spaced(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 4);
    for (i, c) in s.chars().enumerate() {
        if i > 0 {
            out.push('\u{2009}');
        }
        out.push(c);
    }
    out
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

fn display_family() -> FontFamily {
    FontFamily::Name(DISPLAY_FONT.into())
}

/// Uses a light system UI face for the readout when one is available, falling
/// back to egui's bundled font so the app never depends on it.
fn install_display_font(ctx: &egui::Context) {
    const CANDIDATES: &[&str] = &[
        r"C:\Windows\Fonts\segoeuisl.ttf",
        r"C:\Windows\Fonts\segoeui.ttf",
        "/System/Library/Fonts/SFNS.ttf",
        "/System/Library/Fonts/Helvetica.ttc",
    ];
    let mut fonts = egui::FontDefinitions::default();
    let mut family = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    if let Some(bytes) = CANDIDATES.iter().find_map(|path| std::fs::read(path).ok()) {
        fonts.font_data.insert(DISPLAY_FONT.to_owned(), Arc::new(egui::FontData::from_owned(bytes)));
        family.insert(0, DISPLAY_FONT.to_owned());
    }
    fonts.families.insert(display_family(), family);
    ctx.set_fonts(fonts);
}
