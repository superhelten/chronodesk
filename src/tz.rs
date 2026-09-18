//! Time zones for the market board, computed locally.
//!
//! The board shows a fixed catalogue of exchanges, so instead of shipping the
//! IANA database (a megabyte of transition tables for a five-megabyte binary)
//! each zone is a standard offset plus one of three daylight-saving rules:
//! the US, EU and Australian conventions, which between them cover every
//! exchange on the board. The rules are what the jurisdictions have used for
//! years; if one of them changes its law the rule here changes with a
//! rebuild, exactly as a bundled database would.
//!
//! Everything is arithmetic on `NaiveDateTime` in UTC, so it is deterministic
//! and testable without a clock or a network.

use chrono::{Datelike as _, Duration, NaiveDate, NaiveDateTime, Weekday};

/// Which daylight-saving convention a zone follows, if any.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dst {
    None,
    /// Second Sunday of March to first Sunday of November, at 02:00 local.
    Us,
    /// Last Sunday of March to last Sunday of October, at 01:00 UTC.
    Eu,
    /// Southern hemisphere: first Sunday of October (02:00 standard) to the
    /// first Sunday of April (03:00 daylight).
    Au,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Zone {
    /// Standard offset from UTC in minutes; India's +05:30 is why not hours.
    pub standard: i32,
    pub dst: Dst,
}

const HOUR: i32 = 60;

impl Zone {
    pub const fn new(standard: i32, dst: Dst) -> Self {
        Self { standard, dst }
    }

    /// Offset from UTC in minutes at the instant `utc`.
    pub fn offset(self, utc: NaiveDateTime) -> i32 {
        if self.dst_active(utc) { self.standard + HOUR } else { self.standard }
    }

    pub fn to_local(self, utc: NaiveDateTime) -> NaiveDateTime {
        utc + Duration::minutes(i64::from(self.offset(utc)))
    }

    /// The instant a local wall-clock time refers to. Across a transition the
    /// offset at the *event* is what counts, not the one in force now, so
    /// both candidates are tried and the consistent one kept: the earlier
    /// one in the repeated hour, standard time in the skipped one.
    pub fn to_utc(self, local: NaiveDateTime) -> NaiveDateTime {
        let as_daylight = local - Duration::minutes(i64::from(self.standard + HOUR));
        let as_standard = local - Duration::minutes(i64::from(self.standard));
        if self.dst != Dst::None && self.dst_active(as_daylight) {
            as_daylight
        } else {
            as_standard
        }
    }

    fn dst_active(self, utc: NaiveDateTime) -> bool {
        let year = utc.year();
        let at = |date: NaiveDate, hour: u32| date.and_hms_opt(hour, 0, 0).expect("whole hour");
        let local_to_utc = |local: NaiveDateTime, offset: i32| local - Duration::minutes(i64::from(offset));
        match self.dst {
            Dst::None => false,
            Dst::Us => {
                let start = local_to_utc(at(nth_weekday(year, 3, Weekday::Sun, 2), 2), self.standard);
                let end = local_to_utc(at(nth_weekday(year, 11, Weekday::Sun, 1), 2), self.standard + HOUR);
                (start..end).contains(&utc)
            }
            Dst::Eu => {
                let start = at(last_weekday(year, 3, Weekday::Sun), 1);
                let end = at(last_weekday(year, 10, Weekday::Sun), 1);
                (start..end).contains(&utc)
            }
            Dst::Au => {
                // Daylight time spans the new year, so it is active outside
                // the April–October window rather than inside it.
                let end = local_to_utc(at(nth_weekday(year, 4, Weekday::Sun, 1), 3), self.standard + HOUR);
                let start = local_to_utc(at(nth_weekday(year, 10, Weekday::Sun, 1), 2), self.standard);
                utc < end || utc >= start
            }
        }
    }
}

/// The `n`th (1-based) `weekday` of a month.
fn nth_weekday(year: i32, month: u32, weekday: Weekday, n: u32) -> NaiveDate {
    let first = NaiveDate::from_ymd_opt(year, month, 1).expect("valid month");
    let days_ahead = (7 + weekday.num_days_from_monday() - first.weekday().num_days_from_monday()) % 7;
    first + Duration::days(i64::from(days_ahead + 7 * (n - 1)))
}

fn last_weekday(year: i32, month: u32, weekday: Weekday) -> NaiveDate {
    let next_month = if month == 12 { (year + 1, 1) } else { (year, month + 1) };
    let last = NaiveDate::from_ymd_opt(next_month.0, next_month.1, 1).expect("valid month") - Duration::days(1);
    let days_back = (7 + last.weekday().num_days_from_monday() - weekday.num_days_from_monday()) % 7;
    last - Duration::days(i64::from(days_back))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utc(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(y, mo, d).unwrap().and_hms_opt(h, mi, 0).unwrap()
    }

    const NEW_YORK: Zone = Zone::new(-5 * HOUR, Dst::Us);
    const LONDON: Zone = Zone::new(0, Dst::Eu);
    const OSLO: Zone = Zone::new(HOUR, Dst::Eu);
    const SYDNEY: Zone = Zone::new(10 * HOUR, Dst::Au);
    const TOKYO: Zone = Zone::new(9 * HOUR, Dst::None);
    const MUMBAI: Zone = Zone::new(5 * HOUR + 30, Dst::None);

    #[test]
    fn nth_and_last_weekdays_of_a_month() {
        assert_eq!(nth_weekday(2026, 3, Weekday::Sun, 1), NaiveDate::from_ymd_opt(2026, 3, 1).unwrap());
        assert_eq!(nth_weekday(2026, 3, Weekday::Sun, 2), NaiveDate::from_ymd_opt(2026, 3, 8).unwrap());
        assert_eq!(nth_weekday(2026, 11, Weekday::Sun, 1), NaiveDate::from_ymd_opt(2026, 11, 1).unwrap());
        assert_eq!(last_weekday(2026, 3, Weekday::Sun), NaiveDate::from_ymd_opt(2026, 3, 29).unwrap());
        assert_eq!(last_weekday(2026, 10, Weekday::Sun), NaiveDate::from_ymd_opt(2026, 10, 25).unwrap());
        assert_eq!(last_weekday(2026, 12, Weekday::Sun), NaiveDate::from_ymd_opt(2026, 12, 27).unwrap());
    }

    #[test]
    fn zones_without_daylight_saving_never_move() {
        for (zone, offset) in [(TOKYO, 9 * HOUR), (MUMBAI, 5 * HOUR + 30)] {
            assert_eq!(zone.offset(utc(2026, 1, 15, 12, 0)), offset);
            assert_eq!(zone.offset(utc(2026, 7, 15, 12, 0)), offset);
        }
        assert_eq!(MUMBAI.to_local(utc(2026, 9, 18, 4, 0)), utc(2026, 9, 18, 9, 30));
    }

    /// 2026: US daylight time runs from 8 March 07:00 UTC to 1 November 06:00 UTC.
    #[test]
    fn us_rule_switches_at_two_in_the_morning_local() {
        assert_eq!(NEW_YORK.offset(utc(2026, 3, 8, 6, 59)), -5 * HOUR, "still EST at 01:59");
        assert_eq!(NEW_YORK.offset(utc(2026, 3, 8, 7, 0)), -4 * HOUR, "EDT from 02:00 EST");
        assert_eq!(NEW_YORK.offset(utc(2026, 11, 1, 5, 59)), -4 * HOUR, "still EDT at 01:59 EDT");
        assert_eq!(NEW_YORK.offset(utc(2026, 11, 1, 6, 0)), -5 * HOUR, "EST from 02:00 EDT");
        assert_eq!(NEW_YORK.offset(utc(2026, 1, 1, 0, 0)), -5 * HOUR);
    }

    /// 2026: EU summer time runs from 29 March to 25 October, both at 01:00 UTC.
    #[test]
    fn eu_rule_switches_at_one_utc_for_every_member() {
        for (zone, winter) in [(LONDON, 0), (OSLO, HOUR)] {
            assert_eq!(zone.offset(utc(2026, 3, 29, 0, 59)), winter);
            assert_eq!(zone.offset(utc(2026, 3, 29, 1, 0)), winter + HOUR);
            assert_eq!(zone.offset(utc(2026, 10, 25, 0, 59)), winter + HOUR);
            assert_eq!(zone.offset(utc(2026, 10, 25, 1, 0)), winter);
        }
    }

    /// 2026: Sydney leaves daylight time on Sunday 5 April at 03:00 AEDT
    /// (Saturday 16:00 UTC) and returns on Sunday 4 October at 02:00 AEST
    /// (Saturday 16:00 UTC).
    #[test]
    fn australian_rule_spans_the_new_year() {
        assert_eq!(SYDNEY.offset(utc(2026, 1, 15, 0, 0)), 11 * HOUR, "summer in January");
        assert_eq!(SYDNEY.offset(utc(2026, 4, 4, 15, 59)), 11 * HOUR);
        assert_eq!(SYDNEY.offset(utc(2026, 4, 4, 16, 0)), 10 * HOUR);
        assert_eq!(SYDNEY.offset(utc(2026, 7, 15, 0, 0)), 10 * HOUR, "winter in July");
        assert_eq!(SYDNEY.offset(utc(2026, 10, 3, 15, 59)), 10 * HOUR);
        assert_eq!(SYDNEY.offset(utc(2026, 10, 3, 16, 0)), 11 * HOUR);
        assert_eq!(SYDNEY.offset(utc(2026, 12, 31, 23, 59)), 11 * HOUR);
    }

    #[test]
    fn local_time_follows_the_offset() {
        assert_eq!(NEW_YORK.to_local(utc(2026, 9, 18, 13, 30)), utc(2026, 9, 18, 9, 30));
        assert_eq!(NEW_YORK.to_local(utc(2026, 1, 18, 14, 30)), utc(2026, 1, 18, 9, 30));
        assert_eq!(SYDNEY.to_local(utc(2026, 9, 17, 23, 0)), utc(2026, 9, 18, 9, 0));
    }

    #[test]
    fn local_to_utc_round_trips_away_from_transitions() {
        for zone in [NEW_YORK, LONDON, OSLO, SYDNEY, TOKYO, MUMBAI] {
            for t in [utc(2026, 2, 3, 9, 30), utc(2026, 7, 3, 16, 0), utc(2026, 12, 24, 12, 0)] {
                assert_eq!(zone.to_utc(zone.to_local(t)), t, "{zone:?} at {t}");
            }
        }
    }

    /// An event on the far side of a transition must use the offset in force
    /// *then*: Monday's 09:30 open after the March switch is an hour earlier
    /// in UTC than Friday's arithmetic would suggest.
    #[test]
    fn local_to_utc_uses_the_offset_at_the_event() {
        // Monday 9 March 2026 09:30 EDT is 13:30 UTC, not 14:30.
        assert_eq!(NEW_YORK.to_utc(utc(2026, 3, 9, 9, 30)), utc(2026, 3, 9, 13, 30));
        // Friday before the switch is still standard time.
        assert_eq!(NEW_YORK.to_utc(utc(2026, 3, 6, 16, 0)), utc(2026, 3, 6, 21, 0));
        // Sydney: Monday 6 April 2026 10:00 AEST is Sunday 00:00 UTC, not 23:00 Saturday.
        assert_eq!(SYDNEY.to_utc(utc(2026, 4, 6, 10, 0)), utc(2026, 4, 6, 0, 0));
        assert_eq!(SYDNEY.to_utc(utc(2026, 4, 3, 16, 0)), utc(2026, 4, 3, 5, 0));
    }

    #[test]
    fn the_repeated_hour_resolves_to_its_first_occurrence() {
        // 01:30 on 1 November 2026 happens twice in New York; the daylight
        // reading (05:30 UTC) comes first.
        assert_eq!(NEW_YORK.to_utc(utc(2026, 11, 1, 1, 30)), utc(2026, 11, 1, 5, 30));
    }

    #[test]
    fn the_skipped_hour_resolves_as_standard_time() {
        // 02:30 on 8 March 2026 does not exist in New York; read as EST it is
        // 07:30 UTC, which the clock shows as 03:30 EDT.
        let instant = NEW_YORK.to_utc(utc(2026, 3, 8, 2, 30));
        assert_eq!(instant, utc(2026, 3, 8, 7, 30));
        assert_eq!(NEW_YORK.to_local(instant), utc(2026, 3, 8, 3, 30));
    }
}
