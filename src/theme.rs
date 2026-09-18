//! Visual tokens: every colour, alpha and proportion the renderer uses.
//!
//! Collected here so the drawing code reads as intent ("the halo alpha")
//! instead of magic numbers, and so a future themed config has one place to
//! write into. The values are the ones the current look was measured with.

use eframe::egui::Color32;
use serde::{Deserialize, Serialize};

use crate::app::serde_by_id;

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Theme {
    pub color: Colors,
    pub ratio: Ratios,
    pub segments: Segments,
}

/// Proportions of the seven-segment face, all relative to the main font
/// size so it steps with the sizes exactly as the typeface does.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Segments {
    /// Height of the whole cell; the digit is centred in it.
    pub line: f32,
    pub height: f32,
    pub width: f32,
    pub thickness: f32,
    /// Space left between the pointed ends of neighbouring segments.
    pub gap: f32,
    /// Horizontal shift per unit of height: the italic lean of the digits.
    pub slant: f32,
    /// Horizontal space between neighbouring cells.
    pub spacing: f32,
}

impl Default for Segments {
    fn default() -> Self {
        Self { line: 1.0, height: 0.72, width: 0.40, thickness: 0.085, gap: 0.025, slant: 0.09, spacing: 0.16 }
    }
}

/// Fixed colour presets. Only the readout colours differ; halo, backdrop and
/// controls are tuned for contrast and stay the same in every preset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Palette {
    #[default]
    Default,
    Warm,
    Cool,
    Amber,
}

impl Palette {
    pub const ALL: [Self; 4] = [Self::Default, Self::Warm, Self::Cool, Self::Amber];

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.id().eq_ignore_ascii_case(id))
    }

    pub fn id(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Warm => "warm",
            Self::Cool => "cool",
            Self::Amber => "amber",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Default => "Default",
            Self::Warm => "Warm",
            Self::Cool => "Cool",
            Self::Amber => "Amber",
        }
    }
}
serde_by_id!(Palette, "palette");

impl Theme {
    /// The theme to draw with: a preset, optionally dimmed for night mode.
    /// `night_dim` is the factor applied to the readout colours (1.0 = none).
    pub fn resolve(palette: Palette, night_dim: Option<f32>) -> Self {
        let mut theme = Self::default();
        let c = &mut theme.color;
        (c.text, c.alert) = match palette {
            Palette::Default => (c.text, c.alert),
            Palette::Warm => (Color32::from_rgb(255, 221, 170), Color32::from_rgb(245, 120, 60)),
            Palette::Cool => (Color32::from_rgb(200, 228, 255), Color32::from_rgb(255, 170, 60)),
            Palette::Amber => (Color32::from_rgb(245, 180, 60), Color32::from_rgb(240, 80, 70)),
        };
        if let Some(dim) = night_dim.filter(|d| *d < 1.0) {
            c.text = c.text.gamma_multiply(dim);
            c.alert = c.alert.gamma_multiply(dim);
        }
        theme
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Colors {
    pub text: Color32,
    /// Applied to the caption when a backdrop makes full strength unnecessary.
    pub caption_dim: f32,
    /// Timer that has run out.
    pub alert: Color32,
    /// Per-copy alpha of the two halo rings. They overlap, hence the low values:
    /// measured over pure white the halo lands near #464646, ~9:1 to the glyph.
    pub halo: [u8; 2],
    /// The seven-segment face is haloed by a single stroke around each
    /// segment instead of stacked copies, so it carries its own alpha.
    pub halo_stroke: u8,
    pub backdrop: Color32,
    /// Must stay exactly #00FF00 for keying to work.
    pub chroma: Color32,
    /// Faint frame shown while the pointer is over an overlay without backdrop.
    pub hover_frame: Color32,
    /// Dark disc behind the hover controls, for contrast over busy windows.
    pub control_base: Color32,
    pub control_hover: Color32,
    pub control_glyph: u8,
    pub control_glyph_hover: u8,
}

/// Proportions of the main font size, so every size steps together.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ratios {
    pub caption: f32,
    pub caption_min: f32,
    pub caption_height: f32,
    pub pad_x: f32,
    pub pad_y: f32,
    pub corner: f32,
    pub halo_width: f32,
    pub halo_width_min: f32,
    pub halo_width_max: f32,
    /// Caption-relative, not font-relative.
    pub control: f32,
    pub control_gap: f32,
    pub control_disc: f32,
    pub control_inset: f32,
}

impl Default for Colors {
    fn default() -> Self {
        Self {
            text: Color32::from_rgb(242, 242, 240),
            caption_dim: 0.62,
            alert: Color32::from_rgb(245, 165, 36),
            halo: [34, 22],
            halo_stroke: 160,
            backdrop: Color32::from_rgba_premultiplied(8, 8, 10, 178),
            chroma: Color32::from_rgb(0, 255, 0),
            hover_frame: Color32::from_white_alpha(46),
            control_base: Color32::from_black_alpha(190),
            control_hover: Color32::from_white_alpha(36),
            control_glyph: 235,
            control_glyph_hover: 255,
        }
    }
}

impl Default for Ratios {
    fn default() -> Self {
        Self {
            caption: 0.26,
            caption_min: 10.0,
            caption_height: 1.5,
            pad_x: 0.36,
            pad_y: 0.16,
            corner: 0.22,
            halo_width: 0.028,
            halo_width_min: 1.0,
            halo_width_max: 1.7,
            control: 1.25,
            control_gap: 0.9,
            control_disc: 0.78,
            control_inset: 0.22,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chroma_is_exactly_the_key_colour() {
        // Anything else and a streaming key leaves a fringe.
        assert_eq!(Colors::default().chroma, Color32::from_rgb(0, 255, 0));
    }

    #[test]
    fn halo_rings_fade_outwards() {
        let halo = Colors::default().halo;
        assert!(halo[0] > halo[1], "the outer ring must be the fainter one");
    }

    #[test]
    fn the_default_palette_is_the_measured_look() {
        assert_eq!(Theme::resolve(Palette::Default, None), Theme::default());
        assert_eq!(Theme::resolve(Palette::Default, Some(1.0)), Theme::default());
    }

    #[test]
    fn every_palette_keeps_the_contrast_and_keying_tokens() {
        let base = Theme::default();
        for palette in Palette::ALL {
            let t = Theme::resolve(palette, None);
            assert_eq!(t.color.chroma, base.color.chroma, "{palette:?} must not touch the key colour");
            assert_eq!(t.color.halo, base.color.halo, "{palette:?}");
            assert_eq!(t.color.backdrop, base.color.backdrop, "{palette:?}");
            assert_eq!(t.color.control_base, base.color.control_base, "{palette:?}");
            assert_eq!(t.ratio, base.ratio, "{palette:?} must not change the layout");
            assert_eq!(t.segments, base.segments, "{palette:?} must not change the digital face");
        }
    }

    #[test]
    fn segments_fit_their_cell() {
        let s = Segments::default();
        assert!(s.height <= s.line, "the digit must fit the line box");
        assert!(s.thickness * 2.0 < s.width, "two verticals must leave room between them");
        assert!(s.thickness * 3.0 < s.height, "three horizontals must leave room between them");
        assert!(s.gap < s.thickness);
    }

    #[test]
    fn palettes_differ_in_text_colour() {
        let texts: Vec<_> = Palette::ALL.iter().map(|&p| Theme::resolve(p, None).color.text).collect();
        for (i, a) in texts.iter().enumerate() {
            for b in &texts[i + 1..] {
                assert_ne!(a, b, "two presets share a text colour");
            }
        }
    }

    #[test]
    fn night_dim_fades_the_readout_and_nothing_else() {
        let base = Theme::resolve(Palette::Default, None);
        let dim = Theme::resolve(Palette::Default, Some(0.7));
        assert!(dim.color.text.a() < base.color.text.a());
        assert!(dim.color.alert.a() < base.color.alert.a());
        assert_eq!(dim.color.chroma, base.color.chroma);
        assert_eq!(dim.color.backdrop, base.color.backdrop);
        assert_eq!(dim.color.halo, base.color.halo);
        assert_eq!(dim.color.control_glyph, base.color.control_glyph);
    }

    #[test]
    fn palette_ids_round_trip() {
        for palette in Palette::ALL {
            assert_eq!(Palette::from_id(palette.id()), Some(palette));
        }
        assert_eq!(Palette::from_id("neon"), None);
    }
}
