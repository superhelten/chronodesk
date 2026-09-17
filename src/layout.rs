//! Derived state: everything the renderer needs that follows from the config
//! rather than from the clock.
//!
//! None of this is persisted. It is rebuilt at startup and then only when its
//! key changes — the chosen size, or the display scale. The readout itself
//! changes every tick, but its *measurements* do not: digits share one cell
//! width, so a line's width is a sum of cached numbers instead of a fresh text
//! layout per frame.

use std::collections::HashMap;
use std::sync::Arc;

use eframe::egui::{Galley, Vec2, vec2};

use crate::app::Size;
use crate::theme::Theme;

/// Characters a readout can contain; cached up front.
pub const READOUT_CHARS: &str = "0123456789:.";

/// What the cached layout depends on. Anything else (mode, time, colours) can
/// change without a rebuild.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LayoutKey {
    pub size: Size,
    /// `f32::to_bits`, so the key stays comparable and hashable.
    pub pixels_per_point: u32,
}

impl LayoutKey {
    pub fn new(size: Size, pixels_per_point: f32) -> Self {
        Self { size, pixels_per_point: pixels_per_point.to_bits() }
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
}

impl Metrics {
    pub fn new(size: Size, theme: &Theme) -> Self {
        let r = &theme.ratio;
        let font = size.font_size();
        let caption_font = (font * r.caption).max(r.caption_min);
        Self {
            font,
            caption_font,
            pad: vec2(font * r.pad_x, font * r.pad_y),
            caption_height: caption_font * r.caption_height,
            corner: font * r.corner,
            halo_width: (font * r.halo_width).clamp(r.halo_width_min, r.halo_width_max),
            control: caption_font * r.control,
            control_gap: caption_font * r.control * r.control_gap,
        }
    }
}

/// Measured glyphs for the current font size. `galleys` is empty in tests,
/// where only the measurements matter.
#[derive(Debug, Default)]
pub struct Glyphs {
    /// Every digit is drawn in a cell this wide, so the readout never jitters.
    pub digit_width: f32,
    pub height: f32,
    /// Cell widths for the non-digit characters.
    pub widths: HashMap<char, f32>,
    pub galleys: HashMap<char, Arc<Galley>>,
}

pub struct DerivedLayout {
    key: Option<LayoutKey>,
    metrics: Metrics,
    glyphs: Glyphs,
    /// Key of the cached caption galley: its text and font size.
    caption_key: Option<(String, f32)>,
    caption_galley: Option<Arc<Galley>>,
    rebuilds: u32,
}

impl DerivedLayout {
    pub fn new(theme: &Theme) -> Self {
        Self {
            key: None,
            metrics: Metrics::new(Size::default(), theme),
            glyphs: Glyphs::default(),
            caption_key: None,
            caption_galley: None,
            rebuilds: 0,
        }
    }

    pub fn metrics(&self) -> &Metrics {
        &self.metrics
    }

    pub fn glyphs(&self) -> &Glyphs {
        &self.glyphs
    }

    /// Number of times the glyph cache has actually been rebuilt; used by tests
    /// to prove the frame loop is not re-measuring text.
    #[cfg(test)]
    pub fn rebuilds(&self) -> u32 {
        self.rebuilds
    }

    /// Rebuilds the cache when `key` differs from the cached one. `measure` is
    /// only called then, and returns the glyphs for `Metrics::font`.
    pub fn ensure(
        &mut self,
        key: LayoutKey,
        theme: &Theme,
        measure: impl FnOnce(&Metrics) -> Glyphs,
    ) -> bool {
        if self.key == Some(key) {
            return false;
        }
        self.metrics = Metrics::new(key.size, theme);
        self.glyphs = measure(&self.metrics);
        self.key = Some(key);
        // Galleys from the previous font size must not survive.
        self.caption_key = None;
        self.caption_galley = None;
        self.rebuilds += 1;
        true
    }

    /// Width of a readout line, from cached cell widths only.
    pub fn width_of(&self, text: &str) -> f32 {
        text.chars().map(|c| self.cell_width(c)).sum()
    }

    pub fn cell_width(&self, c: char) -> f32 {
        if c.is_ascii_digit() {
            self.glyphs.digit_width
        } else {
            self.glyphs.widths.get(&c).copied().unwrap_or(self.glyphs.digit_width)
        }
    }

    pub fn galley(&self, c: char) -> Option<&Arc<Galley>> {
        self.glyphs.galleys.get(&c)
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
}

#[cfg(test)]
mod tests {
    use super::*;

    fn glyphs() -> Glyphs {
        Glyphs {
            digit_width: 24.0,
            height: 50.0,
            widths: HashMap::from([(':', 10.0), ('.', 8.0)]),
            galleys: HashMap::new(),
        }
    }

    fn key(size: Size) -> LayoutKey {
        LayoutKey::new(size, 1.0)
    }

    #[test]
    fn metrics_follow_the_size() {
        let theme = Theme::default();
        let small = Metrics::new(Size::Small, &theme);
        let large = Metrics::new(Size::Large, &theme);
        assert!(large.font > small.font);
        assert!(large.pad.x > small.pad.x);
        assert!(large.control > small.control);
        // The caption has a floor so it stays readable at the smallest size.
        assert!(small.caption_font >= theme.ratio.caption_min);
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
        assert!(layout.ensure(LayoutKey::new(Size::Large, 1.5), &theme, |_| glyphs()));
        assert_eq!(layout.rebuilds(), 3);
    }

    #[test]
    fn measure_receives_the_updated_metrics() {
        let theme = Theme::default();
        let mut layout = DerivedLayout::new(&theme);
        let mut seen = 0.0;
        layout.ensure(key(Size::Large), &theme, |m| {
            seen = m.font;
            glyphs()
        });
        assert_eq!(seen, Size::Large.font_size());
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
        let theme = Theme::default();
        let mut layout = DerivedLayout::new(&theme);
        layout.ensure(key(Size::Medium), &theme, |_| glyphs());
        assert_eq!(layout.cell_width('?'), 24.0);
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
}
