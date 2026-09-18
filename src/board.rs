//! The market board on screen: cell geometry and painting.
//!
//! Each market is a cell — a status dot, a printed label, the local time,
//! and AM/PM in twelve-hour mode. Stacked vertically the cells share columns
//! so the times align on their colons whatever the labels are, with a
//! hairline between rows. Laid out horizontally they run left to right as
//! modules of a strip, each with its label printed above its digits and a
//! hairline between modules, the way a multi-zone hardware clock is built.
//! The geometry is a sum of cached measurements: labels come from the label
//! cache, times from the row glyph cells, so a frame here measures nothing.
//!
//! Cells are sized for the widest value they can hold, not the one showing
//! now, so the window never resizes as the clocks tick: the time column fits
//! two-digit hours, and the caption line is reserved for the longest
//! countdown any market on the board can produce.

use std::sync::Arc;

use eframe::egui::{Color32, Galley, Painter, Pos2, Stroke, Vec2, pos2, vec2};
use serde::{Deserialize, Serialize};

use crate::app::serde_by_id;
use crate::layout::{DerivedLayout, Metrics};
use crate::market::{self, Labels, Row, Status};
use crate::text::{paint_galley, paint_readout};
use crate::theme::Theme;

/// How the cells are arranged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Layout {
    /// One market per line, label beside the time, times in a shared
    /// right-aligned column.
    #[default]
    Vertical,
    /// Every market on one line, label printed above its time.
    Horizontal,
}

impl Layout {
    pub const ALL: [Self; 2] = [Self::Vertical, Self::Horizontal];

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|l| l.id().eq_ignore_ascii_case(id))
    }

    pub fn id(self) -> &'static str {
        match self {
            Self::Vertical => "vertical",
            Self::Horizontal => "horizontal",
        }
    }

    pub fn toggled(self) -> Self {
        match self {
            Self::Vertical => Self::Horizontal,
            Self::Horizontal => Self::Vertical,
        }
    }
}
serde_by_id!(Layout, "board layout");

/// One market's place on the board, relative to the board's top-left corner.
/// `label_y` and `time_y` are the centre lines of the label and the time;
/// they coincide when the label sits beside the time.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Cell {
    pub dot_x: f32,
    pub label_x: f32,
    pub label_y: f32,
    /// Times are right-aligned here so the colons line up.
    pub time_right: f32,
    pub time_y: f32,
    pub period_x: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Geometry {
    /// The cells only; the caption line below is the app's.
    pub size: Vec2,
    pub cells: Vec<Cell>,
    /// Hairlines between cells, as segments relative to the board's corner.
    pub dividers: Vec<(Pos2, Pos2)>,
    /// The widest caption this market set can produce, so the window can be
    /// sized once instead of following the countdown minute by minute.
    pub caption_min_width: f32,
}

/// `lay` lays out a label at the caption size, letter-spaced like every
/// caption; it is only called for text the label cache has not seen.
pub fn measure(
    layout: &mut DerivedLayout,
    rows: &[Row],
    arrangement: Layout,
    labels: Labels,
    lay: &dyn Fn(&str) -> Arc<Galley>,
) -> Geometry {
    let m = *layout.metrics();
    let mut label = |text: &str| layout.label_galley(text, lay).size().x;

    let label_widths: Vec<f32> = rows.iter().map(|r| label(r.label)).collect();
    let period_width = if rows.iter().any(|r| r.period.is_some()) { label("AM").max(label("PM")) } else { 0.0 };

    // The widest digit of the caption face, for the worst-case countdown.
    let digit = ('0'..='9').fold(('0', 0.0), |best, d| {
        let w = label(&d.to_string());
        if w > best.1 { (d, w) } else { best }
    });
    let markets: Vec<_> = rows.iter().map(|r| r.market).collect();
    let caption_min_width =
        market::widest_captions(&markets, labels, digit.0).iter().map(|c| label(c)).fold(0.0, f32::max);

    // Two-digit hours, with or without seconds, whatever the rows show now.
    let with_seconds = rows.first().is_some_and(|r| r.time.matches(':').count() == 2);
    let time_width = layout.row_glyphs().width_of(if with_seconds { "00:00:00" } else { "00:00" });
    // The time plus its AM/PM, when there is one.
    let period_span = if period_width > 0.0 { m.board_gap * 0.6 + period_width } else { 0.0 };
    let dot_span = m.dot_radius * 2.0 + m.board_gap;

    let (cells, size, dividers): (Vec<Cell>, Vec2, Vec<(Pos2, Pos2)>) = match arrangement {
        Layout::Vertical => {
            let label_width = label_widths.iter().copied().fold(0.0, f32::max);
            let time_right = dot_span + label_width + m.board_gap + time_width;
            let width = if rows.is_empty() { 0.0 } else { time_right + period_span };
            let cells = (0..rows.len())
                .map(|i| {
                    let mid = i as f32 * m.row_pitch + m.row_pitch / 2.0;
                    Cell { dot_x: 0.0, label_x: dot_span, label_y: mid, time_right, time_y: mid, period_x: time_right + m.board_gap * 0.6 }
                })
                .collect();
            let dividers = (1..rows.len()).map(|i| {
                let y = i as f32 * m.row_pitch;
                (pos2(0.0, y), pos2(width, y))
            });
            (cells, vec2(width, rows.len() as f32 * m.row_pitch), dividers.collect())
        }
        Layout::Horizontal => {
            // A module: the label line over the time line, as wide as the
            // wider of the two, and a gap with a hairline between modules.
            let label_line = m.caption_height;
            let height = if rows.is_empty() { 0.0 } else { label_line + m.row_pitch };
            let module_gap = m.board_gap * 2.0;
            let mut x = 0.0;
            let mut dividers = Vec::new();
            let cells = label_widths
                .iter()
                .enumerate()
                .map(|(i, &label_width)| {
                    if i > 0 {
                        let at = x - module_gap / 2.0;
                        dividers.push((pos2(at, 0.0), pos2(at, height)));
                    }
                    let width = (dot_span + label_width).max(time_width + period_span);
                    let time_right = x + width - period_span;
                    let cell = Cell {
                        dot_x: x,
                        label_x: x + dot_span,
                        label_y: label_line / 2.0,
                        time_right,
                        time_y: label_line + m.row_pitch / 2.0,
                        period_x: time_right + m.board_gap * 0.6,
                    };
                    x += width + module_gap;
                    cell
                })
                .collect();
            let width = if rows.is_empty() { 0.0 } else { x - module_gap };
            (cells, vec2(width, height), dividers)
        }
    };
    Geometry { size, cells, dividers, caption_min_width }
}

/// How the board is dressed this frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Style {
    /// The main readout's halo width; the rows scale it down with their
    /// smaller digits.
    pub halo: Option<f32>,
    /// Fade the times of closed markets. Only done over a backdrop: without
    /// one the halo has just bought the contrast that dimming would spend,
    /// and the hollow dot still tells the state.
    pub dim_closed: bool,
    /// Hairlines are translucent, so they are left out under a chroma key.
    pub dressing: bool,
    /// The digital face: the colour of the unlit figure eight under every
    /// digit, or `None` for no ghosting. Decided once by the app so the
    /// board and the main line can never disagree.
    pub ghost: Option<Color32>,
}

/// Paints the cells with the board's top-left corner at `origin`. Labels —
/// the city or code and AM/PM — are printed white like the legends on a
/// hardware clock and never dim; only the LEDs carry the palette: the time,
/// the status dot, and the hollow ring of a closed market, which dims with
/// its time.
#[allow(clippy::too_many_arguments, reason = "a painting call site, not an API")]
pub fn paint(
    painter: &Painter,
    origin: Pos2,
    rows: &[Row],
    geo: &Geometry,
    layout: &mut DerivedLayout,
    theme: &Theme,
    style: Style,
    lay: &dyn Fn(&str) -> Arc<Galley>,
) {
    let m: Metrics = *layout.metrics();
    let row_halo = style.halo.map(|h| (h * m.row_font / m.font).max(1.0));
    let label_halo = row_halo.map(|_| 1.0);
    let shade = Color32::from_black_alpha(theme.color.halo_stroke);
    let row_height = layout.row_glyphs().height;

    if style.dressing {
        let stroke = Stroke::new(1.0, theme.color.separator);
        for (a, b) in &geo.dividers {
            painter.line_segment([origin + a.to_vec2(), origin + b.to_vec2()], stroke);
        }
    }

    for (row, cell) in rows.iter().zip(&geo.cells) {
        let time_color = match (row.status, style.dim_closed) {
            (Status::Closed, true) => theme.dim_caption(theme.color.text),
            _ => theme.color.text,
        };
        let label_color = theme.color.label;

        // Status dot beside the label: filled while trading, amber through
        // a midday break, a ring when closed. Over a transparent desktop a
        // dark rim keeps it visible against whatever is behind.
        let centre = pos2(origin.x + cell.dot_x + m.dot_radius, origin.y + cell.label_y);
        let r = m.dot_radius;
        match row.status {
            Status::Open | Status::Break => {
                if let Some(h) = row_halo {
                    painter.circle_filled(centre, r + h, shade);
                }
                let fill = if row.status == Status::Open { theme.color.open } else { theme.color.alert };
                painter.circle_filled(centre, r, fill);
            }
            Status::Closed => {
                if let Some(h) = row_halo {
                    painter.circle_stroke(centre, r, Stroke::new(m.ring_width + 2.0 * h, shade));
                }
                painter.circle_stroke(centre, r, Stroke::new(m.ring_width, time_color));
            }
        }

        let label = layout.label_galley(row.label, lay);
        let pos = pos2(origin.x + cell.label_x, origin.y + cell.label_y - label.size().y / 2.0);
        paint_galley(painter, pos, label, label_color, label_halo, theme);

        let time_width = layout.row_glyphs().width_of(&row.time);
        let pos = pos2(origin.x + cell.time_right - time_width, origin.y + cell.time_y - row_height / 2.0);
        paint_readout(painter, pos, &row.time, layout.row_glyphs(), time_color, row_halo, style.ghost, theme);

        if let Some(period) = row.period {
            let galley = layout.label_galley(period, lay);
            let pos = pos2(origin.x + cell.period_x, origin.y + cell.time_y - galley.size().y / 2.0);
            paint_galley(painter, pos, galley, label_color, label_halo, theme);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::Size;
    use crate::clock::ClockFormat;
    use crate::layout::{Font, Glyphs, LayoutKey};
    use crate::market::{Board, BoardStyle, Market};
    use chrono::NaiveDate;
    use eframe::egui::{self, FontId};
    use std::collections::HashMap;

    fn board(markets: &[Market], format: ClockFormat, show_seconds: bool, labels: Labels) -> Board {
        let t = NaiveDate::from_ymd_opt(2026, 9, 18).unwrap().and_hms_opt(14, 0, 0).unwrap();
        market::board(t, markets, BoardStyle { format, show_seconds, labels })
    }

    fn glyphs(font: f32) -> Glyphs {
        Glyphs { digit_width: font * 0.6, height: font * 1.2, widths: HashMap::from([(':', font * 0.3)]), ..Glyphs::default() }
    }

    /// Runs `f` inside a headless egui pass so labels can be laid out.
    fn with_layout(f: impl FnOnce(&mut DerivedLayout, &dyn Fn(&str) -> Arc<Galley>)) {
        let theme = Theme::default();
        let mut layout = DerivedLayout::new(&theme);
        layout.ensure(LayoutKey::new(Size::Medium, 1.0, Font::Sans), &theme, glyphs);
        let ctx = egui::Context::default();
        let mut f = Some(f);
        let mut out = ctx.run_ui(Default::default(), |ui| {
            let ctx = ui.ctx();
            let lay = |text: &str| {
                ctx.fonts_mut(|fonts| fonts.layout_no_wrap(text.to_owned(), FontId::proportional(11.0), Color32::WHITE))
            };
            if let Some(f) = f.take() {
                f(&mut layout, &lay);
            }
        });
        // Nothing consumes the font atlas here.
        out.textures_delta.clear();
    }

    #[test]
    fn columns_are_sized_for_the_widest_value_not_the_current_one() {
        with_layout(|layout, lay| {
            let m = *layout.metrics();
            let b = board(&[Market::NewYork, Market::Tokyo], ClockFormat::H12, false, Labels::City);
            assert_eq!((b.rows[0].time.as_str(), b.rows[1].time.as_str()), ("10:00", "11:00"));
            let geo = measure(layout, &b.rows, Layout::Vertical, Labels::City, lay);
            // Five cells at row size: two-digit hours even if every row showed one.
            let five = layout.row_glyphs().width_of("00:00");
            let label_width = layout.label_galley("NEW YORK", lay).size().x;
            let cell = geo.cells[0];
            assert!((cell.time_right - (cell.label_x + label_width + m.board_gap + five)).abs() < 1e-3);
            assert_eq!(geo.cells[1].time_right, cell.time_right, "one column for every row");
            assert!(cell.period_x > cell.time_right, "AM/PM sits after the time");
            assert!(geo.size.x > cell.period_x);
            assert_eq!(geo.size.y, 2.0 * m.row_pitch);
            assert_eq!(cell.label_y, cell.time_y, "label beside the time");
            assert_eq!(geo.cells[1].time_y, m.row_pitch * 1.5);
        });
    }

    #[test]
    fn seconds_widen_the_time_column_and_a_24h_board_has_no_period_column() {
        with_layout(|layout, lay| {
            let rows = |s| board(&[Market::London], ClockFormat::H24, s, Labels::City).rows;
            let short = measure(layout, &rows(false), Layout::Vertical, Labels::City, lay);
            let long = measure(layout, &rows(true), Layout::Vertical, Labels::City, lay);
            let eight = layout.row_glyphs().width_of("00:00:00");
            let five = layout.row_glyphs().width_of("00:00");
            assert!((long.cells[0].time_right - short.cells[0].time_right - (eight - five)).abs() < 1e-3);
            assert_eq!(short.size.x, short.cells[0].time_right, "no AM/PM column in 24-hour mode");
        });
    }

    /// The caption line must be able to hold the longest countdown for the
    /// widest city, or the window would resize every minute.
    #[test]
    fn the_caption_reservation_covers_every_countdown_shape() {
        with_layout(|layout, lay| {
            let markets = [Market::NewYork, Market::HongKong];
            let rows = board(&markets, ClockFormat::H24, false, Labels::City).rows;
            let geo = measure(layout, &rows, Layout::Vertical, Labels::City, lay);
            for text in ["HONG KONG CLOSES IN 23H 59M", "HONG KONG OPENS IN 2D 16H", "NEW YORK OPENS IN 1M"] {
                let w = lay(text).size().x;
                assert!(w <= geo.caption_min_width + 1e-3, "{text}: {w} > {}", geo.caption_min_width);
            }
            // With codes the reservation follows the shorter names.
            let rows = board(&markets, ClockFormat::H24, false, Labels::Code).rows;
            let codes = measure(layout, &rows, Layout::Vertical, Labels::Code, lay);
            assert!(codes.caption_min_width < geo.caption_min_width);
            assert!(lay("HKEX CLOSES IN 23H 59M").size().x <= codes.caption_min_width + 1e-3);
        });
    }

    /// A strip is a row of modules: label printed above the digits, each
    /// module as wide as the wider of the two, hairlines in the gaps.
    #[test]
    fn a_strip_stacks_the_label_over_the_time_in_modules() {
        with_layout(|layout, lay| {
            let m = *layout.metrics();
            let rows = board(&[Market::NewYork, Market::Oslo, Market::Tokyo], ClockFormat::H24, false, Labels::City).rows;
            let geo = measure(layout, &rows, Layout::Horizontal, Labels::City, lay);
            assert_eq!(geo.size.y, m.caption_height + m.row_pitch);
            for cell in &geo.cells {
                assert!(cell.label_y < cell.time_y, "label above the time: {cell:?}");
                assert_eq!(cell.label_y, m.caption_height / 2.0);
            }
            for pair in geo.cells.windows(2) {
                assert!(pair[1].dot_x > pair[0].time_right, "modules must not overlap: {pair:?}");
            }
            let five = layout.row_glyphs().width_of("00:00");
            let dot_span = m.dot_radius * 2.0 + m.board_gap;
            let new_york = layout.label_galley("NEW YORK", lay).size().x;
            let oslo = layout.label_galley("OSLO", lay).size().x;
            let module = |c: &Cell| c.time_right - c.dot_x;
            assert!((module(&geo.cells[0]) - (dot_span + new_york).max(five)).abs() < 1e-3, "widest of label and time");
            assert!((module(&geo.cells[1]) - (dot_span + oslo).max(five)).abs() < 1e-3);
            assert!((geo.size.x - geo.cells[2].time_right).abs() < 1e-3, "the strip ends with the last time");

            // One hairline per gap, spanning the module height, centred in the gap.
            assert_eq!(geo.dividers.len(), 2);
            for (i, (a, b)) in geo.dividers.iter().enumerate() {
                assert_eq!(a.x, b.x, "a vertical hairline");
                assert_eq!((a.y, b.y), (0.0, geo.size.y));
                let (left, right) = (geo.cells[i].time_right, geo.cells[i + 1].dot_x);
                assert!(a.x > left && a.x < right, "hairline {i} at {} not between {left} and {right}", a.x);
            }

            let stacked = measure(layout, &rows, Layout::Vertical, Labels::City, lay);
            assert!(geo.size.x > stacked.size.x && geo.size.y < stacked.size.y);
        });
    }

    #[test]
    fn a_stack_has_a_hairline_between_every_pair_of_rows() {
        with_layout(|layout, lay| {
            let m = *layout.metrics();
            let rows = board(&Market::DEFAULT, ClockFormat::H24, false, Labels::City).rows;
            let geo = measure(layout, &rows, Layout::Vertical, Labels::City, lay);
            assert_eq!(geo.dividers.len(), rows.len() - 1);
            for (i, (a, b)) in geo.dividers.iter().enumerate() {
                assert_eq!(a.y, b.y, "a horizontal hairline");
                assert_eq!(a.y, (i + 1) as f32 * m.row_pitch);
                assert_eq!((a.x, b.x), (0.0, geo.size.x));
            }
        });
    }

    #[test]
    fn an_empty_board_takes_no_room_either_way() {
        with_layout(|layout, lay| {
            for arrangement in Layout::ALL {
                let geo = measure(layout, &[], arrangement, Labels::City, lay);
                assert_eq!(geo.size, Vec2::ZERO, "{arrangement:?}");
                assert_eq!(geo.caption_min_width, 0.0);
                assert!(geo.cells.is_empty() && geo.dividers.is_empty());
            }
        });
    }

    #[test]
    fn layout_ids_round_trip_and_toggle() {
        for l in Layout::ALL {
            assert_eq!(Layout::from_id(l.id()), Some(l));
            assert_eq!(l.toggled().toggled(), l);
        }
        assert_eq!(Layout::from_id("diagonal"), None);
    }
}
