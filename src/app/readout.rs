//! What the overlay shows this frame, and when that next changes.

use std::time::{Duration, Instant};

use chrono::{DateTime, Local, Timelike as _};
use eframe::egui::Color32;

use crate::clock::{ClockStyle, clock_readout};
use crate::market::{self, BoardStyle};
use crate::night::{self, Schedule};
use crate::ring;
use crate::theme::{self, Theme};
use crate::timer;
use crate::welcome;

use super::{ChronoApp, Mode};

/// How long a finished timer blinks before settling on a steady colour.
const BLINK_FOR: Duration = Duration::from_secs(30);

impl ChronoApp {
    /// Resolves the theme for this frame and returns how long night mode can
    /// be left alone before it has to be looked at again (`Auto` only).
    pub(super) fn apply_night(&mut self, local: DateTime<Local>) -> Option<Duration> {
        let s = &self.settings;
        let schedule = Schedule { from: s.night_from, to: s.night_to };
        let (dim, wake) = night::resolve(s.night, schedule, s.night_dim, &local);
        self.theme = Theme::resolve(s.palette, dim, s.chroma);
        wake
    }

    /// What to draw above the caption line, the caption, the seconds ring's
    /// position, and when the display next changes.
    ///
    /// The clock is drawn in the primary colour, the counters in the
    /// secondary one; they only differ in a dual-colour preset.
    pub(super) fn readout(&self, now: Instant, local: DateTime<Local>) -> Readout {
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
                let style = ClockStyle {
                    format: s.clock_format,
                    show_seconds: s.show_seconds,
                    show_date: s.show_date,
                    zone: s.second_zone,
                };
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
                        if lit { c.alert } else { theme::fade(c.alert, 0.25, s.chroma) },
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
pub(super) enum Scene {
    /// One big readout line.
    Line { main: String, color: Color32 },
    /// The market board, one row per exchange.
    Board(Vec<market::Row>),
    /// The welcome card.
    Card,
}

pub(super) struct Readout {
    pub(super) scene: Scene,
    pub(super) caption: String,
    pub(super) next_change: Option<Duration>,
    /// How many LEDs of the studio ring are lit, when it is on.
    pub(super) ring: Option<usize>,
}

