//! Drawing that is not a readout: the seconds ring, the welcome card, the
//! hover buttons, and the width a one-line readout is held at.

use std::sync::Arc;

use eframe::egui::{self, Color32, FontId, Pos2, Rect, Sense, Stroke, Vec2, pos2, vec2};

use crate::clock::ClockFormat;
use crate::layout::{LayoutKey, Metrics};
use crate::ring;
use crate::text::{display_family, label_family, spaced};
use crate::theme::Theme;
use crate::tray::Command;
use crate::welcome;

use super::Mode;

/// The studio seconds ring: sixty discrete LEDs along the window's outline,
/// the first `lit` of them on, clockwise from the top.
///
/// Every LED sits in a dark socket, so the unlit ones read as a faint dark
/// glow with a whisper of the LED colour, the way an off diode does. The one
/// that lit last blooms a little brighter than the rest, and the four
/// quarter positions carry marker LEDs in their own colour whether lit or
/// not. With `dressing` off (chroma key) only the lit LEDs are drawn, and
/// without the bloom, which is translucent.
pub(super) fn paint_ring(painter: &egui::Painter, rect: Rect, m: &Metrics, theme: &Theme, lit: usize, halo: Option<f32>, dressing: bool) {
    let track = rect.shrink(m.ring_band / 2.0);
    let corner = (m.corner - m.ring_band / 2.0).max(0.0);
    let c = &theme.color;
    let r = m.led_radius;
    let (on, marker) = (c.secondary, c.ring_marker);
    let socket = Color32::from_black_alpha(c.ring_socket);
    let shade = Color32::from_black_alpha(c.halo_stroke);
    for (i, centre) in ring::dots(track, corner).into_iter().enumerate() {
        let is_marker = ring::is_marker(i);
        let colour = if is_marker { marker } else { on };
        let radius = if is_marker { r * 1.25 } else { r };
        if dressing {
            painter.circle_filled(centre, radius * 1.6, socket);
        } else if let Some(h) = halo {
            painter.circle_filled(centre, radius + h, shade);
        }
        if i < lit || is_marker {
            // The bloom stays within the socket, so it never smears onto the desktop.
            if dressing && i + 1 == lit {
                painter.circle_filled(centre, radius * 1.6, colour.gamma_multiply(f32::from(c.ring_bloom) / 255.0));
            }
            painter.circle_filled(centre, radius, colour);
        } else if dressing {
            painter.circle_filled(centre, radius, colour.gamma_multiply(f32::from(c.ring_unlit) / 255.0));
        }
    }
}

/// The welcome card, laid out: a title, then one row per tip with the key or
/// place in bold and what it does beside it. A fixed, readable type size
/// whatever size the clock is set to, since this is prose, not a readout.
pub(super) struct Card {
    pub(super) title: Arc<egui::Galley>,
    pub(super) rows: Vec<(Arc<egui::Galley>, Arc<egui::Galley>)>,
    pub(super) key_width: f32,
    pub(super) size: Vec2,
}

impl Card {
    const TYPE: f32 = 13.0;
    const ROW: f32 = 21.0;
    const TITLE_ROW: f32 = 30.0;
    const GUTTER: f32 = 14.0;

    pub(super) fn lay_out(painter: &egui::Painter, theme: &Theme) -> Self {
        let c = &theme.color;
        let title = painter.layout_no_wrap(spaced(welcome::TITLE), FontId::new(Self::TYPE - 1.0, display_family()), c.secondary);
        let rows: Vec<_> = welcome::ROWS
            .iter()
            .map(|(key, what)| {
                (
                    painter.layout_no_wrap((*key).to_owned(), FontId::new(Self::TYPE, label_family()), c.label),
                    painter.layout_no_wrap((*what).to_owned(), FontId::new(Self::TYPE, display_family()), c.text.gamma_multiply(0.85)),
                )
            })
            .collect();
        let key_width = rows.iter().map(|(key, _)| key.size().x).fold(0.0, f32::max);
        let text_width = rows.iter().map(|(_, what)| what.size().x).fold(0.0, f32::max);
        let width = (key_width + Self::GUTTER + text_width).max(title.size().x);
        let size = vec2(width, Self::TITLE_ROW + Self::ROW * rows.len() as f32);
        Self { title, rows, key_width, size }
    }

    pub(super) fn paint(&self, painter: &egui::Painter, origin: Pos2) {
        painter.galley(origin, Arc::clone(&self.title), Color32::PLACEHOLDER);
        for (i, (key, what)) in self.rows.iter().enumerate() {
            let y = origin.y + Self::TITLE_ROW + Self::ROW * i as f32;
            // Keys right-aligned against the gutter, so the explanations line up.
            painter.galley(pos2(origin.x + self.key_width - key.size().x, y), Arc::clone(key), Color32::PLACEHOLDER);
            painter.galley(pos2(origin.x + self.key_width + Self::GUTTER, y), Arc::clone(what), Color32::PLACEHOLDER);
        }
    }
}

/// Where the two hover buttons sit within the caption line.
pub(super) fn control_rects(area: Rect, metrics: &Metrics) -> [(Rect, Command); 2] {
    let (size, gap) = (metrics.control, metrics.control_gap);
    let center = area.center();
    [
        (Rect::from_center_size(center - vec2((size + gap) / 2.0, 0.0), Vec2::splat(size)), Command::StartPause),
        (Rect::from_center_size(center + vec2((size + gap) / 2.0, 0.0), Vec2::splat(size)), Command::Reset),
    ]
}

/// Start/pause and reset buttons, drawn as shapes so no icon font is needed.
pub(super) fn controls(
    ui: &mut egui::Ui,
    area: Rect,
    metrics: &Metrics,
    theme: &Theme,
    start_label: &str,
) -> Option<Command> {
    let size = metrics.control;
    let mut clicked = None;
    for (rect, cmd) in control_rects(area, metrics) {
        // A stable id per button: `format!` here would allocate every frame.
        let id = ui.id().with(match cmd {
            Command::StartPause => "start_pause",
            _ => "reset",
        });
        let response = ui.interact(rect, id, Sense::click());
        let alpha = if response.hovered() {
            theme.color.control_glyph_hover
        } else {
            theme.color.control_glyph
        };
        let color = Color32::from_white_alpha(alpha);
        let painter = ui.painter();
        let disc = size * theme.ratio.control_disc;
        // Dark base keeps the glyphs legible over busy windows behind the overlay.
        painter.circle_filled(rect.center(), disc, theme.color.control_base);
        if response.hovered() {
            painter.circle_filled(rect.center(), disc, theme.color.control_hover);
        }
        let r = rect.shrink(size * theme.ratio.control_inset);
        match cmd {
            Command::StartPause if start_label == "Pause" => {
                let bar = vec2(r.width() * 0.3, r.height());
                painter.rect_filled(Rect::from_min_size(r.left_top(), bar), 1.0, color);
                painter.rect_filled(Rect::from_min_size(r.right_top() - vec2(bar.x, 0.0), bar), 1.0, color);
            }
            Command::StartPause => {
                let pts = vec![r.left_top() + vec2(r.width() * 0.1, 0.0), r.right_center(), r.left_bottom() + vec2(r.width() * 0.1, 0.0)];
                painter.add(egui::Shape::convex_polygon(pts, color, Stroke::NONE));
            }
            _ => {
                painter.rect_filled(r.shrink(r.width() * 0.08), 1.5, color);
            }
        }
        if response.clicked() {
            clicked = Some(cmd);
        }
    }
    clicked
}

/// What decides the width a one-line readout needs: while none of it
/// changes, neither may the window.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct LineShape {
    pub(super) layout: LayoutKey,
    pub(super) mode: Mode,
    pub(super) clock_format: ClockFormat,
    pub(super) show_seconds: bool,
    pub(super) show_date: bool,
    pub(super) timer_minutes: u64,
}

/// The widest a readout has been since its shape last changed. The text on
/// one line comes and goes in width (a timer's "TIMER · 25 MIN" becomes
/// "TIMER" when it starts, a stopwatch says "PAUSED", the date is longer on
/// some days), and a window that followed it would shrink and grow under
/// the user, with the digits jumping sideways each time.
pub(super) struct WidthFloor<K> {
    key: Option<K>,
    width: f32,
}

impl<K> Default for WidthFloor<K> {
    fn default() -> Self {
        Self { key: None, width: 0.0 }
    }
}

impl<K: PartialEq> WidthFloor<K> {
    pub(super) fn hold(&mut self, key: K, width: f32) -> f32 {
        if self.key.as_ref() != Some(&key) {
            (self.key, self.width) = (Some(key), 0.0);
        }
        self.width = self.width.max(width);
        self.width
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_keeps_its_widest_width_until_its_shape_changes() {
        let mut floor = WidthFloor::default();
        assert_eq!(floor.hold("timer 25", 92.0), 92.0, "TIMER · 25 MIN");
        assert_eq!(floor.hold("timer 25", 66.0), 92.0, "started: the caption is shorter, the window is not");
        assert_eq!(floor.hold("timer 25", 100.0), 100.0);
        assert_eq!(floor.hold("timer 30", 70.0), 70.0, "a new duration is measured afresh");
    }
}
