//! The studio seconds ring: sixty LEDs around the readout, lit clockwise
//! from the top as the seconds pass, the way broadcast clocks show them.
//!
//! The overlay is a wide rounded rectangle rather than a disc, so the ring
//! follows that outline: a track just inside the edge, with the dots spaced
//! evenly along its perimeter, straight runs and corner arcs alike. The
//! geometry is a pure function of the rectangle, so it is computed once per
//! frame from cached metrics and tested without a painter.

use std::f32::consts::PI;

use eframe::egui::{Pos2, Rect, pos2};

/// LEDs in a full turn.
pub const DOTS: usize = 60;

/// Centres of the `DOTS` LEDs on the rounded rectangle `track` (corner
/// radius `radius`), the first at top centre and the rest clockwise.
pub fn dots(track: Rect, radius: f32) -> Vec<Pos2> {
    let radius = radius.min(track.width() / 2.0).min(track.height() / 2.0).max(0.0);
    let perimeter = 2.0 * (track.width() + track.height()) - 8.0 * radius + 2.0 * PI * radius;
    (0..DOTS).map(|i| point_at(track, radius, perimeter * i as f32 / DOTS as f32)).collect()
}

/// The point `t` along the track, measured clockwise from top centre.
fn point_at(track: Rect, r: f32, t: f32) -> Pos2 {
    let (w, h) = (track.width(), track.height());
    let (straight_x, straight_y) = (w - 2.0 * r, h - 2.0 * r);
    let arc = PI * r / 2.0;
    let (left, top, right, bottom) = (track.left(), track.top(), track.right(), track.bottom());

    // Legs in order: half the top edge, a corner, the right edge, a corner,
    // the bottom edge, a corner, the left edge, a corner, half the top edge.
    let mut t = t;
    let half_top = straight_x / 2.0;
    if t < half_top {
        return pos2(left + r + half_top + t, top);
    }
    t -= half_top;
    let corners = [
        (pos2(right - r, top + r), -PI / 2.0),
        (pos2(right - r, bottom - r), 0.0),
        (pos2(left + r, bottom - r), PI / 2.0),
        (pos2(left + r, top + r), PI),
    ];
    let edges: [(f32, &dyn Fn(f32) -> Pos2); 4] = [
        (straight_y, &|d| pos2(right, top + r + d)),
        (straight_x, &|d| pos2(right - r - d, bottom)),
        (straight_y, &|d| pos2(left, bottom - r - d)),
        (straight_x / 2.0, &|d| pos2(left + r + d, top)),
    ];
    for ((centre, start), (length, along)) in corners.into_iter().zip(edges) {
        if t < arc {
            let angle = start + (t / arc) * PI / 2.0;
            return pos2(centre.x + r * angle.cos(), centre.y + r * angle.sin());
        }
        t -= arc;
        if t < length {
            return along(t);
        }
        t -= length;
    }
    // Rounding at the very end of the last leg: back at the top centre.
    pos2(left + r + half_top, top)
}

/// How many LEDs are lit at `second` of the minute on a clock or a running
/// stopwatch: the top one at :00, the whole ring at :59.
pub fn lit(second: u32) -> usize {
    (second as usize % DOTS) + 1
}

/// How many LEDs are lit with `seconds_left` in the current minute of a
/// countdown: the ring drains with the digits, fifty-nine at :59, one at
/// :01, empty on the minute and once the countdown is over.
pub fn lit_remaining(seconds_left: u64) -> usize {
    (seconds_left % DOTS as u64) as usize
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::vec2;

    fn track(w: f32, h: f32) -> Rect {
        Rect::from_min_size(pos2(10.0, 20.0), vec2(w, h))
    }

    /// Distance from `p` to the outline of a rounded rectangle.
    fn distance_to_outline(track: Rect, r: f32, p: Pos2) -> f32 {
        let inner = track.shrink(r);
        let cx = p.x.clamp(inner.left(), inner.right());
        let cy = p.y.clamp(inner.top(), inner.bottom());
        ((p.x - cx).powi(2) + (p.y - cy).powi(2)).sqrt() - r
    }

    #[test]
    fn sixty_dots_lie_on_the_outline_and_start_at_top_centre() {
        let t = track(200.0, 80.0);
        let d = dots(t, 12.0);
        assert_eq!(d.len(), DOTS);
        assert_eq!(d[0], pos2(t.center().x, t.top()));
        for (i, p) in d.iter().enumerate() {
            assert!(distance_to_outline(t, 12.0, *p).abs() < 1e-3, "dot {i} at {p:?} is off the track");
        }
    }

    #[test]
    fn dots_are_evenly_spaced_along_the_perimeter() {
        let t = track(200.0, 80.0);
        let d = dots(t, 12.0);
        let perimeter = 2.0 * (200.0 + 80.0) - 8.0 * 12.0 + 2.0 * PI * 12.0;
        let step = perimeter / DOTS as f32;
        for pair in d.windows(2) {
            let gap = (pair[1] - pair[0]).length();
            // Chords across a corner are a little shorter than the arc.
            assert!(gap <= step + 1e-3 && gap > step * 0.9, "gap {gap} vs step {step}");
        }
    }

    #[test]
    fn on_a_square_the_quarter_dots_sit_at_the_edge_centres() {
        let t = track(100.0, 100.0);
        let d = dots(t, 10.0);
        let close = |a: Pos2, b: Pos2| (a - b).length() < 1e-3;
        assert!(close(d[15], pos2(t.right(), t.center().y)), "{:?}", d[15]);
        assert!(close(d[30], pos2(t.center().x, t.bottom())), "{:?}", d[30]);
        assert!(close(d[45], pos2(t.left(), t.center().y)), "{:?}", d[45]);
    }

    #[test]
    fn the_ring_runs_clockwise() {
        let d = dots(track(200.0, 80.0), 12.0);
        assert!(d[1].x > d[0].x && (d[1].y - d[0].y).abs() < 1e-3, "first step goes right along the top");
        assert!(d[59].x < d[0].x, "last dot is left of the top centre");
    }

    #[test]
    fn a_radius_larger_than_the_track_is_clamped() {
        let d = dots(track(40.0, 40.0), 100.0);
        assert_eq!(d.len(), DOTS);
        assert!(d.iter().all(|p| p.x.is_finite() && p.y.is_finite()));
    }

    #[test]
    fn the_top_led_is_lit_at_zero_and_all_at_fifty_nine() {
        assert_eq!(lit(0), 1);
        assert_eq!(lit(30), 31);
        assert_eq!(lit(59), DOTS);
        assert_eq!(lit(60), 1, "wraps like a clock");
    }

    #[test]
    fn a_countdown_drains_the_ring_and_ends_empty() {
        assert_eq!(lit_remaining(60), 0, "a whole minute left: nothing into this minute yet");
        assert_eq!(lit_remaining(59), 59);
        assert_eq!(lit_remaining(1), 1);
        assert_eq!(lit_remaining(0), 0, "time's up");
        assert_eq!(lit_remaining(25 * 60), 0, "a fresh timer starts on the minute");
        assert_eq!(lit_remaining(24 * 60 + 59), 59);
    }
}
