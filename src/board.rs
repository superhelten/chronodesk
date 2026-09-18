//! The market board on screen: column geometry and painting.
//!
//! Each market is a cell — a status dot, the label, the local time, and
//! AM/PM in twelve-hour mode. Stacked vertically the cells share columns so
//! the times align on their colons whatever the labels are; laid out
//! horizontally they run left to right as one line, each cell as wide as
//! its own label. The geometry is a sum of cached measurements: labels come
//! from the label cache, times from the row glyph cells, so a frame here
//! measures nothing.
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
    /// One market per line, times in a shared right-aligned column.
    #[default]
    Vertical,
    /// Every market on one line, as a ticker strip.
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
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Cell {
    pub top: f32,
    pub dot_x: f32,
    pub label_x: f32,
    /// Times are right-aligned here so the colons line up.
    pub time_right: f32,
    pub period_x: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Geometry {
    /// The cells only; the caption line below is the app's.
    pub size: Vec2,
    pub cells: Vec<Cell>,
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

    let dot_span = m.dot_radius * 2.0 + m.board_gap;
    let cell_end = |time_right: f32| if period_width > 0.0 { time_right + m.board_gap * 0.6 + period_width } else { time_right };
    let cells: Vec<Cell> = match arrangement {
        Layout::Vertical => {
            let label_width = label_widths.iter().copied().fold(0.0, f32::max);
            let time_right = dot_span + label_width + m.board_gap + time_width;
            (0..rows.len())
                .map(|i| Cell {
                    top: i as f32 * m.row_pitch,
                    dot_x: 0.0,
                    label_x: dot_span,
                    time_right,
                    period_x: time_right + m.board_gap * 0.6,
                })
                .collect()
        }
        Layout::Horizontal => {
            let mut x = 0.0;
            label_widths
                .iter()
                .map(|&label_width| {
                    let time_right = x + dot_span + label_width + m.board_gap + time_width;
                    let cell = Cell { top: 0.0, dot_x: x, label_x: x + dot_span, time_right, period_x: time_right + m.board_gap * 0.6 };
                    x = cell_end(time_right) + m.board_gap * 2.0;
                    cell
                })
                .collect()
        }
    };
    let width = cells.iter().map(|c| cell_end(c.time_right)).fold(0.0, f32::max);
    let height = match arrangement {
        Layout::Vertical => rows.len() as f32 * m.row_pitch,
        Layout::Horizontal if rows.is_empty() => 0.0,
        Layout::Horizontal => m.row_pitch,
    };
    Geometry { size: vec2(width, height), cells, caption_min_width }
}

/// Paints the cells with the board's top-left corner at `origin`.
///
/// `halo` is the main readout's halo width; the rows scale it down with
/// their smaller digits. `dim_closed` fades closed markets, which is only
/// done over a backdrop: without one the halo has just bought the contrast
/// that dimming would spend, and the hollow dot still tells the state.
#[allow(clippy::too_many_arguments, reason = "a painting call site, not an API")]
pub fn paint(
    painter: &Painter,
    origin: Pos2,
    rows: &[Row],
    geo: &Geometry,
    layout: &mut DerivedLayout,
    theme: &Theme,
    halo: Option<f32>,
    dim_closed: bool,
    lay: &dyn Fn(&str) -> Arc<Galley>,
) {
    let m: Metrics = *layout.metrics();
    let row_halo = halo.map(|h| (h * m.row_font / m.font).max(1.0));
    let shade = Color32::from_black_alpha(theme.color.halo_stroke);
    let row_height = layout.row_glyphs().height;

    for (row, cell) in rows.iter().zip(&geo.cells) {
        let mid = origin.y + cell.top + m.row_pitch / 2.0;
        let color = match (row.status, dim_closed) {
            (Status::Closed, true) => theme.dim_caption(theme.color.text),
            _ => theme.color.text,
        };

        // Status dot: filled while trading, amber through a midday break,
        // a ring when closed. Over a transparent desktop a dark rim keeps
        // it visible against whatever is behind.
        let centre = pos2(origin.x + cell.dot_x + m.dot_radius, mid);
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
                painter.circle_stroke(centre, r, Stroke::new(m.ring_width, color));
            }
        }

        let label = layout.label_galley(row.label, lay);
        let pos = pos2(origin.x + cell.label_x, mid - label.size().y / 2.0);
        paint_galley(painter, pos, label, color, row_halo.map(|_| 1.0), theme);

        let time_width = layout.row_glyphs().width_of(&row.time);
        let pos = pos2(origin.x + cell.time_right - time_width, mid - row_height / 2.0);
        paint_readout(painter, pos, &row.time, layout.row_glyphs(), color, row_halo, theme);

        if let Some(period) = row.period {
            let galley = layout.label_galley(period, lay);
            let pos = pos2(origin.x + cell.period_x, mid - galley.size().y / 2.0);
            paint_galley(painter, pos, galley, color, row_halo.map(|_| 1.0), theme);
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
            assert_eq!(geo.cells[1].top, m.row_pitch);
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

    /// Horizontally the cells run left to right on one line, each as wide
    /// as its own label, and the whole board is one row tall.
    #[test]
    fn a_horizontal_board_is_one_line_of_cells_in_order() {
        with_layout(|layout, lay| {
            let m = *layout.metrics();
            let rows = board(&[Market::NewYork, Market::Oslo, Market::Tokyo], ClockFormat::H24, false, Labels::City).rows;
            let geo = measure(layout, &rows, Layout::Horizontal, Labels::City, lay);
            assert_eq!(geo.size.y, m.row_pitch);
            assert!(geo.cells.iter().all(|c| c.top == 0.0));
            for pair in geo.cells.windows(2) {
                assert!(pair[1].dot_x > pair[0].time_right, "cells must not overlap: {pair:?}");
            }
            let five = layout.row_glyphs().width_of("00:00");
            let oslo = layout.label_galley("OSLO", lay).size().x;
            let new_york = layout.label_galley("NEW YORK", lay).size().x;
            let cell_width = |c: &Cell| c.time_right - c.dot_x;
            assert!((cell_width(&geo.cells[1]) - cell_width(&geo.cells[0]) - (oslo - new_york)).abs() < 1e-3);
            assert!((cell_width(&geo.cells[1]) - (m.dot_radius * 2.0 + m.board_gap + oslo + m.board_gap + five)).abs() < 1e-3);
            assert_eq!(geo.size.x, geo.cells[2].time_right, "the line ends with the last time");

            let stacked = measure(layout, &rows, Layout::Vertical, Labels::City, lay);
            assert!(geo.size.x > stacked.size.x && geo.size.y < stacked.size.y);
        });
    }

    #[test]
    fn an_empty_board_takes_no_room_either_way() {
        with_layout(|layout, lay| {
            for arrangement in Layout::ALL {
                let geo = measure(layout, &[], arrangement, Labels::City, lay);
                assert_eq!(geo.size, Vec2::ZERO, "{arrangement:?}");
                assert_eq!(geo.caption_min_width, 0.0);
                assert!(geo.cells.is_empty());
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
