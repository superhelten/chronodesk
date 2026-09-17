//! Visual tokens: every colour, alpha and proportion the renderer uses.
//!
//! Collected here so the drawing code reads as intent ("the halo alpha")
//! instead of magic numbers, and so a future themed config has one place to
//! write into. The values are the ones the current look was measured with.

use eframe::egui::Color32;

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Theme {
    pub color: Colors,
    pub ratio: Ratios,
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
}
