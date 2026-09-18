//! The dot-matrix face: readout characters as a 5×7 grid of round LEDs,
//! the way multi-zone and studio hardware clocks draw their digits.
//!
//! Like the seven-segment face it is pure geometry — no font, no atlas —
//! and it plugs into the same glyph cache: every dot is a small convex
//! polygon in cell coordinates, computed once per layout rebuild from the
//! theme's proportions and painted by translation. The full grid is cached
//! under [`GHOST_CHAR`] so the unlit dots can be drawn under every digit.

use std::f32::consts::TAU;

use eframe::egui::{Pos2, pos2};

use crate::layout::{GHOST_CHAR, Glyph, Glyphs};
use crate::theme::Matrix;

pub const COLS: usize = 5;
pub const ROWS: usize = 7;
/// A dot as a polygon: enough sides to read as round at every size.
const DOT_SIDES: usize = 12;

/// The classic 5×7 LED font, top row first, `#` for a lit dot.
const DIGITS: [[&str; ROWS]; 10] = [
    [".###.", "#...#", "#..##", "#.#.#", "##..#", "#...#", ".###."],
    ["..#..", ".##..", "..#..", "..#..", "..#..", "..#..", ".###."],
    [".###.", "#...#", "....#", "...#.", "..#..", ".#...", "#####"],
    ["#####", "...#.", "..#..", "...#.", "....#", "#...#", ".###."],
    ["...#.", "..##.", ".#.#.", "#..#.", "#####", "...#.", "...#."],
    ["#####", "#....", "####.", "....#", "....#", "#...#", ".###."],
    ["..##.", ".#...", "#....", "####.", "#...#", "#...#", ".###."],
    ["#####", "....#", "...#.", "..#..", ".#...", ".#...", ".#..."],
    [".###.", "#...#", "#...#", ".###.", "#...#", "#...#", ".###."],
    [".###.", "#...#", "#...#", ".####", "....#", "...#.", ".##.."],
];

/// Every readout character at `font` size, in cell coordinates.
///
/// A digit cell is five columns of dots plus the spacing shared out to both
/// sides; the colon and the point are one column wide. The grid is centred
/// in a line box `font * line` tall, so the face sits on the same baseline
/// logic as the others.
pub fn glyphs(font: f32, mx: &Matrix) -> Glyphs {
    let (pitch, r, spacing, line) = (font * mx.pitch, font * mx.dot, font * mx.spacing, font * mx.line);
    let grid_height = (ROWS - 1) as f32 * pitch + 2.0 * r;
    let top = (line - grid_height) / 2.0 + r;
    let centre = |col: usize, row: usize| pos2(spacing / 2.0 + r + col as f32 * pitch, top + row as f32 * pitch);
    let dot = |c: Pos2| -> Vec<Pos2> {
        (0..DOT_SIDES)
            .map(|i| {
                let a = TAU * i as f32 / DOT_SIDES as f32;
                pos2(c.x + r * a.cos(), c.y + r * a.sin())
            })
            .collect()
    };
    let grid = |lit: &dyn Fn(usize, usize) -> bool| -> Vec<Vec<Pos2>> {
        (0..ROWS).flat_map(|row| (0..COLS).filter(move |&col| lit(col, row)).map(move |col| (col, row))).map(|(c, r)| dot(centre(c, r))).collect()
    };

    let digit_width = (COLS - 1) as f32 * pitch + 2.0 * r + spacing;
    let mut glyphs = Glyphs { digit_width, height: line, ..Default::default() };
    for (d, pattern) in DIGITS.iter().enumerate() {
        let polygons = grid(&|col, row| pattern[row].as_bytes()[col] == b'#');
        glyphs.glyphs.insert(char::from_digit(d as u32, 10).expect("digit"), Glyph::Digital(polygons));
    }
    glyphs.glyphs.insert(GHOST_CHAR, Glyph::Digital(grid(&|_, _| true)));

    let narrow = 2.0 * r + spacing;
    glyphs.widths.insert(':', narrow);
    glyphs.widths.insert('.', narrow);
    glyphs.glyphs.insert(':', Glyph::Digital(vec![dot(centre(0, 2)), dot(centre(0, 4))]));
    glyphs.glyphs.insert('.', Glyph::Digital(vec![dot(centre(0, ROWS - 1))]));
    glyphs
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::READOUT_CHARS;

    const FONT: f32 = 44.0;

    fn build() -> Glyphs {
        glyphs(FONT, &Matrix::default())
    }

    fn polygons(glyphs: &Glyphs, c: char) -> &Vec<Vec<Pos2>> {
        match glyphs.glyphs.get(&c) {
            Some(Glyph::Digital(p)) => p,
            other => panic!("{c:?}: expected a matrix glyph, got {other:?}"),
        }
    }

    #[test]
    fn covers_the_readout_alphabet_and_the_ghost_grid() {
        let g = build();
        let mut chars: Vec<char> = g.glyphs.keys().copied().collect();
        chars.sort_unstable();
        let mut expected: Vec<char> = READOUT_CHARS.chars().chain([GHOST_CHAR]).collect();
        expected.sort_unstable();
        assert_eq!(chars, expected);
    }

    #[test]
    fn every_pattern_is_five_by_seven() {
        for (d, pattern) in DIGITS.iter().enumerate() {
            for row in pattern {
                assert_eq!(row.len(), COLS, "digit {d}: {row}");
                assert!(row.bytes().all(|b| b == b'#' || b == b'.'), "digit {d}: {row}");
            }
        }
    }

    #[test]
    fn the_ghost_grid_holds_every_dot_and_digits_are_subsets_of_it() {
        let g = build();
        let all = polygons(&g, GHOST_CHAR);
        assert_eq!(all.len(), COLS * ROWS);
        for d in '0'..='9' {
            let lit = polygons(&g, d);
            assert!(lit.len() < all.len(), "{d} lights fewer dots than the grid");
            for dot in lit {
                assert!(all.contains(dot), "{d}: a dot off the grid");
            }
        }
        assert_eq!(polygons(&g, '8').len(), 17, "the eight lights seventeen dots");
        assert_eq!(polygons(&g, '1').len(), 10);
    }

    #[test]
    fn dots_are_round_and_inside_their_cell() {
        let g = build();
        let r = FONT * Matrix::default().dot;
        for c in READOUT_CHARS.chars().chain([GHOST_CHAR]) {
            let width = if c.is_ascii_digit() || c == GHOST_CHAR { g.digit_width } else { g.widths[&c] };
            for poly in polygons(&g, c) {
                assert_eq!(poly.len(), DOT_SIDES);
                let cx = poly.iter().map(|p| p.x).sum::<f32>() / poly.len() as f32;
                let cy = poly.iter().map(|p| p.y).sum::<f32>() / poly.len() as f32;
                for p in poly {
                    let d = ((p.x - cx).powi(2) + (p.y - cy).powi(2)).sqrt();
                    assert!((d - r).abs() < 1e-3, "{c:?}: not a circle of radius {r}");
                    assert!(p.x >= 0.0 && p.x <= width, "{c:?}: x={} outside 0..{width}", p.x);
                    assert!(p.y >= 0.0 && p.y <= g.height, "{c:?}: y={} outside 0..{}", p.y, g.height);
                }
            }
        }
    }

    #[test]
    fn the_colon_is_two_dots_in_a_narrow_cell_between_the_rows_of_a_digit() {
        let g = build();
        let colon = polygons(&g, ':');
        assert_eq!(colon.len(), 2);
        assert!(g.widths[&':'] < g.digit_width / 2.0);
        let ys: Vec<f32> = colon.iter().map(|p| p.iter().map(|q| q.y).sum::<f32>() / p.len() as f32).collect();
        assert!(ys[0] < ys[1]);
        assert!(ys[0] > 0.0 && ys[1] < g.height);
    }

    #[test]
    fn the_grid_is_centred_in_the_line_box() {
        let g = build();
        let ys: Vec<f32> = polygons(&g, GHOST_CHAR).iter().flatten().map(|p| p.y).collect();
        let (top, bottom) = (ys.iter().cloned().fold(f32::MAX, f32::min), ys.iter().cloned().fold(f32::MIN, f32::max));
        assert!((top - (g.height - bottom)).abs() < 1e-3, "top margin {top}, bottom margin {}", g.height - bottom);
    }

    #[test]
    fn everything_scales_linearly_with_the_font_size() {
        let mx = Matrix::default();
        let (small, large) = (glyphs(20.0, &mx), glyphs(40.0, &mx));
        assert!((large.digit_width - 2.0 * small.digit_width).abs() < 1e-4);
        assert!((large.height - 2.0 * small.height).abs() < 1e-4);
    }
}
