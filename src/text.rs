//! The typeface face: the readout and caption drawn with a font.
//!
//! Everything here is the only code that touches egui's font atlas. The
//! digital face (`digital.rs`) never does, which is what keeps a face switch
//! a plain layout rebuild rather than a font reload.

use std::sync::Arc;

use eframe::egui::{self, Color32, FontFamily, FontId, Galley, Pos2, vec2};

use crate::layout::{self, Glyph, Glyphs};
use crate::theme::Theme;

const DISPLAY_FONT: &str = "display";

pub fn display_family() -> FontFamily {
    FontFamily::Name(DISPLAY_FONT.into())
}

/// Uses a light system UI face for the readout when one is available, falling
/// back to egui's bundled font so the app never depends on it.
pub fn install_display_font(ctx: &egui::Context) {
    const CANDIDATES: &[&str] = &[
        r"C:\Windows\Fonts\segoeuisl.ttf",
        r"C:\Windows\Fonts\segoeui.ttf",
        "/System/Library/Fonts/SFNS.ttf",
        "/System/Library/Fonts/Helvetica.ttc",
    ];
    let mut fonts = egui::FontDefinitions::default();
    let mut family = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    if let Some(bytes) = CANDIDATES.iter().find_map(|path| std::fs::read(path).ok()) {
        fonts.font_data.insert(DISPLAY_FONT.to_owned(), Arc::new(egui::FontData::from_owned(bytes)));
        family.insert(0, DISPLAY_FONT.to_owned());
    }
    fonts.families.insert(display_family(), family);
    ctx.set_fonts(fonts);
}

/// Lays out every character a readout can contain, once per font size. Digits
/// share the widest digit's cell so the readout doesn't jitter as it ticks,
/// whatever the font's own digit metrics are.
pub fn measure_glyphs(ctx: &egui::Context, font_size: f32) -> Glyphs {
    let font = FontId::new(font_size, display_family());
    ctx.fonts_mut(|fonts| {
        let mut layout = |c: char| fonts.layout_no_wrap(c.to_string(), font.clone(), Color32::WHITE);
        let digit_width = ('0'..='9').map(|d| layout(d).size().x).fold(0.0, f32::max);
        let mut glyphs = Glyphs { digit_width, ..Default::default() };
        for c in layout::READOUT_CHARS.chars() {
            let galley = layout(c);
            glyphs.height = glyphs.height.max(galley.size().y);
            if !c.is_ascii_digit() {
                glyphs.widths.insert(c, galley.size().x);
            }
            glyphs.glyphs.insert(c, Glyph::Text(galley));
        }
        glyphs
    })
}

/// Draws `galley`, optionally haloed by a dark edge `width` points thick.
///
/// egui has no outlined text, so the halo is offset copies of the glyph. Two
/// rings of eight, the outer one fainter, read as a soft dark edge; a single
/// hard ring at a width thin strokes can stand turns the readout into a hollow
/// outline font instead.
pub fn paint_galley(
    painter: &egui::Painter,
    pos: Pos2,
    galley: Arc<Galley>,
    color: Color32,
    outline: Option<f32>,
    theme: &Theme,
) {
    if let Some(width) = outline {
        for (radius, alpha) in [(width, theme.color.halo[0]), (width * 2.1, theme.color.halo[1])] {
            let diagonal = radius * std::f32::consts::FRAC_1_SQRT_2;
            let ring = [
                vec2(radius, 0.0),
                vec2(-radius, 0.0),
                vec2(0.0, radius),
                vec2(0.0, -radius),
                vec2(diagonal, diagonal),
                vec2(diagonal, -diagonal),
                vec2(-diagonal, diagonal),
                vec2(-diagonal, -diagonal),
            ];
            let shade = Color32::from_black_alpha(alpha);
            for offset in ring {
                painter.galley_with_override_text_color(pos + offset, galley.clone(), shade);
            }
        }
    }
    painter.galley_with_override_text_color(pos, galley, color);
}

/// Letter-spaced caption ("T I M E R"-lite): thin spaces read cleaner at small sizes.
pub fn spaced(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 4);
    for (i, c) in s.chars().enumerate() {
        if i > 0 {
            out.push('\u{2009}');
        }
        out.push(c);
    }
    out
}
