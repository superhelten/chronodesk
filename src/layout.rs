//! Derived state: everything the renderer needs that follows from the config
//! rather than from the clock.
//!
//! None of this is persisted. It is rebuilt at startup and then only when its
//! key changes — the chosen size, face, or the display scale. The readout
//! itself changes every tick, but its *measurements* do not: digits share one
//! cell width, so a line's width is a sum of cached numbers instead of a fresh
//! text layout per frame.

use std::collections::HashMap;
use std::sync::Arc;

use eframe::egui::{Galley, Pos2, Vec2, vec2};
use serde::{Deserialize, Serialize};

use crate::app::{Size, serde_by_id};
use crate::theme::Theme;

/// Characters a readout can contain; cached up front.
pub const READOUT_CHARS: &str = "0123456789:.";

/// How the readout is drawn: a typeface, or seven-segment digits built from
/// polygons (see `digital.rs`), which need no font at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Font {
    #[default]
    Sans,
    Digital,
}

impl Font {
    pub const ALL: [Self; 2] = [Self::Sans, Self::Digital];

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|f| f.id().eq_ignore_ascii_case(id))
    }

    pub fn id(self) -> &'static str {
        match self {
            Self::Sans => "sans",
            Self::Digital => "digital",
        }
    }

    pub fn toggled(self) -> Self {
        match self {
            Self::Sans => Self::Digital,
            Self::Digital => Self::Sans,
        }
    }
}
serde_by_id!(Font, "font");

/// What the cached layout depends on. Anything else (mode, time, colours) can
/// change without a rebuild.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LayoutKey {
    pub size: Size,
    /// `f32::to_bits`, so the key stays comparable and hashable.
    pub pixels_per_point: u32,
    pub font: Font,
}

impl LayoutKey {
    pub fn new(size: Size, pixels_per_point: f32, font: Font) -> Self {
        Self { size, pixels_per_point: pixels_per_point.to_bits(), font }
    }
}

/// Sizes and spacings, all derived from the main font size and the theme.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Metrics {
    pub font: f32,
    pub caption_font: f32,
    pub pad: Vec2,
    pub caption_height: f32,
    pub corner: f32,
    pub halo_width: f32,
    pub control: f32,
    pub control_gap: f32,
    /// Market board: the time on a row, the row's pitch, the gap between
    /// columns, and the status dot.
    pub row_font: f32,
    pub row_pitch: f32,
    pub board_gap: f32,
    pub dot_radius: f32,
    pub ring_width: f32,
    /// Seconds ring: LED radius, and the band it takes around the content.
    pub led_radius: f32,
    pub ring_band: f32,
}

impl Metrics {
    pub fn new(size: Size, theme: &Theme) -> Self {
        let r = &theme.ratio;
        let font = size.font_size();
        let caption_font = (font * r.caption).max(r.caption_min);
        let row_font = (font * r.board_time).max(r.board_time_min);
        Self {
            font,
            caption_font,
            pad: vec2(font * r.pad_x, font * r.pad_y),
            caption_height: caption_font * r.caption_height,
            corner: font * r.corner,
            halo_width: (font * r.halo_width).clamp(r.halo_width_min, r.halo_width_max),
            control: caption_font * r.control,
            control_gap: caption_font * r.control * r.control_gap,
            row_font,
            row_pitch: row_font * r.board_pitch,
            board_gap: caption_font * r.board_gap,
            dot_radius: caption_font * r.board_dot,
            ring_width: (caption_font * r.board_ring).max(1.0),
            led_radius: (caption_font * r.ring_dot).max(1.0),
            ring_band: (caption_font * r.ring_dot).max(1.0) * r.ring_band,
        }
    }
}

/// One readout character, ready to paint at a cell's top-left corner.
#[derive(Debug, Clone, PartialEq)]
pub enum Glyph {
    Text(Arc<Galley>),
    /// Convex polygons in cell coordinates.
    Digital(Vec<Vec<Pos2>>),
}

/// Measured glyphs for one font size and face. `glyphs` is empty in tests,
/// where only the measurements matter.
#[derive(Debug)]
pub struct Glyphs {
    /// Every digit is drawn in a cell this wide, so the readout never jitters.
    pub digit_width: f32,
    pub height: f32,
    /// Cell widths for the non-digit characters.
    pub widths: HashMap<char, f32>,
    pub glyphs: HashMap<char, Glyph>,
}

impl Default for Glyphs {
    fn default() -> Self {
        Self { digit_width: 0.0, height: 0.0, widths: HashMap::new(), glyphs: HashMap::new() }
    }
}

impl Glyphs {
    /// Width of a readout line, from cached cell widths only.
    pub fn width_of(&self, text: &str) -> f32 {
        text.chars().map(|c| self.cell_width(c)).sum()
    }

    pub fn cell_width(&self, c: char) -> f32 {
        if c.is_ascii_digit() {
            self.digit_width
        } else {
            self.widths.get(&c).copied().unwrap_or(self.digit_width)
        }
    }

    pub fn glyph(&self, c: char) -> Option<&Glyph> {
        self.glyphs.get(&c)
    }
}

pub struct DerivedLayout {
    key: Option<LayoutKey>,
    metrics: Metrics,
    /// The main readout line, at `Metrics::font`.
    glyphs: Glyphs,
    /// The market board's rows, at `Metrics::row_font`. Measured on the same
    /// rebuild so switching modes never re-measures anything.
    row_glyphs: Glyphs,
    /// Key of the cached caption galley: its text and font size. One slot,
    /// because the caption changes every minute at most and is replaced,
    /// never revisited.
    caption_key: Option<(String, f32)>,
    caption_galley: Option<Arc<Galley>>,
    /// Labels that are fixed for a market set (city names, AM/PM, the widest
    /// captions), all at the caption size. Cleared on rebuild, never evicted
    /// otherwise: there are a few dozen at most.
    labels: HashMap<String, Arc<Galley>>,
    rebuilds: u32,
}

impl DerivedLayout {
    pub fn new(theme: &Theme) -> Self {
        Self {
            key: None,
            metrics: Metrics::new(Size::default(), theme),
            glyphs: Glyphs::default(),
            row_glyphs: Glyphs::default(),
            caption_key: None,
            caption_galley: None,
            labels: HashMap::new(),
            rebuilds: 0,
        }
    }

    pub fn metrics(&self) -> &Metrics {
        &self.metrics
    }

    pub fn glyphs(&self) -> &Glyphs {
        &self.glyphs
    }

    pub fn row_glyphs(&self) -> &Glyphs {
        &self.row_glyphs
    }

    /// Number of times the glyph cache has actually been rebuilt; used by the
    /// tests and the instrumentation to prove the frame loop is not
    /// re-measuring text.
    pub fn rebuilds(&self) -> u32 {
        self.rebuilds
    }

    /// Rebuilds the cache when `key` differs from the cached one. `measure` is
    /// only called then, once per font size the layout needs.
    pub fn ensure(&mut self, key: LayoutKey, theme: &Theme, mut measure: impl FnMut(f32) -> Glyphs) -> bool {
        if self.key == Some(key) {
            return false;
        }
        self.metrics = Metrics::new(key.size, theme);
        self.glyphs = measure(self.metrics.font);
        self.row_glyphs = measure(self.metrics.row_font);
        self.key = Some(key);
        // Galleys from the previous font size must not survive.
        self.caption_key = None;
        self.caption_galley = None;
        self.labels.clear();
        self.rebuilds += 1;
        true
    }

    /// Width of a main readout line, from cached cell widths only.
    pub fn width_of(&self, text: &str) -> f32 {
        self.glyphs.width_of(text)
    }

    fn caption_is_stale(&self, text: &str, font: f32) -> bool {
        self.caption_key.as_ref().is_none_or(|(t, f)| t != text || *f != font)
    }

    /// The caption changes far more rarely than the readout, so its galley is
    /// laid out once per distinct string instead of once per frame.
    pub fn caption_galley(&mut self, text: &str, font: f32, layout: impl FnOnce() -> Arc<Galley>) -> Arc<Galley> {
        if self.caption_is_stale(text, font) || self.caption_galley.is_none() {
            self.caption_key = Some((text.to_owned(), font));
            self.caption_galley = Some(layout());
        }
        self.caption_galley.clone().expect("just populated")
    }

    /// A fixed label at the caption size, laid out on first use and kept
    /// until the next rebuild.
    pub fn label_galley(&mut self, text: &str, layout: impl FnOnce(&str) -> Arc<Galley>) -> Arc<Galley> {
        if let Some(galley) = self.labels.get(text) {
            return galley.clone();
        }
        let galley = layout(text);
        self.labels.insert(text.to_owned(), galley.clone());
        galley
    }

    #[cfg(test)]
    fn label_count(&self) -> usize {
        self.labels.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::pos2;

    fn glyphs() -> Glyphs {
        Glyphs {
            digit_width: 24.0,
            height: 50.0,
            widths: HashMap::from([(':', 10.0), ('.', 8.0)]),
            glyphs: HashMap::new(),
        }
    }

    /// Glyphs whose measurements record the font size they were asked for.
    fn sized(font: f32) -> Glyphs {
        Glyphs { digit_width: font, height: font, ..glyphs() }
    }

    fn key(size: Size) -> LayoutKey {
        LayoutKey::new(size, 1.0, Font::Sans)
    }

    /// A galley needs a live font atlas, which a headless context provides
    /// inside a pass; no window or GPU is involved.
    fn with_fonts(f: impl FnOnce(&dyn Fn(&str) -> Arc<Galley>)) {
        let ctx = eframe::egui::Context::default();
        let mut f = Some(f);
        let mut out = ctx.run_ui(Default::default(), |ui| {
            let ctx = ui.ctx();
            let lay = |text: &str| {
                ctx.fonts_mut(|fonts| {
                    fonts.layout_no_wrap(text.to_owned(), eframe::egui::FontId::proportional(11.0), eframe::egui::Color32::WHITE)
                })
            };
            if let Some(f) = f.take() {
                f(&lay);
            }
        });
        // Nothing consumes the font atlas here.
        out.textures_delta.clear();
    }

    #[test]
    fn metrics_follow_the_size() {
        let theme = Theme::default();
        let small = Metrics::new(Size::Small, &theme);
        let large = Metrics::new(Size::Large, &theme);
        assert!(large.font > small.font);
        assert!(large.pad.x > small.pad.x);
        assert!(large.control > small.control);
        assert!(large.row_font > small.row_font);
        assert!(large.row_pitch > large.row_font, "rows need room above and below the digits");
        // The caption has a floor so it stays readable at the smallest size.
        assert!(small.caption_font >= theme.ratio.caption_min);
    }

    /// The board's digits are half the main size, but at the smallest size
    /// that would drop under the legibility floor, so they stop there.
    #[test]
    fn row_font_has_a_floor_and_stays_below_the_main_font() {
        let theme = Theme::default();
        for size in Size::ALL {
            let m = Metrics::new(size, &theme);
            assert!(m.row_font >= theme.ratio.board_time_min, "{size:?}");
            assert!(m.row_font < m.font, "{size:?}");
            assert!(m.row_font > m.caption_font, "{size:?}: the time must outrank the city label");
        }
        assert_eq!(Metrics::new(Size::Small, &theme).row_font, theme.ratio.board_time_min);
    }

    #[test]
    fn the_ring_band_clears_its_dots_at_every_size() {
        let theme = Theme::default();
        for size in Size::ALL {
            let m = Metrics::new(size, &theme);
            assert!(m.led_radius >= 1.0, "{size:?}");
            assert!(m.ring_band >= m.led_radius * 3.0, "{size:?}: dots need room on both sides");
            assert!(m.ring_band < m.font, "{size:?}: the band must stay a rim, not a frame");
        }
    }

    #[test]
    fn halo_width_is_clamped_at_both_ends() {
        let theme = Theme::default();
        for size in Size::ALL {
            let m = Metrics::new(size, &theme);
            assert!(m.halo_width >= theme.ratio.halo_width_min);
            assert!(m.halo_width <= theme.ratio.halo_width_max);
        }
    }

    #[test]
    fn cache_rebuilds_only_when_the_key_changes() {
        let theme = Theme::default();
        let mut layout = DerivedLayout::new(&theme);

        assert!(layout.ensure(key(Size::Medium), &theme, |_| glyphs()));
        assert_eq!(layout.rebuilds(), 1);

        // A frame that changes nothing must not re-measure.
        for _ in 0..100 {
            assert!(!layout.ensure(key(Size::Medium), &theme, |_| panic!("re-measured")));
        }
        assert_eq!(layout.rebuilds(), 1);

        // Config change: new size.
        assert!(layout.ensure(key(Size::Large), &theme, |_| glyphs()));
        assert_eq!(layout.rebuilds(), 2);
        assert_eq!(layout.metrics().font, Size::Large.font_size());

        // Display scale change at the same size.
        assert!(layout.ensure(LayoutKey::new(Size::Large, 1.5, Font::Sans), &theme, |_| glyphs()));
        assert_eq!(layout.rebuilds(), 3);
    }

    /// The face decides how every cell is measured, so it is part of the key:
    /// switching it rebuilds once, and switching back rebuilds once more.
    #[test]
    fn changing_the_font_face_rebuilds_the_cache() {
        let theme = Theme::default();
        let mut layout = DerivedLayout::new(&theme);
        layout.ensure(LayoutKey::new(Size::Medium, 1.0, Font::Sans), &theme, |_| glyphs());
        assert!(layout.ensure(LayoutKey::new(Size::Medium, 1.0, Font::Digital), &theme, |_| glyphs()));
        assert_eq!(layout.rebuilds(), 2);
        assert!(!layout.ensure(LayoutKey::new(Size::Medium, 1.0, Font::Digital), &theme, |_| panic!("re-measured")));
        assert!(layout.ensure(LayoutKey::new(Size::Medium, 1.0, Font::Sans), &theme, |_| glyphs()));
        assert_eq!(layout.rebuilds(), 3);
    }

    #[test]
    fn font_ids_round_trip_and_toggle() {
        for font in Font::ALL {
            assert_eq!(Font::from_id(font.id()), Some(font));
            assert_ne!(font.toggled(), font);
            assert_eq!(font.toggled().toggled(), font);
        }
        assert_eq!(Font::from_id("serif"), None);
    }

    /// A digital glyph is a list of convex polygons at the cell origin; the
    /// cache stores it like a galley and hands it back by character.
    #[test]
    fn digital_glyphs_are_stored_and_looked_up_by_character() {
        let theme = Theme::default();
        let mut layout = DerivedLayout::new(&theme);
        let square = vec![pos2(0.0, 0.0), pos2(1.0, 0.0), pos2(1.0, 1.0), pos2(0.0, 1.0)];
        layout.ensure(LayoutKey::new(Size::Medium, 1.0, Font::Digital), &theme, |_| Glyphs {
            digit_width: 10.0,
            height: 20.0,
            glyphs: HashMap::from([('8', Glyph::Digital(vec![square.clone()]))]),
            ..Glyphs::default()
        });
        match layout.glyphs().glyph('8') {
            Some(Glyph::Digital(polygons)) => assert_eq!(polygons, &vec![square]),
            other => panic!("expected a digital glyph, got {other:?}"),
        }
        assert!(layout.glyphs().glyph('9').is_none());
    }

    /// One rebuild measures both faces the renderer can need: the main line
    /// at the main size and the board rows at the row size. Mode is not in
    /// the key, so a mode switch costs nothing.
    #[test]
    fn a_rebuild_measures_the_main_line_and_the_board_rows() {
        let theme = Theme::default();
        let mut layout = DerivedLayout::new(&theme);
        let mut asked = Vec::new();
        layout.ensure(key(Size::Large), &theme, |font| {
            asked.push(font);
            sized(font)
        });
        let m = *layout.metrics();
        assert_eq!(asked, vec![m.font, m.row_font]);
        assert_eq!(layout.glyphs().digit_width, m.font);
        assert_eq!(layout.row_glyphs().digit_width, m.row_font);
        assert_eq!(m.font, Size::Large.font_size());
    }

    #[test]
    fn line_width_is_stable_across_digit_changes() {
        let theme = Theme::default();
        let mut layout = DerivedLayout::new(&theme);
        layout.ensure(key(Size::Medium), &theme, |_| glyphs());

        let width = layout.width_of("00:00:00");
        assert_eq!(width, 6.0 * 24.0 + 2.0 * 10.0);
        // Every digit shares a cell, so the readout never shifts as it ticks.
        assert_eq!(layout.width_of("18:49:37"), width);
        // A different shape does change the width.
        assert_eq!(layout.width_of("00:00.0"), 5.0 * 24.0 + 10.0 + 8.0);
    }

    #[test]
    fn unknown_characters_fall_back_to_a_digit_cell() {
        assert_eq!(glyphs().cell_width('?'), 24.0);
    }

    #[test]
    fn caption_is_relaid_out_only_when_its_text_or_size_changes() {
        let theme = Theme::default();
        let mut layout = DerivedLayout::new(&theme);
        layout.ensure(key(Size::Medium), &theme, |_| glyphs());

        assert!(layout.caption_is_stale("STOPWATCH", 11.0));
        // Simulate the cache being filled, since a Galley needs a live font atlas.
        layout.caption_key = Some(("STOPWATCH".to_owned(), 11.0));
        assert!(!layout.caption_is_stale("STOPWATCH", 11.0), "same caption must reuse the galley");
        assert!(layout.caption_is_stale("PAUSED", 11.0), "new text needs a new galley");
        assert!(layout.caption_is_stale("STOPWATCH", 17.0), "new size needs a new galley");
    }

    #[test]
    fn a_rebuild_drops_the_caption_cache() {
        let theme = Theme::default();
        let mut layout = DerivedLayout::new(&theme);
        layout.ensure(key(Size::Medium), &theme, |_| glyphs());
        layout.caption_key = Some(("TIMER".to_owned(), 11.0));
        layout.ensure(key(Size::Large), &theme, |_| glyphs());
        assert!(layout.caption_is_stale("TIMER", 11.0), "galleys from the old font must not survive");
    }

    /// City labels are laid out once per rebuild, however many frames ask
    /// for them, and a changing caption never evicts them.
    #[test]
    fn labels_are_laid_out_once_and_survive_caption_changes() {
        let theme = Theme::default();
        let mut layout = DerivedLayout::new(&theme);
        layout.ensure(key(Size::Medium), &theme, |_| glyphs());

        let mut layouts = 0;
        with_fonts(|lay| {
            for frame in 0..100 {
                for city in ["NEW YORK", "LONDON", "TOKYO"] {
                    layout.label_galley(city, |text| {
                        layouts += 1;
                        lay(text)
                    });
                }
                layout.caption_galley(&format!("LONDON CLOSES IN {frame}M"), 11.0, || lay("caption"));
            }
        });
        assert_eq!(layouts, 3);
        assert_eq!(layout.label_count(), 3);

        layout.ensure(key(Size::Large), &theme, |_| glyphs());
        assert_eq!(layout.label_count(), 0, "labels from the old font must not survive");
    }
}
