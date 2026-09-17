//! Pure stopwatch / countdown logic. Every method takes `now` explicitly so the
//! behaviour is deterministic and unit-testable.

use std::time::{Duration, Instant};

#[derive(Debug, Default, Clone)]
pub struct Stopwatch {
    accumulated: Duration,
    started: Option<Instant>,
}

impl Stopwatch {
    pub fn is_running(&self) -> bool {
        self.started.is_some()
    }

    pub fn elapsed(&self, now: Instant) -> Duration {
        self.accumulated + self.started.map_or(Duration::ZERO, |s| now.saturating_duration_since(s))
    }

    pub fn start(&mut self, now: Instant) {
        if self.started.is_none() {
            self.started = Some(now);
        }
    }

    pub fn pause(&mut self, now: Instant) {
        if let Some(s) = self.started.take() {
            self.accumulated += now.saturating_duration_since(s);
        }
    }

    pub fn toggle(&mut self, now: Instant) {
        if self.is_running() {
            self.pause(now);
        } else {
            self.start(now);
        }
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

#[derive(Debug, Clone)]
pub struct Countdown {
    duration: Duration,
    clock: Stopwatch,
}

impl Countdown {
    pub fn new(duration: Duration) -> Self {
        Self { duration, clock: Stopwatch::default() }
    }

    pub fn duration(&self) -> Duration {
        self.duration
    }

    /// Changes the duration and resets the countdown.
    pub fn set_duration(&mut self, duration: Duration) {
        self.duration = duration;
        self.clock.reset();
    }

    pub fn is_running(&self, now: Instant) -> bool {
        self.clock.is_running() && !self.is_finished(now)
    }

    /// True while nothing has been started yet (full duration remaining).
    pub fn is_idle(&self) -> bool {
        !self.clock.is_running() && self.clock.accumulated == Duration::ZERO
    }

    pub fn remaining(&self, now: Instant) -> Duration {
        self.duration.saturating_sub(self.clock.elapsed(now))
    }

    pub fn is_finished(&self, now: Instant) -> bool {
        self.duration > Duration::ZERO && self.clock.elapsed(now) >= self.duration
    }

    /// How long ago the countdown hit zero, if it has.
    pub fn overtime(&self, now: Instant) -> Option<Duration> {
        self.is_finished(now).then(|| self.clock.elapsed(now) - self.duration)
    }

    /// Start/pause. Toggling a finished countdown rearms it.
    pub fn toggle(&mut self, now: Instant) {
        if self.is_finished(now) {
            self.clock.reset();
        } else {
            self.clock.toggle(now);
        }
    }

    pub fn reset(&mut self) {
        self.clock.reset();
    }
}

/// `MM:SS.t` below one hour, `H:MM:SS` above.
pub fn format_stopwatch(d: Duration) -> String {
    let total_ms = d.as_millis();
    let secs = total_ms / 1000;
    let (h, m, s) = (secs / 3600, (secs / 60) % 60, secs % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m:02}:{s:02}.{}", (total_ms % 1000) / 100)
    }
}

/// Remaining time rounded *up* to whole seconds, so a fresh 25 min timer shows
/// `25:00` and `00:00` appears exactly when it finishes.
pub fn format_countdown(d: Duration) -> String {
    let secs = d.as_nanos().div_ceil(1_000_000_000);
    let (h, m, s) = (secs / 3600, (secs / 60) % 60, secs % 60);
    if h > 0 { format!("{h}:{m:02}:{s:02}") } else { format!("{m:02}:{s:02}") }
}

/// Time until the displayed stopwatch value next changes.
pub fn next_stopwatch_tick(elapsed: Duration) -> Duration {
    let step = if elapsed >= Duration::from_secs(3600) { 1_000_000_000 } else { 100_000_000 };
    let into = (elapsed.as_nanos() % step) as u64;
    Duration::from_nanos(step as u64 - into)
}

/// Time until the displayed countdown value next changes (ceil semantics).
pub fn next_countdown_tick(remaining: Duration) -> Duration {
    let into = (remaining.as_nanos() % 1_000_000_000) as u64;
    Duration::from_nanos(if into == 0 { 1_000_000_000 } else { into })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(v: u64) -> Duration {
        Duration::from_millis(v)
    }

    #[test]
    fn stopwatch_accumulates_across_pauses() {
        let t0 = Instant::now();
        let mut sw = Stopwatch::default();
        assert_eq!(sw.elapsed(t0), Duration::ZERO);
        sw.start(t0);
        assert_eq!(sw.elapsed(t0 + ms(1500)), ms(1500));
        sw.pause(t0 + ms(2000));
        assert!(!sw.is_running());
        assert_eq!(sw.elapsed(t0 + ms(9000)), ms(2000));
        sw.toggle(t0 + ms(10_000));
        assert_eq!(sw.elapsed(t0 + ms(10_500)), ms(2500));
        sw.reset();
        assert_eq!(sw.elapsed(t0 + ms(20_000)), Duration::ZERO);
    }

    #[test]
    fn double_start_does_not_restart() {
        let t0 = Instant::now();
        let mut sw = Stopwatch::default();
        sw.start(t0);
        sw.start(t0 + ms(500));
        assert_eq!(sw.elapsed(t0 + ms(1000)), ms(1000));
    }

    #[test]
    fn countdown_runs_to_zero_and_reports_overtime() {
        let t0 = Instant::now();
        let mut cd = Countdown::new(ms(3000));
        assert!(cd.is_idle());
        cd.toggle(t0);
        assert!(cd.is_running(t0 + ms(100)));
        assert_eq!(cd.remaining(t0 + ms(1000)), ms(2000));
        assert!(!cd.is_finished(t0 + ms(2999)));
        assert!(cd.is_finished(t0 + ms(3000)));
        assert!(!cd.is_running(t0 + ms(3000)));
        assert_eq!(cd.remaining(t0 + ms(5000)), Duration::ZERO);
        assert_eq!(cd.overtime(t0 + ms(5000)), Some(ms(2000)));
    }

    #[test]
    fn toggling_finished_countdown_rearms_it() {
        let t0 = Instant::now();
        let mut cd = Countdown::new(ms(1000));
        cd.toggle(t0);
        cd.toggle(t0 + ms(4000));
        assert!(!cd.is_finished(t0 + ms(4000)));
        assert_eq!(cd.remaining(t0 + ms(4000)), ms(1000));
    }

    #[test]
    fn set_duration_resets() {
        let t0 = Instant::now();
        let mut cd = Countdown::new(ms(1000));
        cd.toggle(t0);
        cd.set_duration(ms(5000));
        assert_eq!(cd.remaining(t0 + ms(800)), ms(5000));
        assert!(cd.is_idle());
    }

    #[test]
    fn stopwatch_format() {
        assert_eq!(format_stopwatch(Duration::ZERO), "00:00.0");
        assert_eq!(format_stopwatch(ms(61_999)), "01:01.9");
        assert_eq!(format_stopwatch(ms(3_599_950)), "59:59.9");
        assert_eq!(format_stopwatch(Duration::from_secs(3600 + 62)), "1:01:02");
    }

    #[test]
    fn countdown_format_rounds_up() {
        assert_eq!(format_countdown(Duration::from_secs(25 * 60)), "25:00");
        assert_eq!(format_countdown(ms(59_001)), "01:00");
        assert_eq!(format_countdown(ms(1)), "00:01");
        assert_eq!(format_countdown(Duration::ZERO), "00:00");
        assert_eq!(format_countdown(Duration::from_secs(2 * 3600)), "2:00:00");
    }

    #[test]
    fn ticks_land_on_display_boundaries() {
        assert_eq!(next_stopwatch_tick(ms(1230)), ms(70));
        assert_eq!(next_stopwatch_tick(ms(1200)), ms(100));
        assert_eq!(next_stopwatch_tick(Duration::from_millis(3_600_250)), ms(750));
        assert_eq!(next_countdown_tick(ms(2400)), ms(400));
        assert_eq!(next_countdown_tick(ms(2000)), ms(1000));
    }
}
