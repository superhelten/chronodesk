//! The Pomodoro cycle the timer can run: four focus periods, a short break
//! after each of the first three and a long one after the fourth.
//!
//! A focus period lasts whatever the timer is set to, 25 minutes unless the
//! user picked another duration. A short break is a fifth of that, the five
//! minutes of the classic technique at 25, and a long break three short ones.
//! Nothing starts by itself: a finished period chimes like any timer, and
//! the next one begins when the user presses Start, so a cycle left alone
//! waits instead of running on through an empty room.
//!
//! The position in the cycle is a plain index, `0..PHASES`, which is all the
//! config file needs to keep across a restart.

use std::time::Duration;

/// Focus, break, focus, break, focus, break, focus, long break.
pub const PHASES: u8 = 8;
const FOCUS_PERIODS: u8 = PHASES / 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// The `n`th focus period of the cycle, from 1.
    Focus(u8),
    Break,
    LongBreak,
}

impl Phase {
    /// What the phase is called in the caption.
    pub fn name(self) -> String {
        match self {
            Self::Focus(n) => format!("FOCUS {n}/{FOCUS_PERIODS}"),
            Self::Break => "BREAK".to_owned(),
            Self::LongBreak => "LONG BREAK".to_owned(),
        }
    }

    /// What the caption says once the phase has run out.
    pub fn over(self) -> &'static str {
        match self {
            Self::Focus(_) => "TIME FOR A BREAK",
            Self::Break | Self::LongBreak => "BACK TO FOCUS",
        }
    }

    /// The Start item's label for the phase after this one, once this has
    /// run out.
    pub fn start_next(self) -> &'static str {
        match self {
            Self::Focus(_) => "Start break",
            Self::Break | Self::LongBreak => "Start focus",
        }
    }
}

/// The phase at `index`; anything out of range counts from the top.
pub fn phase(index: u8) -> Phase {
    let index = index % PHASES;
    match index {
        _ if index.is_multiple_of(2) => Phase::Focus(index / 2 + 1),
        _ if index == PHASES - 1 => Phase::LongBreak,
        _ => Phase::Break,
    }
}

pub fn next(index: u8) -> u8 {
    (index % PHASES + 1) % PHASES
}

/// How long the phase at `index` lasts, for focus periods of `focus_minutes`.
pub fn duration(index: u8, focus_minutes: u64) -> Duration {
    let short = (focus_minutes as f64 / 5.0).round().max(1.0) as u64;
    let minutes = match phase(index) {
        Phase::Focus(_) => focus_minutes,
        Phase::Break => short,
        Phase::LongBreak => short * 3,
    };
    Duration::from_secs(minutes * 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minutes(m: u64) -> Duration {
        Duration::from_secs(m * 60)
    }

    #[test]
    fn a_cycle_is_four_focus_periods_with_a_long_break_at_the_end() {
        let cycle: Vec<Phase> = (0..PHASES).map(phase).collect();
        assert_eq!(
            cycle,
            [
                Phase::Focus(1),
                Phase::Break,
                Phase::Focus(2),
                Phase::Break,
                Phase::Focus(3),
                Phase::Break,
                Phase::Focus(4),
                Phase::LongBreak
            ]
        );
    }

    #[test]
    fn after_the_long_break_the_cycle_starts_over() {
        let mut index = 0;
        for _ in 0..PHASES {
            index = next(index);
        }
        assert_eq!(index, 0);
        assert_eq!(next(PHASES - 1), 0);
    }

    /// A hand-edited config can hold any number; it is read as a position in
    /// the cycle rather than rejected.
    #[test]
    fn an_index_out_of_range_counts_from_the_top() {
        assert_eq!(phase(PHASES), Phase::Focus(1));
        assert_eq!(phase(255), phase(255 % PHASES));
        assert!(next(200) < PHASES);
    }

    #[test]
    fn at_twenty_five_minutes_the_breaks_are_the_classic_five_and_fifteen() {
        assert_eq!(duration(0, 25), minutes(25));
        assert_eq!(duration(1, 25), minutes(5));
        assert_eq!(duration(PHASES - 1, 25), minutes(15));
    }

    #[test]
    fn breaks_scale_with_the_focus_period_and_never_vanish() {
        assert_eq!(duration(1, 50), minutes(10));
        assert_eq!(duration(PHASES - 1, 50), minutes(30));
        assert_eq!(duration(1, 1), minutes(1), "a one-minute focus still gets a break");
        assert_eq!(duration(PHASES - 1, 1), minutes(3));
    }

    #[test]
    fn captions_name_the_phase_and_what_comes_next() {
        assert_eq!(phase(2).name(), "FOCUS 2/4");
        assert_eq!(phase(3).name(), "BREAK");
        assert_eq!(phase(7).name(), "LONG BREAK");
        assert_eq!(phase(0).over(), "TIME FOR A BREAK");
        assert_eq!(phase(7).over(), "BACK TO FOCUS");
        assert_eq!(phase(0).start_next(), "Start break");
        assert_eq!(phase(1).start_next(), "Start focus");
    }
}
