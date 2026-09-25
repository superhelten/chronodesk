//! The alarm: three chimes at a time of day picked from the menu.
//!
//! It rings once, not every day: picking a time arms it for the next moment
//! the clock shows that time, and ringing (or a click) disarms it. What is
//! armed is a real instant, worked out when the time is picked, so the hour
//! the clocks skip or repeat is dealt with once, and an overlay that was
//! closed or asleep at that moment does not ring late when it comes back.

use std::time::Duration;

use chrono::{DateTime, TimeZone};

use crate::night::{TimeOfDay, resolve_ahead};

/// How long a due alarm rings: the three chimes, ten seconds apart, under a
/// blinking frame, as long as a finished timer blinks.
pub const RING_FOR: Duration = Duration::from_secs(30);

/// The next instant `now`'s clock shows `at`, in epoch seconds. A time the
/// clock already shows, or has passed today, is tomorrow's.
pub fn arm<Tz: TimeZone>(at: TimeOfDay, now: &DateTime<Tz>) -> i64 {
    let local = now.naive_local();
    let mut day = local.date();
    for _ in 0..2 {
        let target = day.and_time(at.time());
        if target > local
            && let Some(real) = resolve_ahead(&target, now)
            && real > *now
        {
            return real.timestamp();
        }
        day = day.succ_opt().unwrap_or(day);
    }
    now.timestamp() + 24 * 3600
}

/// How long ago the alarm fell due, once it has.
pub fn overtime(due: i64, now_ms: i64) -> Option<Duration> {
    let ms = now_ms.checked_sub(due.checked_mul(1000)?)?;
    (ms >= 0).then(|| Duration::from_millis(ms as u64))
}

/// How long until the alarm falls due, while it has not.
pub fn until(due: i64, now_ms: i64) -> Option<Duration> {
    let ms = due.checked_mul(1000)?.checked_sub(now_ms)?;
    (ms > 0).then(|| Duration::from_millis(ms as u64))
}

/// The menu's switch: the time, and whether it is today's or tomorrow's
/// once armed, so a time already past today does not look like a mistake.
pub fn label<Tz: TimeZone>(at: TimeOfDay, due: Option<i64>, now: &DateTime<Tz>) -> String {
    let day = due.and_then(|due| now.timezone().timestamp_opt(due, 0).single()).map(|when| {
        if when.date_naive() == now.date_naive() { " (today)" } else { " (tomorrow)" }
    });
    format!("Alarm at {at}{}", day.unwrap_or(""))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::night::tests::{Cet, cet};

    fn t(text: &str) -> TimeOfDay {
        TimeOfDay::parse(text).unwrap()
    }

    fn wait(due: i64, now: &DateTime<Cet>) -> Duration {
        until(due, now.timestamp_millis()).expect("armed ahead")
    }

    #[test]
    fn a_time_still_ahead_is_today_and_one_gone_is_tomorrow() {
        let now = cet(9, 25, 9, 0);
        assert_eq!(wait(arm(t("14:30"), &now), &now), Duration::from_secs(5 * 3600 + 30 * 60));
        assert_eq!(wait(arm(t("08:00"), &now), &now), Duration::from_secs(23 * 3600));
        // The minute the clock shows right now has already begun: tomorrow.
        let now = cet(9, 25, 14, 30) + chrono::Duration::seconds(10);
        assert_eq!(wait(arm(t("14:30"), &now), &now), Duration::from_secs(24 * 3600 - 10));
    }

    /// 02:30 does not exist the night the clocks go forward: it rings as the
    /// clock jumps from 02:00 to 03:00. The night they go back it is shown
    /// twice, and rings the first time.
    #[test]
    fn the_hour_that_changes_rings_once_when_the_clock_gets_there() {
        let now = cet(3, 29, 1, 0);
        assert_eq!(wait(arm(t("02:30"), &now), &now), Duration::from_secs(3600));
        let now = cet(10, 25, 1, 0);
        assert_eq!(wait(arm(t("02:30"), &now), &now), Duration::from_secs(90 * 60));
        // Between the two showings: armed at 02:20 the second time round, it
        // is the second 02:30, ten minutes off, and not one long gone.
        let second = Cet.with_ymd_and_hms(2026, 10, 25, 2, 20, 0).latest().unwrap();
        assert_eq!(wait(arm(t("02:30"), &second), &second), Duration::from_secs(10 * 60));
    }

    #[test]
    fn it_is_due_from_the_armed_second_on() {
        let due = 1_000;
        assert_eq!(until(due, 999_000), Some(Duration::from_secs(1)));
        assert_eq!(until(due, 1_000_000), None);
        assert_eq!(overtime(due, 999_999), None);
        assert_eq!(overtime(due, 1_000_000), Some(Duration::ZERO));
        assert_eq!(overtime(due, 1_012_500), Some(Duration::from_millis(12_500)));
    }

    #[test]
    fn the_menu_says_which_day_it_rings() {
        let now = cet(9, 25, 9, 0);
        assert_eq!(label(t("14:30"), None, &now), "Alarm at 14:30");
        assert_eq!(label(t("14:30"), Some(arm(t("14:30"), &now)), &now), "Alarm at 14:30 (today)");
        assert_eq!(label(t("07:05"), Some(arm(t("07:05"), &now)), &now), "Alarm at 07:05 (tomorrow)");
    }
}
