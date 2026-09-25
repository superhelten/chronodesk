//! What the menu, the keys and the hover buttons ask for, and the menu's
//! picture of the app in return.

use std::time::{Duration, Instant};

use eframe::egui::{self, ViewportCommand, WindowLevel};

use crate::alarm;
use crate::market::Market;
use crate::night::TimeOfDay;
use crate::tray::{Command, MenuState};

use super::{ChronoApp, Mode, minutes};

/// How stale the menu's autostart check mark may get. The registry can change
/// behind the app's back (Settings, Task Manager); it is re-read on frames that
/// are drawn anyway, never by waking up for it.
const AUTOSTART_RECHECK: Duration = Duration::from_secs(10);

impl ChronoApp {
    pub(super) fn apply(&mut self, cmd: Command, ctx: &egui::Context, now: Instant) {
        // Whoever reaches for the menu or a key has found their way in.
        if cmd != Command::ShowWelcome {
            self.dismiss_welcome();
        }
        // So does anyone silencing a ringing alarm; whatever they picked still
        // happens, which lets a new alarm time replace the one ringing. The
        // switch acts on the check mark that was showing: clicked while it
        // rings, it means "off", not "again tomorrow".
        let alarm_was_on = self.settings.alarm_due.is_some();
        self.silence_alarm();
        let local = self.wall_clock();
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
            Command::ToggleAlarm => s.alarm_due = (!alarm_was_on).then(|| alarm::arm(s.alarm_at, &local)),
            Command::SetAlarmHour(hour) => {
                s.alarm_at = TimeOfDay::new(hour, s.alarm_at.minute()).unwrap_or(s.alarm_at);
                s.alarm_due = Some(alarm::arm(s.alarm_at, &local));
            }
            Command::SetAlarmMinute(minute) => {
                s.alarm_at = TimeOfDay::new(s.alarm_at.hour(), minute).unwrap_or(s.alarm_at);
                s.alarm_due = Some(alarm::arm(s.alarm_at, &local));
            }
            Command::SetAlarm(at) => {
                s.alarm_at = at;
                s.alarm_due = Some(alarm::arm(at, &local));
            }
            Command::SetSize(size) => s.size = size,
            Command::ToggleFont => s.font = s.font.toggled(),
            Command::SetFont(font) => s.font = font,
            Command::ToggleBackdrop => s.backdrop = !s.backdrop,
            Command::ToggleOutline => s.text_outline = !s.text_outline,
            Command::ToggleChroma => s.chroma = !s.chroma,
            Command::ToggleSeconds => s.show_seconds = !s.show_seconds,
            Command::ToggleClockFormat => s.clock_format = s.clock_format.toggled(),
            Command::ToggleDate => s.show_date = !s.show_date,
            Command::SetSecondZone(zone) => s.second_zone = zone,
            Command::SetPalette(palette) => s.palette = palette,
            Command::SetNight(night) => s.night = night,
            Command::ToggleOnTop => {
                s.always_on_top = !s.always_on_top;
                let level = if s.always_on_top { WindowLevel::AlwaysOnTop } else { WindowLevel::Normal };
                // A script's overlay stays at the bottom whatever it toggles.
                if !self.instrument.active() {
                    ctx.send_viewport_cmd(ViewportCommand::WindowLevel(level));
                }
            }
            Command::ToggleAutostart => self.toggle_autostart(now),
            Command::ShowWelcome => self.welcome = true,
            Command::Quit => ctx.send_viewport_cmd(ViewportCommand::Close),
            Command::Update => {}
            Command::ToggleUpdateCheck => {
                s.check_updates = !s.check_updates;
                // Switched off, it offers nothing it found before either.
                if !s.check_updates {
                    s.update_available = None;
                }
            }
        }
        if cmd == Command::Update {
            self.start_update(ctx);
        }
        if counters_touched {
            self.save_counters(now);
        }
        let alarm_touched =
            matches!(cmd, Command::ToggleAlarm | Command::SetAlarmHour(_) | Command::SetAlarmMinute(_) | Command::SetAlarm(_));
        if alarm_touched {
            // Like a running counter: a restart right after must not lose it.
            self.write_config();
        }
        // The OS flips check items on click; re-assert our state.
        self.tray.invalidate();
        // Commands can arrive after this frame's readout was taken.
        ctx.request_repaint();
    }

    /// One command per press. A held key repeats, and Space held down would
    /// otherwise flip the counter about thirty times a second, writing the
    /// config file each time, and leave it however the last flip fell.
    pub(super) fn keyboard_commands(&self, ctx: &egui::Context) -> Vec<Command> {
        use egui::Key;
        let pressed = |i: &egui::InputState, key: Key| {
            i.events.iter().any(|e| matches!(e, egui::Event::Key { key: k, pressed: true, repeat: false, .. } if *k == key))
        };
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
            .filter(|(key, _)| pressed(i, *key))
            .map(|(_, cmd)| cmd)
            .collect()
        })
    }

    /// Scrolling over an idle timer nudges its duration by a minute per notch.
    pub(super) fn scroll_timer(&mut self, ctx: &egui::Context) -> Option<Command> {
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

    pub(super) fn menu_state(&self, now: Instant) -> MenuState {
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
            second_zone: s.second_zone,
            palette: s.palette,
            night: s.night,
            markets: s.markets.clone(),
            board_layout: s.board_layout,
            board_labels: s.board_labels,
            seconds_ring: s.seconds_ring,
            timer_sound: s.timer_sound,
            alarm_at: s.alarm_at,
            alarm_on: s.alarm_due.is_some(),
            alarm_label: alarm::label(s.alarm_at, s.alarm_due, &self.wall_clock()),
            always_on_top: s.always_on_top,
            autostart: self.autostart_on.0,
            autostart_available: self.exe.is_some(),
            check_updates: s.check_updates,
            update: self.update_item(),
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

    pub(super) fn recheck_autostart(&mut self, now: Instant) {
        if let Some(exe) = self.exe.as_deref()
            && now.duration_since(self.autostart_on.1) >= AUTOSTART_RECHECK
        {
            self.autostart_on = (self.autostart.enabled(exe), now);
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Local;
    use crate::config::Config;

    /// Clicking the checked "Alarm at …" while it rings means "stop": the
    /// switch acts on the mark that was showing, not on the state that
    /// silencing it has just left behind.
    #[test]
    fn switching_a_ringing_alarm_off_leaves_it_off() {
        let ctx = egui::Context::default();
        let now = Local::now();
        let mut app = ChronoApp::pinned(&ctx, Config::default(), now);
        let t0 = Instant::now();

        app.settings.alarm_due = Some(now.timestamp() - 1);
        assert!(app.alarm_ringing(&now).is_some());
        app.apply(Command::ToggleAlarm, &ctx, t0);
        assert_eq!(app.settings.alarm_due, None, "silenced and off, not re-armed for tomorrow");

        app.apply(Command::ToggleAlarm, &ctx, t0);
        assert!(app.settings.alarm_due.is_some_and(|due| due > now.timestamp()), "off: the switch arms it");
        app.apply(Command::ToggleAlarm, &ctx, t0);
        assert_eq!(app.settings.alarm_due, None, "armed: the switch disarms it");

        // Picking a time while one rings replaces it.
        app.settings.alarm_due = Some(now.timestamp() - 1);
        app.apply(Command::SetAlarmHour(3), &ctx, t0);
        assert!(app.settings.alarm_due.is_some_and(|due| due > now.timestamp()));
        assert_eq!(app.settings.alarm_at.hour(), 3);
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
    fn a_held_key_is_one_command() {
        let ctx = egui::Context::default();
        let app = ChronoApp::pinned(&ctx, Config::default(), Local::now());
        let space = |repeat| egui::Event::Key {
            key: egui::Key::Space,
            physical_key: None,
            pressed: true,
            repeat,
            modifiers: egui::Modifiers::NONE,
        };
        let mut commands = Vec::new();
        let raw = egui::RawInput { events: vec![space(false), space(true), space(true)], ..Default::default() };
        ctx.run_ui(raw, |ui| commands = app.keyboard_commands(ui.ctx())).textures_delta.clear();
        assert_eq!(commands, vec![Command::StartPause]);

        let raw = egui::RawInput { events: vec![space(true)], ..Default::default() };
        ctx.run_ui(raw, |ui| commands = app.keyboard_commands(ui.ctx())).textures_delta.clear();
        assert_eq!(commands, vec![], "still held");
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
        toggle_market(&mut markets, Market::Oslo);
        assert_eq!(
            markets,
            vec![Market::NewYork, Market::London, Market::Oslo, Market::Frankfurt, Market::HongKong, Market::Tokyo]
        );
    }
}
