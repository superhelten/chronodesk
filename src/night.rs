//! Night mode: a manual switch or a daily schedule that dims the readout.
//!
//! The schedule is pure arithmetic on the local time of day, so the app can
//! ask two things without touching the clock itself: is it night now, and
//! how long until that answer changes. The second is what keeps the app
//! asleep between boundaries instead of polling.

use std::fmt;
use std::time::Duration;

use chrono::{DateTime, LocalResult, NaiveDateTime, NaiveTime, TimeZone, Timelike as _};
use serde::{Deserialize, Serialize};

use crate::app::serde_by_id;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NightMode {
    #[default]
    Off,
    On,
    Auto,
}

impl NightMode {
    pub const ALL: [Self; 3] = [Self::Off, Self::On, Self::Auto];

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.id().eq_ignore_ascii_case(id))
    }

    pub fn id(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::On => "on",
            Self::Auto => "auto",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::On => "On",
            Self::Auto => "Auto (scheduled)",
        }
    }
}
serde_by_id!(NightMode, "night mode");

/// A wall-clock time of day, stored as minutes since midnight and written to
/// the config file as `"HH:MM"` so it stays hand-editable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct TimeOfDay(u16);

impl TimeOfDay {
    pub fn new(hour: u32, minute: u32) -> Option<Self> {
        (hour < 24 && minute < 60).then(|| Self((hour * 60 + minute) as u16))
    }

    /// Accepts `HH:MM` and `H:MM`.
    pub fn parse(text: &str) -> Option<Self> {
        let (hour, minute) = text.split_once(':')?;
        if !(1..=2).contains(&hour.len()) || minute.len() != 2 {
            return None;
        }
        Self::new(hour.parse().ok()?, minute.parse().ok()?)
    }

    fn seconds(self) -> u32 {
        u32::from(self.0) * 60
    }

    pub fn hour(self) -> u32 {
        u32::from(self.0) / 60
    }

    pub fn minute(self) -> u32 {
        u32::from(self.0) % 60
    }

    pub fn time(self) -> NaiveTime {
        NaiveTime::from_hms_opt(self.hour(), self.minute(), 0).expect("hour and minute in range")
    }
}

impl fmt::Display for TimeOfDay {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:02}:{:02}", self.0 / 60, self.0 % 60)
    }
}

impl Serialize for TimeOfDay {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for TimeOfDay {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Self::parse(&text).ok_or_else(|| serde::de::Error::custom(format!("expected HH:MM, got '{text}'")))
    }
}

/// The nightly window `[from, to)`. A `from` later than `to` wraps past
/// midnight; equal times mean no window at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Schedule {
    pub from: TimeOfDay,
    pub to: TimeOfDay,
}

impl Schedule {
    pub fn active(self, now: NaiveTime) -> bool {
        let (from, to, now) = (self.from.seconds(), self.to.seconds(), now.num_seconds_from_midnight());
        if from == to {
            false
        } else if from < to {
            (from..to).contains(&now)
        } else {
            now >= from || now < to
        }
    }

    /// Real time until `active` next changes in `now`'s time zone, or `None`
    /// when it never will. Counted on the wall clock alone, the wait is an
    /// hour too long across the night the clocks go forward, and an app that
    /// has nothing else to draw would sleep through the boundary.
    pub fn until_boundary_in<Tz: TimeZone>(self, now: &DateTime<Tz>) -> Option<Duration> {
        let wall = self.until_boundary(now.time())?;
        let target = now.naive_local() + chrono::Duration::from_std(wall).ok()?;
        let real = resolve_ahead(&target, now).and_then(|at| (at - now.clone()).to_std().ok()).filter(|d| !d.is_zero());
        Some(real.unwrap_or(wall))
    }

    /// Wall-clock time until `active` next changes, or `None` when it never will.
    pub fn until_boundary(self, now: NaiveTime) -> Option<Duration> {
        const DAY: u64 = 24 * 3600 * 1_000_000_000;
        if self.from == self.to {
            return None;
        }
        let now_nanos =
            u64::from(now.num_seconds_from_midnight()) * 1_000_000_000 + u64::from(now.nanosecond() % 1_000_000_000);
        let until = [self.from, self.to]
            .into_iter()
            .map(|b| {
                // A boundary passed this very instant already took effect, so
                // the next one of its kind is a day away.
                match (u64::from(b.seconds()) * 1_000_000_000 + DAY - now_nanos) % DAY {
                    0 => DAY,
                    d => d,
                }
            })
            .min()
            .expect("two boundaries");
        Some(Duration::from_nanos(until))
    }
}

/// The real instant a wall-clock time in `now`'s zone stands for, as a
/// clock hand reaches it: an hour the clocks go back through is shown twice,
/// and it is the first showing still ahead of `now`; an hour the clocks skip
/// is never shown, and what was due in it happens as the clock jumps past,
/// at the first minute that exists.
pub fn resolve_ahead<Tz: TimeZone>(local: &NaiveDateTime, now: &DateTime<Tz>) -> Option<DateTime<Tz>> {
    let tz = now.timezone();
    match tz.from_local_datetime(local) {
        LocalResult::Single(at) => Some(at),
        LocalResult::Ambiguous(first, second) => Some(if first > *now { first } else { second }),
        LocalResult::None => {
            (1..=180).find_map(|m| tz.from_local_datetime(&(*local + chrono::Duration::minutes(m))).earliest())
        }
    }
}

/// What night mode does at `now`: the dim factor to apply, if any, and how
/// long until that answer may change (only `Auto` ever needs a wake-up).
pub fn resolve<Tz: TimeZone>(
    mode: NightMode,
    schedule: Schedule,
    dim: f32,
    now: &DateTime<Tz>,
) -> (Option<f32>, Option<Duration>) {
    match mode {
        NightMode::Off => (None, None),
        NightMode::On => (Some(dim), None),
        NightMode::Auto => (schedule.active(now.time()).then_some(dim), schedule.until_boundary_in(now)),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn t(text: &str) -> TimeOfDay {
        TimeOfDay::parse(text).unwrap()
    }

    fn at(h: u32, m: u32, s: u32) -> NaiveTime {
        NaiveTime::from_hms_opt(h, m, s).unwrap()
    }

    fn night() -> Schedule {
        Schedule { from: t("22:00"), to: t("07:00") }
    }

    fn utc(h: u32, m: u32, s: u32) -> DateTime<chrono::Utc> {
        chrono::Utc.with_ymd_and_hms(2026, 9, 16, h, m, s).unwrap()
    }

    /// Central European time for 2026, enough to cross both changes:
    /// summer time from 29 March to 25 October, at 01:00 UTC.
    #[derive(Debug, Clone, Copy)]
    pub(crate) struct Cet;

    impl TimeZone for Cet {
        type Offset = chrono::FixedOffset;

        fn from_offset(_: &Self::Offset) -> Self {
            Cet
        }

        fn offset_from_local_date(&self, _: &chrono::NaiveDate) -> LocalResult<Self::Offset> {
            unimplemented!("not needed here")
        }

        fn offset_from_local_datetime(&self, local: &chrono::NaiveDateTime) -> LocalResult<Self::Offset> {
            // Summer time first: the same wall time is earlier under it.
            let fits: Vec<_> = [2, 1]
                .map(|h| chrono::FixedOffset::east_opt(h * 3600).unwrap())
                .into_iter()
                .filter(|offset| self.offset_from_utc_datetime(&(*local - *offset)) == *offset)
                .collect();
            match fits[..] {
                [] => LocalResult::None,
                [one] => LocalResult::Single(one),
                [first, second] => LocalResult::Ambiguous(first, second),
                _ => unreachable!(),
            }
        }

        fn offset_from_utc_date(&self, _: &chrono::NaiveDate) -> Self::Offset {
            unimplemented!("not needed here")
        }

        fn offset_from_utc_datetime(&self, utc: &chrono::NaiveDateTime) -> Self::Offset {
            let at = |m, d| chrono::NaiveDate::from_ymd_opt(2026, m, d).unwrap().and_hms_opt(1, 0, 0).unwrap();
            let summer = (at(3, 29)..at(10, 25)).contains(utc);
            chrono::FixedOffset::east_opt(if summer { 7200 } else { 3600 }).unwrap()
        }
    }

    pub(crate) fn cet(m: u32, d: u32, h: u32, min: u32) -> DateTime<Cet> {
        Cet.with_ymd_and_hms(2026, m, d, h, min, 0).earliest().unwrap()
    }

    /// 23:00 on the night the clocks go forward: 07:00 is eight hours away
    /// on the clock face but seven in real time.
    #[test]
    fn the_wait_is_real_time_across_the_night_the_clocks_go_forward() {
        assert_eq!(night().until_boundary_in(&cet(3, 28, 23, 0)), Some(Duration::from_secs(7 * 3600)));
        assert_eq!(night().until_boundary_in(&cet(10, 24, 23, 0)), Some(Duration::from_secs(9 * 3600)), "and back");
        assert_eq!(night().until_boundary_in(&cet(9, 16, 23, 0)), Some(Duration::from_secs(8 * 3600)), "any other night");
    }

    /// A boundary inside the hour that is skipped takes effect when the
    /// clock jumps over it; one inside the hour that repeats, the first time.
    #[test]
    fn a_boundary_in_the_hour_that_changes_is_met_when_the_clock_gets_there() {
        let at_2_30 = Schedule { from: t("22:00"), to: t("02:30") };
        // Forward: 02:00 CET becomes 03:00 CEST, an hour after 01:00 CET.
        assert_eq!(at_2_30.until_boundary_in(&cet(3, 29, 1, 0)), Some(Duration::from_secs(3600)));
        // Back: 02:30 is shown first at 00:30 UTC, 90 minutes after 01:00 CEST.
        assert_eq!(at_2_30.until_boundary_in(&cet(10, 25, 1, 0)), Some(Duration::from_secs(90 * 60)));
    }

    #[test]
    fn parses_padded_and_unpadded_hours() {
        assert_eq!(TimeOfDay::parse("22:00"), TimeOfDay::new(22, 0));
        assert_eq!(TimeOfDay::parse("7:05"), TimeOfDay::new(7, 5));
        assert_eq!(TimeOfDay::parse("00:00"), TimeOfDay::new(0, 0));
    }

    #[test]
    fn rejects_out_of_range_and_malformed_times() {
        for bad in ["24:00", "12:60", "22", "ab:cd", "", "22:00:00", "-1:00"] {
            assert_eq!(TimeOfDay::parse(bad), None, "{bad:?} must be rejected");
        }
        assert_eq!(TimeOfDay::new(24, 0), None);
        assert_eq!(TimeOfDay::new(0, 60), None);
    }

    #[test]
    fn writes_itself_as_a_padded_string() {
        assert_eq!(ron::to_string(&t("7:05")).unwrap(), "\"07:05\"");
        assert_eq!(ron::from_str::<TimeOfDay>("\"22:30\"").unwrap(), t("22:30"));
        assert!(ron::from_str::<TimeOfDay>("\"25:00\"").is_err());
    }

    #[test]
    fn a_window_over_midnight_covers_both_sides_of_it() {
        assert!(night().active(at(23, 0, 0)));
        assert!(night().active(at(3, 0, 0)));
        assert!(!night().active(at(12, 0, 0)));
    }

    #[test]
    fn the_window_starts_inclusive_and_ends_exclusive() {
        assert!(night().active(at(22, 0, 0)));
        assert!(!night().active(at(21, 59, 59)));
        assert!(!night().active(at(7, 0, 0)));
        assert!(night().active(at(6, 59, 59)));
    }

    #[test]
    fn a_window_within_one_day_does_not_wrap() {
        let s = Schedule { from: t("01:00"), to: t("05:00") };
        assert!(s.active(at(3, 0, 0)));
        assert!(!s.active(at(23, 0, 0)));
        assert!(!s.active(at(0, 30, 0)));
    }

    #[test]
    fn equal_times_never_activate_and_never_wake() {
        let s = Schedule { from: t("22:00"), to: t("22:00") };
        assert!(!s.active(at(22, 0, 0)));
        assert!(!s.active(at(10, 0, 0)));
        assert_eq!(s.until_boundary(at(10, 0, 0)), None);
    }

    #[test]
    fn wakes_at_the_start_of_the_window() {
        assert_eq!(night().until_boundary(at(21, 30, 0)), Some(Duration::from_secs(30 * 60)));
    }

    #[test]
    fn wakes_at_the_end_of_the_window_across_midnight() {
        assert_eq!(night().until_boundary(at(23, 0, 0)), Some(Duration::from_secs(8 * 3600)));
        let half = at(6, 59, 30).with_nanosecond(250_000_000).unwrap();
        assert_eq!(night().until_boundary(half), Some(Duration::from_millis(29_750)));
    }

    #[test]
    fn a_boundary_that_has_just_passed_waits_for_the_next_one() {
        // At exactly 07:00 the window has closed; the next event is 22:00.
        assert_eq!(night().until_boundary(at(7, 0, 0)), Some(Duration::from_secs(15 * 3600)));
        // At exactly 22:00 it has just opened; the next event is 07:00 tomorrow.
        assert_eq!(night().until_boundary(at(22, 0, 0)), Some(Duration::from_secs(9 * 3600)));
    }

    #[test]
    fn off_neither_dims_nor_wakes() {
        assert_eq!(resolve(NightMode::Off, night(), 0.7, &utc(23, 0, 0)), (None, None));
    }

    #[test]
    fn on_dims_at_any_hour_without_waking() {
        assert_eq!(resolve(NightMode::On, night(), 0.7, &utc(12, 0, 0)), (Some(0.7), None));
    }

    #[test]
    fn auto_follows_the_schedule_and_wakes_at_the_next_boundary() {
        assert_eq!(resolve(NightMode::Auto, night(), 0.7, &utc(23, 0, 0)), (Some(0.7), Some(Duration::from_secs(8 * 3600))));
        assert_eq!(resolve(NightMode::Auto, night(), 0.7, &utc(21, 0, 0)), (None, Some(Duration::from_secs(3600))));
    }

    #[test]
    fn night_mode_ids_round_trip() {
        for mode in NightMode::ALL {
            assert_eq!(NightMode::from_id(mode.id()), Some(mode));
        }
        assert_eq!(NightMode::from_id("dusk"), None);
    }
}
