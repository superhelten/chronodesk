//! What the overlay shows and how big: the two choices everything else
//! is laid out around.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    #[default]
    Clock,
    Stopwatch,
    Timer,
    /// The market board: one row per exchange, in its local time.
    Market,
}

impl Mode {
    pub const ALL: [Self; 4] = [Self::Clock, Self::Stopwatch, Self::Timer, Self::Market];

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.id().eq_ignore_ascii_case(id))
    }

    pub fn id(self) -> &'static str {
        match self {
            Self::Clock => "clock",
            Self::Stopwatch => "stopwatch",
            Self::Timer => "timer",
            Self::Market => "market",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Clock => "Clock",
            Self::Stopwatch => "Stopwatch",
            Self::Timer => "Timer",
            Self::Market => "Markets",
        }
    }

    /// Whether start/pause and reset mean anything: only the two modes
    /// that run something have hover controls and menu actions.
    pub fn has_controls(self) -> bool {
        matches!(self, Self::Stopwatch | Self::Timer)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Size {
    Small,
    #[default]
    Medium,
    Large,
}

impl Size {
    pub const ALL: [Self; 3] = [Self::Small, Self::Medium, Self::Large];

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.id().eq_ignore_ascii_case(id))
    }

    pub fn id(self) -> &'static str {
        match self {
            Self::Small => "small",
            Self::Medium => "medium",
            Self::Large => "large",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Small => "Small",
            Self::Medium => "Medium",
            Self::Large => "Large",
        }
    }

    pub fn font_size(self) -> f32 {
        match self {
            Self::Small => 28.0,
            Self::Medium => 44.0,
            Self::Large => 68.0,
        }
    }
}

// Stored as their lowercase ids rather than variant names: RON drops the name
// of a unit variant when a value is read untyped, which is exactly what the
// field-by-field config loader does.
macro_rules! serde_by_id {
    ($ty:ty, $what:literal) => {
        impl Serialize for $ty {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_str(self.id())
            }
        }

        impl<'de> Deserialize<'de> for $ty {
            fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let id = String::deserialize(deserializer)?;
                Self::from_id(&id)
                    .ok_or_else(|| serde::de::Error::custom(format!("unknown {} '{id}'", $what)))
            }
        }
    };
}

pub(crate) use serde_by_id;
serde_by_id!(Mode, "mode");
serde_by_id!(Size, "size");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mode_ids_round_trip() {
        for mode in Mode::ALL {
            assert_eq!(Mode::from_id(mode.id()), Some(mode));
        }
        assert_eq!(Mode::from_id("market"), Some(Mode::Market));
    }
}
