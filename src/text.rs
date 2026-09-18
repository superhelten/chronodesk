//! The typeface face: the readout and caption drawn with a font.
//!
//! `install_display_font` and `measure_glyphs` are the only code that touches
//! egui's font atlas. The digital face (`digital.rs`) never does, which is
//! what keeps a face switch a plain layout rebuild rather than a font reload.
//! `paint_readout` draws whichever face the cache holds.

use std::sync::Arc;

use eframe::egui::{self, Color32, FontFamily, FontId, Galley, Pos2, pos2, vec2};

use crate::digital;
use crate::layout::{self, GHOST_CHAR, Glyph, Glyphs};
use crate::theme::Theme;

const DISPLAY_FONT: &str = "display";
const LABEL_FONT: &str = "display-bold";

pub fn display_family() -> FontFamily {
    FontFamily::Name(DISPLAY_FONT.into())
}

/// The board's printed labels: a bold face, like the legends silk-screened
/// beside a hardware clock's digits.
pub fn label_family() -> FontFamily {
    FontFamily::Name(LABEL_FONT.into())
}

/// Uses a light system UI face for the readout and its bold cut for the
/// labels when they are available, falling back to egui's bundled font so
/// the app never depends on them.
pub fn install_display_font(ctx: &egui::Context) {
    const CANDIDATES: &[&str] = &[
        r"C:\Windows\Fonts\segoeuisl.ttf",
        r"C:\Windows\Fonts\segoeui.ttf",
        "/System/Library/Fonts/SFNS.ttf",
        "/System/Library/Fonts/Helvetica.ttc",
    ];
    const BOLD: &[&str] = &[r"C:\Windows\Fonts\segoeuib.ttf", "/System/Library/Fonts/Helvetica.ttc"];
    let mut fonts = egui::FontDefinitions::default();
    let base = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    let mut family = base.clone();
    if let Some(bytes) = CANDIDATES.iter().find_map(|path| std::fs::read(path).ok()) {
        fonts.font_data.insert(DISPLAY_FONT.to_owned(), Arc::new(egui::FontData::from_owned(bytes)));
        family.insert(0, DISPLAY_FONT.to_owned());
    }
    let mut bold = base;
    if let Some(bytes) = BOLD.iter().find_map(|path| std::fs::read(path).ok()) {
        fonts.font_data.insert(LABEL_FONT.to_owned(), Arc::new(egui::FontData::from_owned(bytes)));
        bold.insert(0, LABEL_FONT.to_owned());
    }
    fonts.families.insert(display_family(), family);
    fonts.families.insert(label_family(), bold);
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

/// Draws a readout line from cached glyphs, its top-left corner at `origin`.
/// Every character sits in its cell, so the line's width is `glyphs.width_of`
/// and nothing is measured here.
///
/// With `ghost` set, every LED-face digit is drawn over its fully lit figure
/// (cached under `GHOST_CHAR`) in that colour, so the readout reads as a
/// display whose diodes are all there and only some of them on. The
/// typeface has no such thing.
#[allow(clippy::too_many_arguments, reason = "a painting call site, not an API")]
pub fn paint_readout(
    painter: &egui::Painter,
    origin: Pos2,
    text: &str,
    glyphs: &Glyphs,
    color: Color32,
    halo: Option<f32>,
    ghost: Option<Color32>,
    theme: &Theme,
) {
    let full = ghost.and_then(|_| match glyphs.glyph(GHOST_CHAR) {
        Some(Glyph::Digital(polygons)) => Some(polygons),
        _ => None,
    });
    let mut x = origin.x;
    for c in text.chars() {
        let cell = glyphs.cell_width(c);
        match glyphs.glyph(c) {
            Some(Glyph::Text(galley)) => {
                let pos = pos2(x + (cell - galley.size().x) / 2.0, origin.y);
                paint_galley(painter, pos, galley.clone(), color, halo, theme);
            }
            // Already laid out inside its cell.
            Some(Glyph::Digital(polygons)) => {
                if c.is_ascii_digit()
                    && let (Some(full), Some(ghost)) = (full, ghost)
                {
                    digital::paint(painter, pos2(x, origin.y), full, ghost, None, theme);
                }
                digital::paint(painter, pos2(x, origin.y), polygons, color, halo, theme);
            }
            None => {}
        }
        x += cell;
    }
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
