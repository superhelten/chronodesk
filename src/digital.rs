//! The seven-segment face: readout characters built from convex polygons.
//!
//! A clock only ever shows digits, colons and a point, which is exactly the
//! alphabet of a seven-segment display, so the face needs no font data, no
//! atlas and no licence: each character is a handful of hexagons computed
//! once per layout rebuild from the theme's proportions and then painted by
//! translation. The caption stays in the typeface, the way a real display
//! prints its labels beside the segments.

use eframe::egui::{Color32, Painter, Pos2, Shape, Stroke, pos2};

use crate::layout::{Glyph, Glyphs};
use crate::theme::{Segments, Theme};

/// Which segments each digit lights, in the conventional order `a`–`g`:
/// top, upper right, lower right, bottom, lower left, upper left, middle.
const DIGITS: [&str; 10] = ["abcdef", "bc", "abdeg", "abcdg", "bcfg", "acdfg", "acdefg", "abc", "abcdefg", "abcdfg"];

/// Every readout character at `font` size, in cell coordinates.
///
/// Digits are laid out in an upright box first, then sheared for the lean and
/// shifted so nothing pokes left of the cell. A cell is the box, the lean's
/// horizontal reach, and the spacing shared out to both sides.
pub fn glyphs(font: f32, seg: &Segments) -> Glyphs {
    let (h, w, t) = (font * seg.height, font * seg.width, font * seg.thickness);
    let (gap, spacing, line) = (font * seg.gap, font * seg.spacing, font * seg.line);
    let lean = seg.slant * h;
    let top = (line - h) / 2.0;
    let place = |p: Pos2| pos2(p.x + seg.slant * (h / 2.0 - p.y) + lean / 2.0 + spacing / 2.0, p.y + top);

    // Segment centre lines, inset by half a thickness so the box is the outline.
    let (x0, x1) = (t / 2.0, w - t / 2.0);
    let (y0, ym, y1) = (t / 2.0, h / 2.0, h - t / 2.0);
    let centre_line = |s: char| -> (Pos2, Pos2) {
        match s {
            'a' => (pos2(x0, y0), pos2(x1, y0)),
            'b' => (pos2(x1, y0), pos2(x1, ym)),
            'c' => (pos2(x1, ym), pos2(x1, y1)),
            'd' => (pos2(x0, y1), pos2(x1, y1)),
            'e' => (pos2(x0, ym), pos2(x0, y1)),
            'f' => (pos2(x0, y0), pos2(x0, ym)),
            'g' => (pos2(x0, ym), pos2(x1, ym)),
            _ => unreachable!("segment {s}"),
        }
    };
    let dot = |cy: f32| -> Vec<Pos2> {
        let (cx, r) = (t / 2.0, t / 2.0);
        [pos2(cx - r, cy - r), pos2(cx + r, cy - r), pos2(cx + r, cy + r), pos2(cx - r, cy + r)]
            .into_iter()
            .map(place)
            .collect()
    };

    let mut glyphs = Glyphs { digit_width: w + lean + spacing, height: line, ..Default::default() };
    for (d, lit) in DIGITS.iter().enumerate() {
        let polygons = lit
            .chars()
            .map(|s| {
                let (p0, p1) = centre_line(s);
                segment(p0, p1, t, gap).into_iter().map(place).collect()
            })
            .collect();
        glyphs.glyphs.insert(char::from_digit(d as u32, 10).expect("digit"), Glyph::Digital(polygons));
    }
    let dot_width = t + lean + spacing;
    glyphs.widths.insert(':', dot_width);
    glyphs.widths.insert('.', dot_width);
    glyphs.glyphs.insert(':', Glyph::Digital(vec![dot(h * 0.3), dot(h * 0.7)]));
    glyphs.glyphs.insert('.', Glyph::Digital(vec![dot(h - t / 2.0)]));
    glyphs
}

/// A segment as the classic hexagon: a bar `t` thick along `p0`→`p1` whose
/// ends taper to points, each held `gap` short of the corner it shares with
/// its neighbours so the corners stay open.
fn segment(p0: Pos2, p1: Pos2, t: f32, gap: f32) -> [Pos2; 6] {
    let d = (p1 - p0).normalized();
    let n = d.rot90() * (t / 2.0);
    let (tip0, tip1) = (p0 + d * gap, p1 - d * gap);
    let (b0, b1) = (p0 + d * (gap + t / 2.0), p1 - d * (gap + t / 2.0));
    [tip0, b0 + n, b1 + n, tip1, b1 - n, b0 - n]
}

/// Paints one cached glyph with its top-left corner at `pos`, haloed by a
/// stroke `outline` points wide around every segment when asked.
///
/// egui centres a path stroke on the edge, so a stroke twice the halo width
/// reaches exactly `outline` beyond the fill that is drawn over it.
pub fn paint(painter: &Painter, pos: Pos2, polygons: &[Vec<Pos2>], color: Color32, outline: Option<f32>, theme: &Theme) {
    let offset = pos.to_vec2();
    let placed = |poly: &Vec<Pos2>| -> Vec<Pos2> { poly.iter().map(|p| *p + offset).collect() };
    if let Some(width) = outline {
        let shade = Color32::from_black_alpha(theme.color.halo_stroke);
        for poly in polygons {
            painter.add(Shape::convex_polygon(placed(poly), shade, Stroke::new(width * 2.0, shade)));
        }
    }
    for poly in polygons {
        painter.add(Shape::convex_polygon(placed(poly), color, Stroke::NONE));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::READOUT_CHARS;

    const FONT: f32 = 44.0;

    fn build() -> Glyphs {
        glyphs(FONT, &Segments::default())
    }

    fn polygons(glyphs: &Glyphs, c: char) -> &Vec<Vec<Pos2>> {
        match glyphs.glyphs.get(&c) {
            Some(Glyph::Digital(p)) => p,
            other => panic!("{c:?}: expected a digital glyph, got {other:?}"),
        }
    }

    fn cell_width(glyphs: &Glyphs, c: char) -> f32 {
        if c.is_ascii_digit() { glyphs.digit_width } else { glyphs.widths[&c] }
    }

    /// Signed area via the shoelace formula; the sign gives the winding.
    fn cross(o: Pos2, a: Pos2, b: Pos2) -> f32 {
        (a.x - o.x) * (b.y - o.y) - (a.y - o.y) * (b.x - o.x)
    }

    fn is_convex(poly: &[Pos2]) -> bool {
        let n = poly.len();
        let signs: Vec<f32> = (0..n).map(|i| cross(poly[i], poly[(i + 1) % n], poly[(i + 2) % n])).collect();
        signs.iter().all(|&s| s >= 0.0) || signs.iter().all(|&s| s <= 0.0)
    }

    #[test]
    fn covers_exactly_the_readout_alphabet() {
        let g = build();
        let mut chars: Vec<char> = g.glyphs.keys().copied().collect();
        chars.sort_unstable();
        let mut expected: Vec<char> = READOUT_CHARS.chars().collect();
        expected.sort_unstable();
        assert_eq!(chars, expected);
    }

    #[test]
    fn each_digit_lights_the_conventional_segments() {
        let g = build();
        for (d, segments) in DIGITS.iter().enumerate() {
            let c = char::from_digit(d as u32, 10).unwrap();
            assert_eq!(polygons(&g, c).len(), segments.len(), "digit {c}");
        }
        assert_eq!(polygons(&g, ':').len(), 2, "a colon is two dots");
        assert_eq!(polygons(&g, '.').len(), 1, "a point is one dot");
    }

    #[test]
    fn every_segment_is_a_convex_hexagon_inside_its_cell() {
        let g = build();
        for c in READOUT_CHARS.chars() {
            let (w, h) = (cell_width(&g, c), g.height);
            for poly in polygons(&g, c) {
                assert!(poly.len() == 6 || poly.len() == 4, "{c:?}: {} points", poly.len());
                assert!(is_convex(poly), "{c:?}: {poly:?}");
                for p in poly {
                    assert!(p.x >= 0.0 && p.x <= w, "{c:?}: x={} outside 0..{w}", p.x);
                    assert!(p.y >= 0.0 && p.y <= h, "{c:?}: y={} outside 0..{h}", p.y);
                }
            }
        }
    }

    #[test]
    fn digits_share_one_cell_and_punctuation_is_narrower() {
        let g = build();
        assert!(g.digit_width > 0.0);
        assert!(g.widths[&':'] < g.digit_width);
        assert!(g.widths[&'.'] < g.digit_width);
        assert_eq!(g.widths[&':'], g.widths[&'.']);
        assert_eq!(g.height, FONT * Segments::default().line);
    }

    #[test]
    fn the_one_sits_in_the_right_half_of_its_cell() {
        let g = build();
        let mid = g.digit_width / 2.0;
        for poly in polygons(&g, '1') {
            for p in poly {
                assert!(p.x > mid, "{p:?} is left of the cell centre");
            }
        }
    }

    #[test]
    fn the_lean_puts_the_top_right_of_the_bottom() {
        let g = build();
        let top = polygons(&g, '8')[0].iter().map(|p| p.x).sum::<f32>() / 6.0;
        let bottom = polygons(&g, '8')[3].iter().map(|p| p.x).sum::<f32>() / 6.0;
        assert!(top > bottom, "segment a (top) at x={top}, segment d (bottom) at x={bottom}");
    }

    #[test]
    fn everything_scales_linearly_with_the_font_size() {
        let seg = Segments::default();
        let (small, large) = (glyphs(20.0, &seg), glyphs(40.0, &seg));
        assert!((large.digit_width - 2.0 * small.digit_width).abs() < 1e-4);
        assert!((large.height - 2.0 * small.height).abs() < 1e-4);
        let (a, b) = (polygons(&small, '4'), polygons(&large, '4'));
        for (pa, pb) in a.iter().zip(b) {
            for (x, y) in pa.iter().zip(pb) {
                assert!((y.x - 2.0 * x.x).abs() < 1e-3 && (y.y - 2.0 * x.y).abs() < 1e-3);
            }
        }
    }

    #[test]
    fn the_digit_is_centred_vertically_in_the_line_box() {
        let g = build();
        let ys: Vec<f32> = polygons(&g, '8').iter().flatten().map(|p| p.y).collect();
        let (top, bottom) = (ys.iter().cloned().fold(f32::MAX, f32::min), ys.iter().cloned().fold(f32::MIN, f32::max));
        assert!((top - (g.height - bottom)).abs() < 1e-3, "top margin {top}, bottom margin {}", g.height - bottom);
    }

    #[test]
    fn neighbouring_segments_do_not_touch() {
        // The tips of a and f meet at the top-left corner of an 8; a gap must
        // separate them, or the corners fill in and read as a solid blob.
        let g = build();
        let eight = polygons(&g, '8');
        let (a, f) = (&eight[0], &eight[5]);
        let a_tip = a.iter().min_by(|p, q| p.x.total_cmp(&q.x)).unwrap();
        let f_tip = f.iter().min_by(|p, q| p.y.total_cmp(&q.y)).unwrap();
        let seg = Segments::default();
        let gap = FONT * seg.gap;
        // Both tips are set back from the shared corner by the gap along
        // their own axes; the lean shears the horizontal one a little more.
        assert!((f_tip.y - a_tip.y - gap).abs() < 1e-3, "a tip {a_tip:?} vs f tip {f_tip:?}");
        assert!((a_tip.x - f_tip.x - gap * (1.0 + seg.slant)).abs() < 1e-3, "a tip {a_tip:?} vs f tip {f_tip:?}");
    }
}
