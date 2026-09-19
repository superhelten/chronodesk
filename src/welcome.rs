//! What the overlay says the first time it is ever started.
//!
//! A transparent clock with no window frame, no buttons and no taskbar entry
//! gives a newcomer nothing to try. So the very first launch shows one card
//! instead of the clock: where the menu is, what the keys do, and the three
//! looks worth knowing about. It is drawn with the overlay's own colours and
//! type, goes away at the first click, key or menu choice, and never comes
//! back unless asked for (menu → *Quick tips*).
//!
//! The text lives here, away from the painting, so it can be checked against
//! the names the menus actually use.

pub const TITLE: &str = "WELCOME TO CHRONODESK";
pub const DISMISS: &str = "CLICK TO BEGIN";

/// A key or a place, and what it does.
pub const ROWS: [(&str, &str); 8] = [
    ("Right-click", "the overlay or its tray icon for the menu"),
    ("1  2  3  4", "Clock · Stopwatch · Timer · Markets"),
    ("Space  R", "start / pause and reset, or hover for the buttons"),
    ("Scroll", "over an idle timer sets its minutes"),
    ("Drag", "moves it; left-click the tray icon locks it click-through"),
    ("Face", "Appearance > Face: Typeface, Seven-segment, Dot matrix"),
    ("Colours", "Appearance > Colours: Default to Green matrix, Red seven-segment"),
    ("Studio look", "Studio (green / red) + Seconds ring + Dot matrix"),
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::Mode;
    use crate::layout::Font;
    use crate::theme::Palette;

    fn text() -> String {
        ROWS.iter().map(|(key, what)| format!("{key} {what}\n")).collect()
    }

    /// The card is only useful while it uses the words the menus use.
    #[test]
    fn every_mode_and_face_is_named_as_the_menu_names_it() {
        let text = text();
        for mode in Mode::ALL {
            assert!(text.contains(mode.label()), "mode '{}' is missing", mode.label());
        }
        for font in Font::ALL {
            assert!(text.contains(font.label()), "face '{}' is missing", font.label());
        }
        for palette in [Palette::Default, Palette::Green, Palette::Red, Palette::Studio] {
            assert!(text.contains(palette.label()), "palette '{}' is missing", palette.label());
        }
    }

    #[test]
    fn the_mode_keys_are_listed_in_the_order_they_work() {
        let (_, modes) = ROWS.iter().find(|(key, _)| key.starts_with('1')).expect("the number keys");
        let positions: Vec<usize> = Mode::ALL.iter().map(|mode| modes.find(mode.label()).expect("named")).collect();
        assert!(positions.is_sorted(), "1-4 must read in `Mode::ALL` order: {modes}");
    }

    #[test]
    fn rows_stay_short_enough_for_a_card() {
        for (key, what) in ROWS {
            assert!(key.len() <= 12 && what.chars().count() <= 66, "{key}: {what}");
        }
    }
}
