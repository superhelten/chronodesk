use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{Local, Timelike as _};
use eframe::egui::{
    self, Color32, FontFamily, FontId, Galley, PointerButton, Pos2, Rect, Sense, Stroke,
    StrokeKind, Vec2, ViewportCommand, WindowLevel, pos2, vec2,
};
use serde::{Deserialize, Serialize};

use crate::timer::{self, Countdown, Stopwatch};
use crate::tray::{Command, MenuState, Tray};

const TEXT: Color32 = Color32::from_rgb(242, 242, 240);
const ALERT: Color32 = Color32::from_rgb(245, 165, 36);
const SHADOW: Color32 = Color32::from_black_alpha(150);
const BACKDROP: Color32 = Color32::from_rgba_premultiplied(8, 8, 10, 178);
const CHROMA: Color32 = Color32::from_rgb(0, 255, 0);
const DISPLAY_FONT: &str = "display";
/// How long a finished timer blinks before settling on a steady colour.
const BLINK_FOR: Duration = Duration::from_secs(30);
/// Timed wake-ups can fire more than one OS timer tick (~15.6 ms on Windows)
/// early. Waking just before a boundary would find "1 ms left" and spin at vsync
/// until it passes, so aim well past it; a 25 ms display lag is invisible.
const WAKE_SLACK: Duration = Duration::from_millis(25);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Mode {
    #[default]
    Clock,
    Stopwatch,
    Timer,
}

impl Mode {
    pub const ALL: [Self; 3] = [Self::Clock, Self::Stopwatch, Self::Timer];

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Size {
    Small,
    #[default]
    Medium,
    Large,
}

impl Size {
    pub const ALL: [Self; 3] = [Self::Small, Self::Medium, Self::Large];

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

    fn font_size(self) -> f32 {
        match self {
            Self::Small => 28.0,
            Self::Medium => 44.0,
            Self::Large => 68.0,
        }
    }
}

/// Persisted between runs. `locked` is deliberately not stored: the app always
/// starts interactive so it can never come up unreachable.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
struct Settings {
    mode: Mode,
    timer_minutes: u64,
    size: Size,
    backdrop: bool,
    chroma: bool,
    show_seconds: bool,
    always_on_top: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            mode: Mode::Clock,
            timer_minutes: 25,
            size: Size::Medium,
            backdrop: false,
            chroma: false,
            show_seconds: true,
            always_on_top: true,
        }
    }
}

pub struct ChronoApp {
    settings: Settings,
    locked: bool,
    stopwatch: Stopwatch,
    countdown: Countdown,
    tray: Tray,
    /// Last size requested from the OS, to avoid resize commands every frame.
    window_size: Option<Vec2>,
    window_styled: bool,
    wheel: f32,
}

impl ChronoApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let settings: Settings =
            cc.storage.and_then(|s| eframe::get_value(s, eframe::APP_KEY)).unwrap_or_default();

        install_display_font(&cc.egui_ctx);
        if !settings.always_on_top {
            cc.egui_ctx.send_viewport_cmd(ViewportCommand::WindowLevel(WindowLevel::Normal));
        }

        Ok(Self {
            countdown: Countdown::new(minutes(settings.timer_minutes)),
            settings,
            locked: false,
            stopwatch: Stopwatch::default(),
            tray: Tray::new(&cc.egui_ctx),
            window_size: None,
            window_styled: false,
            wheel: 0.0,
        })
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
            chroma: s.chroma,
            show_seconds: s.show_seconds,
            always_on_top: s.always_on_top,
        }
    }

    /// Big readout, small caption line, colour, and when the display next changes.
    fn readout(&self, now: Instant) -> Readout {
        let s = &self.settings;
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
                    color: TEXT,
                    next_change: Some(Duration::from_nanos(until_change)),
                }
            }
            Mode::Stopwatch => {
                let elapsed = self.stopwatch.elapsed(now);
                let running = self.stopwatch.is_running();
                Readout {
                    main: timer::format_stopwatch(elapsed),
                    caption: if running || elapsed.is_zero() { "STOPWATCH" } else { "PAUSED" }.into(),
                    color: TEXT,
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
                        color: if lit { ALERT } else { ALERT.gamma_multiply(0.25) },
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
                        color: TEXT,
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
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let now = Instant::now();
        if !self.window_styled {
            self.window_styled = true;
            strip_window_chrome(frame);
        }

        let mut commands: Vec<Command> = self.tray.commands().collect();
        if !self.locked {
            commands.extend(self.keyboard_commands(&ctx));
        }
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
        let font = s.size.font_size();
        let caption_size = (font * 0.26).max(10.0);
        let pad = vec2(font * 0.36, font * 0.16);
        let shadow = !s.backdrop && !s.chroma;
        let painter = ui.painter().clone();

        // Measure first so the window can hug the content.
        let main = DigitLine::layout(&ctx, &readout.main, FontId::new(font, display_family()));
        let caption_font = FontId::new(caption_size, display_family());
        let caption = painter.layout_no_wrap(spaced(&readout.caption), caption_font, TEXT);
        let caption_h = caption_size * 1.5;
        let content = vec2(main.width.max(caption.size().x), main.height + caption_h);
        self.fit_window(&ctx, (content + pad * 2.0).ceil());

        if s.backdrop {
            painter.rect_filled(rect, font * 0.22, BACKDROP);
        }
        if hovered && !s.backdrop {
            painter.rect_stroke(rect.shrink(0.5), font * 0.22, Stroke::new(1.0, Color32::from_white_alpha(46)), StrokeKind::Inside);
        }

        let main_origin = pos2(rect.center().x - main.width / 2.0, rect.top() + pad.y);
        main.paint(&painter, main_origin, readout.color, shadow);

        let caption_rect = Rect::from_min_size(
            pos2(rect.left(), main_origin.y + main.height),
            vec2(rect.width(), caption_h),
        );
        let show_controls = hovered && s.mode != Mode::Clock;
        let mut pending = None;
        if show_controls {
            pending = controls(ui, caption_rect, caption_size * 1.25, self.menu_state(now).start_label);
        } else {
            let pos = caption_rect.center() - caption.size() / 2.0;
            let color = if readout.color == TEXT { TEXT.gamma_multiply(0.62) } else { readout.color };
            if shadow {
                painter.galley_with_override_text_color(pos + vec2(1.0, 1.0), caption.clone(), SHADOW);
            }
            painter.galley_with_override_text_color(pos, caption, color);
        }
        if let Some(cmd) = pending {
            self.apply(cmd, &ctx, now);
        }

        if background.drag_started_by(PointerButton::Primary) {
            ctx.send_viewport_cmd(ViewportCommand::StartDrag);
        }

        let state = self.menu_state(now);
        self.tray.sync(state);

        // Schedule from the readout actually drawn: sampling the clock again here
        // could straddle a boundary and leave a stale value up for a full period.
        if let Some(next) = readout.next_change {
            ctx.request_repaint_after(next + WAKE_SLACK);
        }

        // Last: this blocks in a native modal loop until the menu closes.
        if background.secondary_clicked() {
            self.tray.show_context_menu(frame);
        }
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        if self.settings.chroma { CHROMA.to_normalized_gamma_f32() } else { [0.0; 4] }
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, eframe::APP_KEY, &self.settings);
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
fn controls(ui: &mut egui::Ui, area: Rect, size: f32, start_label: &str) -> Option<Command> {
    let gap = size * 0.9;
    let center = area.center();
    let buttons = [
        (Rect::from_center_size(center - vec2((size + gap) / 2.0, 0.0), Vec2::splat(size)), Command::StartPause),
        (Rect::from_center_size(center + vec2((size + gap) / 2.0, 0.0), Vec2::splat(size)), Command::Reset),
    ];
    let mut clicked = None;
    for (rect, cmd) in buttons {
        let response = ui.interact(rect, ui.id().with(format!("{cmd:?}")), Sense::click());
        let alpha = if response.hovered() { 255 } else { 235 };
        let color = Color32::from_white_alpha(alpha);
        let painter = ui.painter();
        // Dark base keeps the glyphs legible over busy windows behind the overlay;
        // alpha 190 holds ~9:1 glyph contrast even over pure white.
        painter.circle_filled(rect.center(), size * 0.78, Color32::from_black_alpha(190));
        if response.hovered() {
            painter.circle_filled(rect.center(), size * 0.78, Color32::from_white_alpha(36));
        }
        let r = rect.shrink(size * 0.22);
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

/// A line of text where every digit gets the same cell width, so the readout
/// doesn't jitter as numbers change regardless of the font's digit metrics.
struct DigitLine {
    glyphs: Vec<(Arc<Galley>, f32)>,
    width: f32,
    height: f32,
}

impl DigitLine {
    fn layout(ctx: &egui::Context, text: &str, font: FontId) -> Self {
        ctx.fonts_mut(|fonts| {
            let mut layout = |s: String| fonts.layout_no_wrap(s, font.clone(), Color32::WHITE);
            let digit_w = ('0'..='9').map(|d| layout(d.to_string()).size().x).fold(0.0, f32::max);
            let glyphs: Vec<_> = text
                .chars()
                .map(|c| {
                    let g = layout(c.to_string());
                    let cell = if c.is_ascii_digit() { digit_w } else { g.size().x };
                    (g, cell)
                })
                .collect();
            let width = glyphs.iter().map(|(_, w)| w).sum();
            let height = glyphs.iter().map(|(g, _)| g.size().y).fold(0.0, f32::max);
            Self { glyphs, width, height }
        })
    }

    fn paint(&self, painter: &egui::Painter, origin: Pos2, color: Color32, shadow: bool) {
        let offset = (self.height * 0.03).max(1.0);
        let mut x = origin.x;
        for (galley, cell) in &self.glyphs {
            let pos = pos2(x + (cell - galley.size().x) / 2.0, origin.y);
            if shadow {
                painter.galley_with_override_text_color(pos + vec2(offset, offset), galley.clone(), SHADOW);
            }
            painter.galley_with_override_text_color(pos, galley.clone(), color);
            x += cell;
        }
    }
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
