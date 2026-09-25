//! The clock readout as a pure function of a timestamp and the display
//! options, so its formatting can be tested without a live clock.

use std::time::Duration;

use std::fmt::Display;

use chrono::{DateTime, NaiveDateTime, TimeZone, Timelike as _};
use serde::{Deserialize, Serialize};

use crate::app::serde_by_id;
use crate::world::City;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ClockFormat {
    #[default]
    H24,
    H12,
}

impl ClockFormat {
    pub const ALL: [Self; 2] = [Self::H24, Self::H12];

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|f| f.id().eq_ignore_ascii_case(id))
    }

    pub fn id(self) -> &'static str {
        match self {
            Self::H24 => "24h",
            Self::H12 => "12h",
        }
    }

    pub fn toggled(self) -> Self {
        match self {
            Self::H24 => Self::H12,
            Self::H12 => Self::H24,
        }
    }
}
serde_by_id!(ClockFormat, "clock format");

/// What the clock mode shows, apart from the time itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClockStyle {
    pub format: ClockFormat,
    pub show_seconds: bool,
    pub show_date: bool,
    /// A second time zone, shown after the date.
    pub zone: Option<City>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClockReadout {
    pub main: String,
    /// Upper-case caption line; empty when there is nothing to show.
    pub caption: String,
    /// Time until either line changes.
    pub until_change: Duration,
}

/// Formats `t` for the overlay. `main` holds only readout characters (digits
/// and colons) so the glyph cache covers it; AM/PM goes in the caption.
pub fn clock_readout<Tz: TimeZone>(t: DateTime<Tz>, style: ClockStyle) -> ClockReadout
where
    Tz::Offset: Display,
{
    let main = match (style.format, style.show_seconds) {
        (ClockFormat::H24, true) => t.format("%H:%M:%S"),
        (ClockFormat::H24, false) => t.format("%H:%M"),
        (ClockFormat::H12, true) => t.format("%-I:%M:%S"),
        (ClockFormat::H12, false) => t.format("%-I:%M"),
    }
    .to_string();

    let mut parts: Vec<String> = Vec::with_capacity(2);
    if style.format == ClockFormat::H12 {
        parts.push(t.format("%p").to_string());
    }
    if style.show_date {
        parts.push(t.format("%a %-d %b").to_string());
    }
    if let Some(city) = style.zone {
        parts.push(elsewhere(city, t.naive_utc(), t.naive_local(), style.format));
    }
    let caption = parts.join(" · ").to_uppercase();

    let nanos_into_second = u64::from(t.nanosecond() % 1_000_000_000);
    let until_change = if style.show_seconds {
        1_000_000_000 - nanos_into_second
    } else {
        60_000_000_000 - (u64::from(t.second()) * 1_000_000_000 + nanos_into_second)
    };

    ClockReadout { main, caption, until_change: Duration::from_nanos(until_change) }
}

/// The time in `city`, minutes only, with a day marker when its date is not
/// the local one: "TOKYO 03:30 +1".
fn elsewhere(city: City, utc: NaiveDateTime, local: NaiveDateTime, format: ClockFormat) -> String {
    let there = city.zone().to_local(utc);
    let time = match format {
        ClockFormat::H24 => there.format("%H:%M"),
        ClockFormat::H12 => there.format("%-I:%M %p"),
    };
    let mut text = format!("{} {time}", city.label());
    let days = (there.date() - local.date()).num_days();
    if days != 0 {
        text.push_str(&format!(" {days:+}"));
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{FixedOffset, Local};

    use crate::world::City;

    fn at(h: u32, m: u32, s: u32) -> DateTime<Local> {
        // September: safely outside any DST transition.
        Local.with_ymd_and_hms(2026, 9, 16, h, m, s).unwrap()
    }

    fn style(format: ClockFormat, show_seconds: bool, show_date: bool) -> ClockStyle {
        ClockStyle { format, show_seconds, show_date, zone: None }
    }

    /// Local time two hours ahead of UTC, as in central Europe in summer, pinned
    /// so the other zone's time does not depend on the machine's.
    fn cest(d: u32, h: u32, m: u32) -> DateTime<FixedOffset> {
        FixedOffset::east_opt(2 * 3600).unwrap().with_ymd_and_hms(2026, 9, d, h, m, 0).unwrap()
    }

    fn with_zone(style: ClockStyle, city: City) -> ClockStyle {
        ClockStyle { zone: Some(city), ..style }
    }

    const H24: ClockFormat = ClockFormat::H24;
    const H12: ClockFormat = ClockFormat::H12;

    #[test]
    fn twenty_four_hour_readout_is_unchanged() {
        let r = clock_readout(at(18, 49, 37), style(H24, true, true));
        assert_eq!(r.main, "18:49:37");
        assert_eq!(r.caption, "WED 16 SEP");
    }

    #[test]
    fn twelve_hour_readout_drops_the_leading_zero_and_moves_am_pm_to_the_caption() {
        let r = clock_readout(at(18, 49, 37), style(H12, true, true));
        assert_eq!(r.main, "6:49:37");
        assert_eq!(r.caption, "PM · WED 16 SEP");
        assert!(r.main.chars().all(|c| crate::layout::READOUT_CHARS.contains(c)), "{}", r.main);
    }

    #[test]
    fn am_pm_flips_at_noon_and_at_midnight() {
        assert_eq!(clock_readout(at(11, 59, 59), style(H12, true, false)).caption, "AM");
        assert_eq!(clock_readout(at(12, 0, 0), style(H12, true, false)).caption, "PM");
        assert_eq!(clock_readout(at(23, 59, 59), style(H12, true, false)).caption, "PM");
        assert_eq!(clock_readout(at(0, 0, 0), style(H12, true, false)).caption, "AM");
    }

    #[test]
    fn twelve_hour_clock_shows_twelve_not_zero_around_midnight_and_noon() {
        assert_eq!(clock_readout(at(0, 5, 0), style(H12, false, false)).main, "12:05");
        assert_eq!(clock_readout(at(12, 5, 0), style(H12, false, false)).main, "12:05");
    }

    #[test]
    fn nine_to_ten_o_clock_changes_the_width_only_in_twelve_hour_mode() {
        let nine = clock_readout(at(9, 59, 0), style(H12, false, false)).main;
        let ten = clock_readout(at(10, 0, 0), style(H12, false, false)).main;
        assert_eq!((nine.as_str(), ten.as_str()), ("9:59", "10:00"));
        assert_ne!(nine.len(), ten.len(), "one digit more at ten o'clock");

        let nine = clock_readout(at(9, 59, 0), style(H24, false, false)).main;
        let ten = clock_readout(at(10, 0, 0), style(H24, false, false)).main;
        assert_eq!((nine.as_str(), ten.as_str()), ("09:59", "10:00"));
    }

    #[test]
    fn hiding_the_date_empties_the_caption_in_twenty_four_hour_mode() {
        let r = clock_readout(at(18, 49, 37), style(H24, true, false));
        assert_eq!(r.main, "18:49:37");
        assert_eq!(r.caption, "");
    }

    #[test]
    fn hiding_the_date_keeps_am_pm_in_twelve_hour_mode() {
        let r = clock_readout(at(18, 49, 37), style(H12, true, false));
        assert_eq!(r.caption, "PM");
    }

    #[test]
    fn next_change_is_the_rest_of_the_second_or_minute() {
        let t = at(18, 49, 37).with_nanosecond(250_000_000).unwrap();
        assert_eq!(clock_readout(t, style(H24, true, true)).until_change, Duration::from_millis(750));
        assert_eq!(clock_readout(t, style(H24, false, true)).until_change, Duration::from_millis(22_750));
    }

    #[test]
    fn midnight_is_reached_one_second_after_the_last_second_of_the_day() {
        let r = clock_readout(at(23, 59, 59), style(H12, true, true));
        assert_eq!(r.until_change, Duration::from_secs(1));
        let next = clock_readout(at(23, 59, 59) + chrono::Duration::seconds(1), style(H12, true, true));
        assert_eq!(next.main, "12:00:00");
        assert_eq!(next.caption, "AM · THU 17 SEP", "the date must roll over with the clock");
    }

    #[test]
    fn a_second_zone_follows_the_date_in_the_caption() {
        let r = clock_readout(cest(16, 18, 49), with_zone(style(H24, false, true), City::Utc));
        assert_eq!(r.main, "18:49", "local time stays the readout");
        assert_eq!(r.caption, "WED 16 SEP · UTC 16:49");
    }

    #[test]
    fn a_second_zone_a_day_ahead_or_behind_says_so() {
        let tokyo = clock_readout(cest(16, 20, 30), with_zone(style(H24, false, false), City::Tokyo));
        assert_eq!(tokyo.caption, "TOKYO 03:30 +1");
        let la = clock_readout(cest(17, 1, 0), with_zone(style(H24, false, false), City::LosAngeles));
        assert_eq!(la.caption, "LOS ANGELES 16:00 -1");
    }

    #[test]
    fn a_second_zone_in_twelve_hour_mode_carries_its_own_period() {
        let r = clock_readout(cest(16, 20, 30), with_zone(style(H12, true, true), City::Tokyo));
        assert_eq!(r.main, "8:30:00");
        assert_eq!(r.caption, "PM · WED 16 SEP · TOKYO 3:30 AM +1");
    }

    /// Offsets are whole half hours, so the other zone turns its minute
    /// when the local clock does and needs no wake of its own.
    #[test]
    fn a_second_zone_changes_nothing_about_when_to_draw_next() {
        let t = cest(16, 18, 49).with_second(37).unwrap();
        for show_seconds in [true, false] {
            let plain = clock_readout(t, style(H24, show_seconds, true)).until_change;
            let zoned = clock_readout(t, with_zone(style(H24, show_seconds, true), City::Mumbai)).until_change;
            assert_eq!(plain, zoned);
        }
        assert_eq!(clock_readout(t, with_zone(style(H24, false, false), City::Mumbai)).caption, "MUMBAI 22:19");
    }

    #[test]
    fn a_second_zone_observes_its_own_daylight_saving() {
        let winter = FixedOffset::east_opt(3600).unwrap().with_ymd_and_hms(2026, 1, 15, 15, 0, 0).unwrap();
        assert_eq!(clock_readout(winter, with_zone(style(H24, false, false), City::NewYork)).caption, "NEW YORK 09:00");
        assert_eq!(clock_readout(cest(16, 15, 0), with_zone(style(H24, false, false), City::NewYork)).caption, "NEW YORK 09:00");
    }
}
