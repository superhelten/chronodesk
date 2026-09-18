//! The market board: a fixed catalogue of exchanges, each with a time zone
//! and a trading session, and the board readout as a pure function of a UTC
//! timestamp so every row and the countdown caption can be unit-tested.
//!
//! What is modelled: the regular Monday–Friday session, a midday break where
//! the exchange has one, and daylight-saving time. What is not: holidays and
//! half-day closes. Those need a calendar per exchange that changes every
//! year, which is exactly the kind of data this app does not ship or fetch.

use std::time::Duration;

use chrono::{Datelike as _, NaiveDateTime, NaiveTime, Timelike as _, Weekday};
use serde::{Deserialize, Serialize};

use crate::app::serde_by_id;
use crate::clock::ClockFormat;
use crate::tz::{Dst, Zone};

const HOUR: i32 = 60;

/// Every exchange the board can show, west to east.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Market {
    NewYork,
    London,
    Oslo,
    Frankfurt,
    Mumbai,
    Shanghai,
    HongKong,
    Tokyo,
    Sydney,
}

impl Market {
    pub const ALL: [Self; 9] = [
        Self::NewYork,
        Self::London,
        Self::Oslo,
        Self::Frankfurt,
        Self::Mumbai,
        Self::Shanghai,
        Self::HongKong,
        Self::Tokyo,
        Self::Sydney,
    ];
    /// One row per continent that trades.
    pub const DEFAULT: [Self; 5] = [Self::NewYork, Self::London, Self::Oslo, Self::Tokyo, Self::Sydney];

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.id().eq_ignore_ascii_case(id))
    }

    pub fn id(self) -> &'static str {
        match self {
            Self::NewYork => "new-york",
            Self::London => "london",
            Self::Oslo => "oslo",
            Self::Frankfurt => "frankfurt",
            Self::Mumbai => "mumbai",
            Self::Shanghai => "shanghai",
            Self::HongKong => "hong-kong",
            Self::Tokyo => "tokyo",
            Self::Sydney => "sydney",
        }
    }

    /// The row label: the city, upper-case like every caption.
    pub fn city(self) -> &'static str {
        match self {
            Self::NewYork => "NEW YORK",
            Self::London => "LONDON",
            Self::Oslo => "OSLO",
            Self::Frankfurt => "FRANKFURT",
            Self::Mumbai => "MUMBAI",
            Self::Shanghai => "SHANGHAI",
            Self::HongKong => "HONG KONG",
            Self::Tokyo => "TOKYO",
            Self::Sydney => "SYDNEY",
        }
    }

    /// Menu label: city plus the exchange, since a city can host several.
    pub fn label(self) -> &'static str {
        match self {
            Self::NewYork => "New York (NYSE)",
            Self::London => "London (LSE)",
            Self::Oslo => "Oslo (Euronext Oslo)",
            Self::Frankfurt => "Frankfurt (Xetra)",
            Self::Mumbai => "Mumbai (NSE)",
            Self::Shanghai => "Shanghai (SSE)",
            Self::HongKong => "Hong Kong (HKEX)",
            Self::Tokyo => "Tokyo (TSE)",
            Self::Sydney => "Sydney (ASX)",
        }
    }

    pub fn zone(self) -> Zone {
        match self {
            Self::NewYork => Zone::new(-5 * HOUR, Dst::Us),
            Self::London => Zone::new(0, Dst::Eu),
            Self::Oslo | Self::Frankfurt => Zone::new(HOUR, Dst::Eu),
            Self::Mumbai => Zone::new(5 * HOUR + 30, Dst::None),
            Self::Shanghai | Self::HongKong => Zone::new(8 * HOUR, Dst::None),
            Self::Tokyo => Zone::new(9 * HOUR, Dst::None),
            Self::Sydney => Zone::new(10 * HOUR, Dst::Au),
        }
    }

    /// The regular continuous session, closing auction included where the
    /// exchange runs one (Oslo's 16:20–16:30 is part of its trading day).
    pub fn session(self) -> Session {
        match self {
            Self::NewYork => Session::new(hm(9, 30), hm(16, 0), None),
            Self::London => Session::new(hm(8, 0), hm(16, 30), None),
            Self::Oslo => Session::new(hm(9, 0), hm(16, 30), None),
            Self::Frankfurt => Session::new(hm(9, 0), hm(17, 30), None),
            Self::Mumbai => Session::new(hm(9, 15), hm(15, 30), None),
            Self::Shanghai => Session::new(hm(9, 30), hm(15, 0), Some((hm(11, 30), hm(13, 0)))),
            Self::HongKong => Session::new(hm(9, 30), hm(16, 0), Some((hm(12, 0), hm(13, 0)))),
            Self::Tokyo => Session::new(hm(9, 0), hm(15, 30), Some((hm(11, 30), hm(12, 30)))),
            Self::Sydney => Session::new(hm(10, 0), hm(16, 0), None),
        }
    }
}
serde_by_id!(Market, "market");

/// Minutes since local midnight.
const fn hm(hour: u16, minute: u16) -> u16 {
    hour * 60 + minute
}

/// A weekday trading session in local wall-clock minutes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Session {
    open: u16,
    close: u16,
    /// Midday break, `[start, end)`.
    lunch: Option<(u16, u16)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Open,
    /// Between the morning and afternoon sessions.
    Break,
    Closed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    Opens,
    Closes,
}

impl Event {
    fn verb(self) -> &'static str {
        match self {
            Self::Opens => "OPENS",
            Self::Closes => "CLOSES",
        }
    }
}

impl Session {
    const fn new(open: u16, close: u16, lunch: Option<(u16, u16)>) -> Self {
        Self { open, close, lunch }
    }

    pub fn status(self, local: NaiveDateTime) -> Status {
        if !is_trading_day(local.weekday()) {
            return Status::Closed;
        }
        let now = minute_of_day(local.time());
        if !(self.open..self.close).contains(&now) {
            Status::Closed
        } else if self.lunch.is_some_and(|(start, end)| (start..end).contains(&now)) {
            Status::Break
        } else {
            Status::Open
        }
    }

    /// The next open or close after `local`. Breaks are a status, not an
    /// event: "Tokyo resumes in 12 min" is not what a glance is for.
    pub fn next_event(self, local: NaiveDateTime) -> (Event, NaiveDateTime) {
        let now = minute_of_day(local.time());
        let today = local.date();
        if is_trading_day(today.weekday()) {
            if now < self.open {
                return (Event::Opens, today.and_time(at_minute(self.open)));
            }
            if now < self.close {
                return (Event::Closes, today.and_time(at_minute(self.close)));
            }
        }
        let mut day = today.succ_opt().expect("date in range");
        while !is_trading_day(day.weekday()) {
            day = day.succ_opt().expect("date in range");
        }
        (Event::Opens, day.and_time(at_minute(self.open)))
    }
}

fn is_trading_day(day: Weekday) -> bool {
    !matches!(day, Weekday::Sat | Weekday::Sun)
}

fn minute_of_day(t: NaiveTime) -> u16 {
    (t.hour() * 60 + t.minute()) as u16
}

fn at_minute(minute: u16) -> NaiveTime {
    NaiveTime::from_hms_opt(u32::from(minute) / 60, u32::from(minute) % 60, 0).expect("valid minute")
}

/// How the times on the board are written; the same choices as the clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BoardStyle {
    pub format: ClockFormat,
    pub show_seconds: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub market: Market,
    /// Readout characters only, so the glyph cache covers it.
    pub time: String,
    /// `AM`/`PM` in twelve-hour mode.
    pub period: Option<&'static str>,
    pub status: Status,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Board {
    pub rows: Vec<Row>,
    /// The soonest open or close across the rows, e.g. `NEW YORK OPENS IN 2H 14M`.
    pub caption: String,
    /// Time until any row or the caption changes.
    pub until_change: Duration,
}

/// The board at the instant `utc`, one row per market in the order given.
pub fn board(utc: NaiveDateTime, markets: &[Market], style: BoardStyle) -> Board {
    let mut soonest: Option<(Market, Event, chrono::Duration)> = None;
    let rows = markets
        .iter()
        .map(|&market| {
            let (zone, session) = (market.zone(), market.session());
            let local = zone.to_local(utc);
            let (event, at) = session.next_event(local);
            let wait = zone.to_utc(at).signed_duration_since(utc);
            if soonest.is_none_or(|(_, _, best)| wait < best) {
                soonest = Some((market, event, wait));
            }
            let (time, period) = match (style.format, style.show_seconds) {
                (ClockFormat::H24, true) => (local.format("%H:%M:%S").to_string(), None),
                (ClockFormat::H24, false) => (local.format("%H:%M").to_string(), None),
                (ClockFormat::H12, true) => (local.format("%-I:%M:%S").to_string(), Some(period(local.time()))),
                (ClockFormat::H12, false) => (local.format("%-I:%M").to_string(), Some(period(local.time()))),
            };
            Row { market, time, period, status: session.status(local) }
        })
        .collect();

    let caption = soonest
        .map(|(market, event, wait)| {
            let wait = wait.to_std().unwrap_or_default();
            format!("{} {} IN {}", market.city(), event.verb(), countdown(wait))
        })
        .unwrap_or_default();

    let nanos_into_second = u64::from(utc.nanosecond() % 1_000_000_000);
    let until_change = if style.show_seconds {
        1_000_000_000 - nanos_into_second
    } else {
        60_000_000_000 - (u64::from(utc.second()) * 1_000_000_000 + nanos_into_second)
    };

    Board { rows, caption, until_change: Duration::from_nanos(until_change) }
}

fn period(t: NaiveTime) -> &'static str {
    if t.hour() < 12 { "AM" } else { "PM" }
}

/// `2H 14M`, `42M`, or `1D 9H` from a day out; rounded up so `1M` is shown
/// right until the event and the status flips as it reaches zero.
pub fn countdown(wait: Duration) -> String {
    let minutes = wait.as_secs().div_ceil(60).max(1);
    let (days, hours, mins) = (minutes / 1440, (minutes % 1440) / 60, minutes % 60);
    match (days, hours, mins) {
        (0, 0, m) => format!("{m}M"),
        (0, h, 0) => format!("{h}H"),
        (0, h, m) => format!("{h}H {m}M"),
        (d, 0, _) => format!("{d}D"),
        (d, h, _) => format!("{d}D {h}H"),
    }
}

/// The widest captions the board can produce for `markets`, so the window
/// can be sized once instead of following the countdown minute by minute.
/// `digit` is the widest digit of the caption face.
pub fn widest_captions(markets: &[Market], digit: char) -> Vec<String> {
    let d = digit;
    markets
        .iter()
        .flat_map(|m| [format!("{} CLOSES IN {d}{d}H {d}{d}M", m.city()), format!("{} OPENS IN {d}D {d}{d}H", m.city())])
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn at(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(y, mo, d).unwrap().and_hms_opt(h, mi, 0).unwrap()
    }

    fn style(format: ClockFormat, show_seconds: bool) -> BoardStyle {
        BoardStyle { format, show_seconds }
    }

    const H24: BoardStyle = BoardStyle { format: ClockFormat::H24, show_seconds: false };

    #[test]
    fn market_ids_round_trip_and_the_default_is_a_subset() {
        for market in Market::ALL {
            assert_eq!(Market::from_id(market.id()), Some(market));
            assert!(market.city().chars().all(|c| c.is_ascii_uppercase() || c == ' '), "{market:?}");
        }
        assert_eq!(Market::from_id("mars"), None);
        for market in Market::DEFAULT {
            assert!(Market::ALL.contains(&market));
        }
    }

    #[test]
    fn a_plain_session_is_open_between_its_bell_times() {
        let s = Market::NewYork.session();
        assert_eq!(s.status(at(2026, 9, 18, 9, 29)), Status::Closed, "Friday before the bell");
        assert_eq!(s.status(at(2026, 9, 18, 9, 30)), Status::Open);
        assert_eq!(s.status(at(2026, 9, 18, 15, 59)), Status::Open);
        assert_eq!(s.status(at(2026, 9, 18, 16, 0)), Status::Closed, "the close is exclusive");
        assert_eq!(s.status(at(2026, 9, 19, 12, 0)), Status::Closed, "Saturday");
        assert_eq!(s.status(at(2026, 9, 20, 12, 0)), Status::Closed, "Sunday");
    }

    #[test]
    fn a_midday_break_reads_as_a_break_not_a_close() {
        let s = Market::Tokyo.session();
        assert_eq!(s.status(at(2026, 9, 18, 11, 29)), Status::Open);
        assert_eq!(s.status(at(2026, 9, 18, 11, 30)), Status::Break);
        assert_eq!(s.status(at(2026, 9, 18, 12, 29)), Status::Break);
        assert_eq!(s.status(at(2026, 9, 18, 12, 30)), Status::Open);
        assert_eq!(s.status(at(2026, 9, 18, 15, 30)), Status::Closed);
    }

    #[test]
    fn the_next_event_walks_through_a_trading_day() {
        let s = Market::London.session();
        assert_eq!(s.next_event(at(2026, 9, 18, 7, 0)), (Event::Opens, at(2026, 9, 18, 8, 0)));
        assert_eq!(s.next_event(at(2026, 9, 18, 8, 0)), (Event::Closes, at(2026, 9, 18, 16, 30)));
        assert_eq!(s.next_event(at(2026, 9, 18, 12, 0)), (Event::Closes, at(2026, 9, 18, 16, 30)));
        // Friday after the close: Monday's open, skipping the weekend.
        assert_eq!(s.next_event(at(2026, 9, 18, 16, 30)), (Event::Opens, at(2026, 9, 21, 8, 0)));
        assert_eq!(s.next_event(at(2026, 9, 19, 3, 0)), (Event::Opens, at(2026, 9, 21, 8, 0)));
    }

    #[test]
    fn a_break_is_not_an_event() {
        let s = Market::Tokyo.session();
        assert_eq!(s.next_event(at(2026, 9, 18, 11, 0)), (Event::Closes, at(2026, 9, 18, 15, 30)));
        assert_eq!(s.next_event(at(2026, 9, 18, 12, 0)), (Event::Closes, at(2026, 9, 18, 15, 30)));
    }

    #[test]
    fn countdown_rounds_up_and_drops_empty_units() {
        assert_eq!(countdown(Duration::from_secs(1)), "1M");
        assert_eq!(countdown(Duration::ZERO), "1M");
        assert_eq!(countdown(Duration::from_secs(42 * 60)), "42M");
        assert_eq!(countdown(Duration::from_secs(41 * 60 + 1)), "42M");
        assert_eq!(countdown(Duration::from_secs(2 * 3600)), "2H");
        assert_eq!(countdown(Duration::from_secs(2 * 3600 + 14 * 60)), "2H 14M");
        assert_eq!(countdown(Duration::from_secs(24 * 3600)), "1D");
        assert_eq!(countdown(Duration::from_secs(33 * 3600 + 5 * 60)), "1D 9H");
    }

    /// Friday 18 September 2026, 14:00 UTC: New York has just opened, Europe
    /// is in the afternoon, Asia has closed, Sydney is past midnight.
    #[test]
    fn the_board_shows_each_market_in_its_own_time() {
        let b = board(at(2026, 9, 18, 14, 0), &Market::DEFAULT, H24);
        let rows: Vec<(&str, &str, Status)> = b.rows.iter().map(|r| (r.market.city(), r.time.as_str(), r.status)).collect();
        assert_eq!(
            rows,
            vec![
                ("NEW YORK", "10:00", Status::Open),
                ("LONDON", "15:00", Status::Open),
                ("OSLO", "16:00", Status::Open),
                ("TOKYO", "23:00", Status::Closed),
                ("SYDNEY", "00:00", Status::Closed),
            ]
        );
        assert!(b.rows.iter().all(|r| r.period.is_none()));
        assert!(b.rows.iter().all(|r| r.time.chars().all(|c| crate::layout::READOUT_CHARS.contains(c))));
    }

    #[test]
    fn the_caption_names_the_soonest_event_across_the_board() {
        // 14:00 UTC Friday: London closes 15:30 UTC, Oslo 14:30 UTC, New York 20:00 UTC.
        let b = board(at(2026, 9, 18, 14, 0), &Market::DEFAULT, H24);
        assert_eq!(b.caption, "OSLO CLOSES IN 30M");
        // 14:31 UTC: Oslo is done, London is next.
        let b = board(at(2026, 9, 18, 14, 31), &Market::DEFAULT, H24);
        assert_eq!(b.caption, "LONDON CLOSES IN 59M");
        // Saturday: the earliest Monday open is 00:00 UTC, shared by Tokyo and
        // Sydney (AEST, no daylight time until October); the first row wins a tie.
        let b = board(at(2026, 9, 19, 12, 0), &Market::DEFAULT, H24);
        assert_eq!(b.caption, "TOKYO OPENS IN 1D 12H");
        let b = board(at(2026, 9, 20, 23, 0), &Market::DEFAULT, H24);
        assert_eq!(b.caption, "TOKYO OPENS IN 1H");
        let b = board(at(2026, 9, 20, 23, 0), &[Market::Sydney, Market::Tokyo], H24);
        assert_eq!(b.caption, "SYDNEY OPENS IN 1H");
    }

    /// The countdown must use the offset in force at the event, not now:
    /// Friday 6 March 2026 16:00 EST to Monday's 09:30 EDT open is 2d 16h 30m,
    /// an hour less than the wall clocks suggest.
    #[test]
    fn the_countdown_crosses_a_daylight_saving_change_correctly() {
        let b = board(at(2026, 3, 6, 21, 0), &[Market::NewYork], H24);
        assert_eq!(b.rows[0].time, "16:00");
        assert_eq!(b.caption, "NEW YORK OPENS IN 2D 16H");
        // Sydney the other way: Friday 3 April 16:00 AEDT (05:00 UTC) to
        // Monday 6 April 10:00 AEST (00:00 UTC) is 2d 19h.
        let b = board(at(2026, 4, 3, 5, 0), &[Market::Sydney], H24);
        assert_eq!(b.rows[0].time, "16:00");
        assert_eq!(b.caption, "SYDNEY OPENS IN 2D 19H");
    }

    /// Sydney's Monday morning is Sunday night in New York: the trading-day
    /// test must use each market's own date.
    #[test]
    fn weekdays_are_judged_in_local_time() {
        let b = board(at(2026, 9, 21, 0, 30), &[Market::Sydney, Market::NewYork], H24);
        assert_eq!((b.rows[0].time.as_str(), b.rows[0].status), ("10:30", Status::Open));
        assert_eq!((b.rows[1].time.as_str(), b.rows[1].status), ("20:30", Status::Closed));
    }

    #[test]
    fn twelve_hour_rows_carry_their_period_and_drop_the_leading_zero() {
        let b = board(at(2026, 9, 18, 14, 5), &[Market::NewYork, Market::Tokyo], style(ClockFormat::H12, false));
        assert_eq!((b.rows[0].time.as_str(), b.rows[0].period), ("10:05", Some("AM")));
        assert_eq!((b.rows[1].time.as_str(), b.rows[1].period), ("11:05", Some("PM")));
        let b = board(at(2026, 9, 18, 13, 5), &[Market::NewYork], style(ClockFormat::H12, true));
        assert_eq!((b.rows[0].time.as_str(), b.rows[0].period), ("9:05:00", Some("AM")));
    }

    #[test]
    fn the_board_ticks_by_the_minute_unless_seconds_are_shown() {
        let t = at(2026, 9, 18, 14, 0).with_second(37).unwrap().with_nanosecond(250_000_000).unwrap();
        assert_eq!(board(t, &Market::DEFAULT, H24).until_change, Duration::from_millis(22_750));
        assert_eq!(board(t, &Market::DEFAULT, style(ClockFormat::H24, true)).until_change, Duration::from_millis(750));
    }

    #[test]
    fn an_empty_board_has_no_caption() {
        let b = board(at(2026, 9, 18, 14, 0), &[], H24);
        assert!(b.rows.is_empty());
        assert_eq!(b.caption, "");
    }

    #[test]
    fn widest_captions_cover_the_longest_shapes_for_every_market() {
        let w = widest_captions(&[Market::NewYork, Market::HongKong], '0');
        assert_eq!(
            w,
            vec![
                "NEW YORK CLOSES IN 00H 00M",
                "NEW YORK OPENS IN 0D 00H",
                "HONG KONG CLOSES IN 00H 00M",
                "HONG KONG OPENS IN 0D 00H",
            ]
        );
    }
}
