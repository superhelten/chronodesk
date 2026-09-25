//! Cities for the clock's second time zone.
//!
//! A short list of the places people most often keep an eye on, each with
//! the same offset-plus-rule description the market board uses (see `tz.rs`),
//! so no time zone database is shipped for it either. Cities that share a
//! zone are both listed where people think of them by name: nobody looks for
//! Singapore under "Hong Kong".

use serde::{Deserialize, Serialize};

use crate::app::serde_by_id;
use crate::tz::{Dst, Zone};

const HOUR: i32 = 60;

/// Every city the clock can show beside local time, west to east after UTC.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum City {
    Utc,
    LosAngeles,
    Chicago,
    NewYork,
    SaoPaulo,
    London,
    Paris,
    Oslo,
    Dubai,
    Mumbai,
    Singapore,
    HongKong,
    Tokyo,
    Sydney,
}

impl City {
    pub const ALL: [Self; 14] = [
        Self::Utc,
        Self::LosAngeles,
        Self::Chicago,
        Self::NewYork,
        Self::SaoPaulo,
        Self::London,
        Self::Paris,
        Self::Oslo,
        Self::Dubai,
        Self::Mumbai,
        Self::Singapore,
        Self::HongKong,
        Self::Tokyo,
        Self::Sydney,
    ];

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.id().eq_ignore_ascii_case(id))
    }

    pub fn id(self) -> &'static str {
        match self {
            Self::Utc => "utc",
            Self::LosAngeles => "los-angeles",
            Self::Chicago => "chicago",
            Self::NewYork => "new-york",
            Self::SaoPaulo => "sao-paulo",
            Self::London => "london",
            Self::Paris => "paris",
            Self::Oslo => "oslo",
            Self::Dubai => "dubai",
            Self::Mumbai => "mumbai",
            Self::Singapore => "singapore",
            Self::HongKong => "hong-kong",
            Self::Tokyo => "tokyo",
            Self::Sydney => "sydney",
        }
    }

    /// The name in the menu; the caption shows it upper-case.
    pub fn label(self) -> &'static str {
        match self {
            Self::Utc => "UTC",
            Self::LosAngeles => "Los Angeles",
            Self::Chicago => "Chicago",
            Self::NewYork => "New York",
            Self::SaoPaulo => "São Paulo",
            Self::London => "London",
            Self::Paris => "Paris",
            Self::Oslo => "Oslo",
            Self::Dubai => "Dubai",
            Self::Mumbai => "Mumbai",
            Self::Singapore => "Singapore",
            Self::HongKong => "Hong Kong",
            Self::Tokyo => "Tokyo",
            Self::Sydney => "Sydney",
        }
    }

    pub fn zone(self) -> Zone {
        match self {
            Self::Utc => Zone::new(0, Dst::None),
            Self::LosAngeles => Zone::new(-8 * HOUR, Dst::Us),
            Self::Chicago => Zone::new(-6 * HOUR, Dst::Us),
            Self::NewYork => Zone::new(-5 * HOUR, Dst::Us),
            // Brazil dropped daylight saving time in 2019.
            Self::SaoPaulo => Zone::new(-3 * HOUR, Dst::None),
            Self::London => Zone::new(0, Dst::Eu),
            Self::Paris | Self::Oslo => Zone::new(HOUR, Dst::Eu),
            Self::Dubai => Zone::new(4 * HOUR, Dst::None),
            Self::Mumbai => Zone::new(5 * HOUR + 30, Dst::None),
            Self::Singapore | Self::HongKong => Zone::new(8 * HOUR, Dst::None),
            Self::Tokyo => Zone::new(9 * HOUR, Dst::None),
            Self::Sydney => Zone::new(10 * HOUR, Dst::Au),
        }
    }
}
serde_by_id!(City, "city");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::market::Market;

    #[test]
    fn city_ids_round_trip() {
        for city in City::ALL {
            assert_eq!(City::from_id(city.id()), Some(city));
        }
        assert_eq!(City::from_id("atlantis"), None);
    }

    /// A city that also has an exchange on the board keeps the board's id and
    /// the board's zone, so the two can never disagree about its time.
    #[test]
    fn cities_on_the_board_agree_with_it() {
        for market in Market::ALL {
            if let Some(city) = City::from_id(market.id()) {
                assert_eq!(city.zone(), market.zone(), "{}", market.id());
            }
        }
    }

    /// Whole minutes, or the clock's minute tick would not be the other
    /// zone's minute tick as well.
    #[test]
    fn every_offset_is_a_whole_half_hour() {
        for city in City::ALL {
            assert_eq!(city.zone().standard % 30, 0, "{}", city.id());
        }
    }
}
