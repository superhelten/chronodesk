//! Visual tokens: every colour, alpha and proportion the renderer uses.
//!
//! Collected here so the drawing code reads as intent ("the halo alpha")
//! instead of magic numbers, and so a future themed config has one place to
//! write into. The values are the ones the current look was measured with.

use eframe::egui::Color32;
use serde::{Deserialize, Serialize};

use crate::app::serde_by_id;

/// Readout text is never faded below this alpha in normal use (requirement
/// T6): under it the halo no longer buys enough contrast over a bright
/// background. Night dimming and caption dimming both stop here, including
/// when they stack.
pub const TEXT_ALPHA_FLOOR: f32 = 0.6;

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Theme {
    pub color: Colors,
    pub ratio: Ratios,
    pub segments: Segments,
    pub matrix: Matrix,
}

/// Proportions of the 5×7 dot-matrix face, relative to the main font size.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Matrix {
    /// Height of the whole cell; the grid is centred in it.
    pub line: f32,
    /// Distance between neighbouring dot centres.
    pub pitch: f32,
    /// Dot radius.
    pub dot: f32,
    /// Horizontal space between neighbouring cells.
    pub spacing: f32,
}

impl Default for Matrix {
    fn default() -> Self {
        Self { line: 1.0, pitch: 0.125, dot: 0.047, spacing: 0.16 }
    }
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
    /// Green LED matrix, the station-clock look.
    Green,
    /// Red seven-segment, the studio-counter look.
    Red,
    /// Yellow LED matrix.
    Yellow,
    /// Dual colour: green time, red counters and seconds ring.
    Studio,
}

impl Palette {
    pub const ALL: [Self; 8] =
        [Self::Default, Self::Warm, Self::Cool, Self::Amber, Self::Green, Self::Red, Self::Yellow, Self::Studio];

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.id().eq_ignore_ascii_case(id))
    }

    pub fn id(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Warm => "warm",
            Self::Cool => "cool",
            Self::Amber => "amber",
            Self::Green => "green",
            Self::Red => "red",
            Self::Yellow => "yellow",
            Self::Studio => "studio",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Default => "Default",
            Self::Warm => "Warm",
            Self::Cool => "Cool",
            Self::Amber => "Amber",
            Self::Green => "Green matrix",
            Self::Red => "Red seven-segment",
            Self::Yellow => "Yellow matrix",
            Self::Studio => "Studio (green / red)",
        }
    }
}
serde_by_id!(Palette, "palette");

impl Theme {
    /// The theme to draw with: a preset, optionally dimmed for night mode.
    /// `night_dim` is the factor applied to the readout colours (1.0 = none).
    /// Under a chroma key the open dot takes the text colour: a green dot
    /// over `#00FF00` is exactly what a keyer is told to remove.
    pub fn resolve(palette: Palette, night_dim: Option<f32>, chroma: bool) -> Self {
        let mut theme = Self::default();
        let c = &mut theme.color;
        // The lime of a green LED matrix rather than an emerald.
        let led_green = Color32::from_rgb(140, 250, 70);
        let led_red = Color32::from_rgb(255, 70, 60);
        let led_yellow = Color32::from_rgb(255, 200, 40);
        // Single-colour presets have one readout colour; the counters follow it.
        let (text, alert, secondary) = match palette {
            Palette::Default => (c.text, c.alert, None),
            Palette::Warm => (Color32::from_rgb(255, 221, 170), Color32::from_rgb(245, 120, 60), None),
            Palette::Cool => (Color32::from_rgb(200, 228, 255), Color32::from_rgb(255, 170, 60), None),
            Palette::Amber => (Color32::from_rgb(245, 180, 60), Color32::from_rgb(240, 80, 70), None),
            Palette::Green => (led_green, led_red, None),
            Palette::Red => (led_red, led_yellow, None),
            Palette::Yellow => (led_yellow, led_red, None),
            Palette::Studio => (led_green, led_yellow, Some(led_red)),
        };
        (c.text, c.alert, c.secondary) = (text, alert, secondary.unwrap_or(text));
        // The quarter markers on the seconds ring are red LEDs, unless the
        // ring itself is red, when they are the amber ones.
        c.ring_marker = if palette == Palette::Red { led_yellow } else { led_red };
        if chroma {
            c.open = c.text;
        }
        if let Some(dim) = night_dim.filter(|d| *d < 1.0) {
            c.text = c.text.gamma_multiply(dim);
            c.alert = c.alert.gamma_multiply(dim);
            c.secondary = c.secondary.gamma_multiply(dim);
            c.open = c.open.gamma_multiply(dim);
            c.label = c.label.gamma_multiply(dim);
            c.ring_marker = c.ring_marker.gamma_multiply(dim);
        }
        theme
    }

    /// The unlit segments behind a digital readout in `color`: the faint
    /// "off" state of every diode, the way a real display shows its whole
    /// figure-eight under the digit. Always at the ghost alpha, however
    /// far night mode or a dimmed row has already faded `color`.
    pub fn ghost(&self, color: Color32) -> Color32 {
        let alpha = f32::from(color.a()) / 255.0;
        if alpha <= 0.0 {
            return color;
        }
        color.gamma_multiply((f32::from(self.color.ghost) / 255.0 / alpha).min(1.0))
    }

    /// `color` faded by `factor`, but never below [`TEXT_ALPHA_FLOOR`]: a
    /// caption dimmed on top of night mode must still clear the floor. The
    /// factor is raised rather than the alpha clamped, because `Color32` is
    /// premultiplied and clamping one channel would wash the colour out.
    pub fn dim(color: Color32, factor: f32) -> Color32 {
        let alpha = f32::from(color.a()) / 255.0;
        if alpha <= TEXT_ALPHA_FLOOR {
            return color;
        }
        color.gamma_multiply(factor.max(TEXT_ALPHA_FLOOR / alpha).min(1.0))
    }

    /// A caption or a closed board row over a backdrop.
    pub fn dim_caption(&self, color: Color32) -> Color32 {
        Self::dim(color, self.color.caption_dim)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Colors {
    pub text: Color32,
    /// Stopwatch and timer digits, and the lit LEDs of the seconds ring. The
    /// same as `text` except in a dual-colour preset.
    pub secondary: Color32,
    /// Printed labels on the board (city names, exchange codes, AM/PM):
    /// plain white, whatever colour the LEDs are.
    pub label: Color32,
    /// Hairlines between the board's cells.
    pub separator: Color32,
    /// Alpha of the unlit segments behind a digital digit (5–8 %).
    pub ghost: u8,
    /// Seconds ring: the unlit LED's faint tint of `secondary`, the dark
    /// glow every LED sits in, the bloom around the LED that just lit, and
    /// the colour of the four quarter markers.
    pub ring_unlit: u8,
    pub ring_socket: u8,
    pub ring_bloom: u8,
    pub ring_marker: Color32,
    /// Applied to the caption when a backdrop makes full strength unnecessary.
    pub caption_dim: f32,
    /// Timer that has run out.
    pub alert: Color32,
    /// The dot beside a market that is trading right now.
    pub open: Color32,
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
    /// Market board: the row time as a fraction of the main font, with a
    /// floor in points so the smallest size stays legible (requirement L4).
    pub board_time: f32,
    pub board_time_min: f32,
    /// Row pitch as a multiple of the row font size.
    pub board_pitch: f32,
    /// Caption-relative: the gap between columns, the status dot's radius
    /// and the ring stroke of a closed market.
    pub board_gap: f32,
    pub board_dot: f32,
    pub board_ring: f32,
    /// The board's printed labels as a fraction of the row font: bold and
    /// large, like the legends beside a hardware clock's digits.
    pub board_label: f32,
    /// Seconds ring: LED radius relative to the caption font, and the band
    /// the ring needs around the content as a multiple of that radius.
    pub ring_dot: f32,
    pub ring_band: f32,
}

impl Default for Colors {
    fn default() -> Self {
        Self {
            text: Color32::from_rgb(242, 242, 240),
            secondary: Color32::from_rgb(242, 242, 240),
            label: Color32::WHITE,
            separator: Color32::from_white_alpha(24),
            ghost: 20,
            ring_unlit: 40,
            ring_socket: 90,
            ring_bloom: 70,
            ring_marker: Color32::from_rgb(255, 70, 60),
            caption_dim: 0.62,
            alert: Color32::from_rgb(245, 165, 36),
            open: Color32::from_rgb(88, 214, 122),
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
            board_time: 0.5,
            board_time_min: 16.0,
            board_pitch: 1.45,
            board_gap: 0.9,
            board_dot: 0.28,
            board_ring: 0.12,
            board_label: 0.6,
            ring_dot: 0.16,
            ring_band: 5.0,
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
        assert_eq!(Theme::resolve(Palette::Default, None, false), Theme::default());
        assert_eq!(Theme::resolve(Palette::Default, Some(1.0), false), Theme::default());
    }

    #[test]
    fn every_palette_keeps_the_contrast_and_keying_tokens() {
        let base = Theme::default();
        for palette in Palette::ALL {
            let t = Theme::resolve(palette, None, false);
            assert_eq!(t.color.chroma, base.color.chroma, "{palette:?} must not touch the key colour");
            assert_eq!(t.color.halo, base.color.halo, "{palette:?}");
            assert_eq!(t.color.backdrop, base.color.backdrop, "{palette:?}");
            assert_eq!(t.color.control_base, base.color.control_base, "{palette:?}");
            assert_eq!(t.ratio, base.ratio, "{palette:?} must not change the layout");
            assert_eq!(t.segments, base.segments, "{palette:?} must not change the digital face");
            assert_eq!(t.matrix, base.matrix, "{palette:?} must not change the matrix face");
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
    fn matrix_dots_fit_their_pitch_and_the_line_box() {
        let m = Matrix::default();
        assert!(m.dot * 2.0 < m.pitch, "dots must not touch");
        assert!(6.0 * m.pitch + 2.0 * m.dot <= m.line, "seven rows must fit the line box");
    }

    #[test]
    fn palettes_differ_in_their_readout_colours() {
        let pairs: Vec<_> = Palette::ALL
            .iter()
            .map(|&p| {
                let c = Theme::resolve(p, None, false).color;
                (c.text, c.secondary)
            })
            .collect();
        for (i, a) in pairs.iter().enumerate() {
            for b in &pairs[i + 1..] {
                assert_ne!(a, b, "two presets share their readout colours");
            }
        }
    }

    /// Only the dual-colour preset splits time from counters; every other
    /// preset keeps one readout colour, so nothing about them changes.
    #[test]
    fn only_the_studio_preset_has_a_second_colour() {
        for palette in Palette::ALL {
            let c = Theme::resolve(palette, None, false).color;
            if palette == Palette::Studio {
                assert_ne!(c.secondary, c.text);
                assert_ne!(c.secondary, c.alert, "counters must not read as an alarm");
            } else {
                assert_eq!(c.secondary, c.text, "{palette:?}");
            }
        }
        let dim = Theme::resolve(Palette::Studio, Some(0.7), false).color;
        let full = Theme::resolve(Palette::Studio, None, false).color;
        assert!(dim.secondary.a() < full.secondary.a(), "night mode dims the counters too");
    }

    /// Ghosting is a whisper: five to eight percent, per the hardware brief.
    #[test]
    fn ghost_segments_sit_between_five_and_eight_percent() {
        let t = Theme::default();
        let g = t.ghost(t.color.text);
        let alpha = f32::from(g.a()) / 255.0;
        assert!((0.05..=0.08).contains(&alpha), "ghost alpha {alpha}");
        assert!(g.r() < t.color.text.r(), "premultiplied: the channels fade with the alpha");
        // Night mode has already faded the readout; the ghost must not fade with it.
        let night = Theme::resolve(Palette::Default, Some(TEXT_ALPHA_FLOOR), false);
        let g = night.ghost(night.color.text);
        let alpha = f32::from(g.a()) / 255.0;
        assert!((0.05..=0.08).contains(&alpha), "ghost alpha under night dim {alpha}");
        let dimmed_row = night.ghost(night.dim_caption(night.color.text));
        assert!((0.05..=0.08).contains(&(f32::from(dimmed_row.a()) / 255.0)));
    }

    #[test]
    fn labels_are_white_and_markers_are_red_except_on_a_red_ring() {
        for palette in Palette::ALL {
            let c = Theme::resolve(palette, None, false).color;
            assert_eq!(c.label, Color32::WHITE, "{palette:?}");
            if palette == Palette::Red {
                assert_ne!(c.ring_marker, c.secondary, "markers must stand out from a red ring");
            } else {
                assert_eq!(c.ring_marker, Color32::from_rgb(255, 70, 60), "{palette:?}");
            }
        }
        let night = Theme::resolve(Palette::Default, Some(0.7), false).color;
        assert!(night.label.a() < 255, "night mode dims the printed labels too");
        assert!(night.ring_marker.a() < 255);
    }

    #[test]
    fn ring_ratios_leave_room_for_the_dots() {
        let r = Ratios::default();
        assert!(r.ring_band >= 3.0, "the band must clear the dot on both sides");
        assert!(r.ring_dot > 0.0 && r.ring_dot < r.board_dot);
    }

    #[test]
    fn night_dim_fades_the_readout_and_nothing_else() {
        let base = Theme::resolve(Palette::Default, None, false);
        let dim = Theme::resolve(Palette::Default, Some(0.7), false);
        assert!(dim.color.text.a() < base.color.text.a());
        assert!(dim.color.alert.a() < base.color.alert.a());
        assert!(dim.color.open.a() < base.color.open.a(), "the open dot is part of the readout");
        assert_eq!(dim.color.chroma, base.color.chroma);
        assert_eq!(dim.color.backdrop, base.color.backdrop);
        assert_eq!(dim.color.halo, base.color.halo);
        assert_eq!(dim.color.control_glyph, base.color.control_glyph);
    }

    /// Caption dimming alone sits just above the floor; stacked on night
    /// mode it would fall to 0.43, so the floor has to hold it up.
    #[test]
    fn dimming_never_falls_below_the_alpha_floor() {
        let floor = (TEXT_ALPHA_FLOOR * 255.0) as u8;
        let base = Theme::resolve(Palette::Default, None, false);
        let plain = base.dim_caption(base.color.text);
        assert!(plain.a() >= floor && plain.a() < base.color.text.a(), "alpha {}", plain.a());

        let night = Theme::resolve(Palette::Default, Some(0.7), false);
        let stacked = night.dim_caption(night.color.text);
        assert!(stacked.a() >= floor, "stacked alpha {} is under the floor {floor}", stacked.a());
        assert!(stacked.a() <= night.color.text.a(), "dimming must never brighten");

        // Already at the floor: left exactly alone.
        let at_floor = Theme::resolve(Palette::Default, Some(TEXT_ALPHA_FLOOR), false);
        assert_eq!(at_floor.dim_caption(at_floor.color.text), at_floor.color.text);
        // Premultiplied colour keeps its hue: every channel scales together.
        let warm = Theme::resolve(Palette::Warm, None, false);
        let dimmed = warm.dim_caption(warm.color.text);
        let ratio = f32::from(dimmed.r()) / f32::from(warm.color.text.r());
        assert!((f32::from(dimmed.b()) / f32::from(warm.color.text.b()) - ratio).abs() < 0.02);
    }

    /// A keyer removes everything near `#00FF00`, so the trading dot must
    /// not be green over a chroma background.
    #[test]
    fn under_chroma_the_open_dot_is_not_green() {
        let plain = Theme::resolve(Palette::Default, None, false);
        let keyed = Theme::resolve(Palette::Default, None, true);
        assert_ne!(plain.color.open, plain.color.text);
        assert_eq!(keyed.color.open, keyed.color.text);
        assert_eq!(keyed.color.chroma, plain.color.chroma);
    }

    #[test]
    fn the_floor_matches_the_config_clamp() {
        assert_eq!(crate::config::MIN_NIGHT_DIM, TEXT_ALPHA_FLOOR);
    }

    #[test]
    fn board_ratios_keep_the_row_legible() {
        let r = Ratios::default();
        assert!(r.board_time_min >= 16.0, "row digits must clear the L4 floor at the smallest size");
        assert!(r.board_pitch > 1.0, "rows must not overlap");
    }

    #[test]
    fn palette_ids_round_trip() {
        for palette in Palette::ALL {
            assert_eq!(Palette::from_id(palette.id()), Some(palette));
        }
        assert_eq!(Palette::from_id("neon"), None);
    }
}
