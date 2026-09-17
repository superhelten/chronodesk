//! Where the overlay may sit.
//!
//! A saved position is only meaningful against the monitors that happen to be
//! attached right now. Unplug the screen the overlay lived on, change the
//! resolution, or move a laptop between docks, and `app.ron` still names a spot
//! that no longer exists. Windows clamps such a window somewhere on-screen by
//! itself, so nothing is ever truly lost, but "somewhere" can be a corner, or
//! underneath the taskbar.
//!
//! So the position is evaluated explicitly at startup:
//!
//! * nothing visible on any monitor → top-right of the primary monitor;
//! * visible, but hanging over the taskbar or a screen edge → nudged back into
//!   that monitor's work area, as close to where the user left it as possible;
//! * otherwise left alone, wherever the user put it.
//!
//! **Everything here is in physical pixels**, in the virtual-desktop coordinate
//! space the OS reports monitors in. That is the only space in which multiple
//! monitors share one coordinate system at different scaling factors; egui's
//! points are that space divided by the *current* window's scale, which changes
//! the moment the window crosses to a screen with a different scaling
//! percentage. The caller converts at the boundary, and the margin is scaled by
//! the target monitor's factor so it stays the same visible distance at 100%,
//! 150% or 250%.

use eframe::egui::{Pos2, Rect, Vec2, pos2, vec2};

/// Distance from the work-area corner when the overlay has to be re-placed,
/// in points: a scaling-independent visual gap, not a pixel count.
const MARGIN_PT: f32 = 24.0;

/// How much of the overlay must show on some monitor for a saved position to
/// count as usable. Below this there is nothing left to grab with the mouse,
/// so the position is treated as lost rather than merely clipped.
const MIN_VISIBLE_PT: f32 = 32.0;

/// One attached monitor, as placement needs it. All rectangles are physical
/// pixels in virtual-desktop coordinates, so a screen left of or above the
/// primary one has negative coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Monitor {
    /// The whole screen.
    pub bounds: Rect,
    /// The screen minus the taskbar, the system tray and any other appbar.
    /// The OS reports this in pixels, so it is already correct at any scaling.
    pub work: Rect,
    /// OS scaling factor: 1.0 at 100%, 1.5 at 150%.
    pub scale: f32,
    pub primary: bool,
}

/// Why a window had to be moved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    /// Too little of it landed on any monitor — typically a screen that is no
    /// longer attached.
    OffScreen,
    /// On a monitor, but overlapping the taskbar or running past an edge.
    ClippedWorkArea,
}

impl Reason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OffScreen => "off-screen",
            Self::ClippedWorkArea => "clipped-work-area",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Outcome {
    /// The saved position is fine; leave the window where it is.
    Keep,
    /// Physical-pixel position the window should be moved to.
    Move { to: Pos2, reason: Reason },
}

/// Decides where the overlay should sit.
///
/// `saved` is the stored position in physical pixels and `logical_size` the
/// window's size in points, which is scaling-independent: its pixel size
/// depends on whichever monitor it ends up on. `zoom` is egui's zoom factor.
///
/// An empty `monitors` means the monitors could not be read, and then the
/// honest answer is to change nothing and let the OS place the window.
pub fn evaluate(saved: Pos2, logical_size: Vec2, zoom: f32, monitors: &[Monitor]) -> Outcome {
    let Some(primary) = primary(monitors) else {
        return Outcome::Keep;
    };

    // Which screen the overlay is on decides how large it is in pixels, so each
    // candidate is measured with its own scaling factor rather than a shared one.
    let host = monitors
        .iter()
        .filter_map(|m| {
            let visible = window_on(saved, logical_size, zoom, m).intersect(m.bounds);
            let enough = MIN_VISIBLE_PT * m.scale;
            let (w, h) = (visible.width(), visible.height());
            (w >= enough && h >= enough).then_some((m, w * h))
        })
        .max_by(|a, b| a.1.total_cmp(&b.1));

    let Some((host, _)) = host else {
        return Outcome::Move { to: corner_of(primary, logical_size, zoom), reason: Reason::OffScreen };
    };

    let window = window_on(saved, logical_size, zoom, host);
    if host.work.contains_rect(window) {
        return Outcome::Keep;
    }
    Outcome::Move {
        to: clamp_into(host.work, saved, window.size()),
        reason: Reason::ClippedWorkArea,
    }
}

/// One line per monitor, for the instrumentation report and for diagnosing a
/// placement decision after the fact.
pub fn describe(monitors: &[Monitor]) -> String {
    if monitors.is_empty() {
        return "no monitors detected".to_owned();
    }
    monitors
        .iter()
        .map(|m| {
            format!(
                "bounds={} work={} scale={} {}",
                as_px(m.bounds),
                as_px(m.work),
                m.scale,
                if m.primary { "primary" } else { "secondary" },
            )
        })
        .collect::<Vec<_>>()
        .join(" | ")
}

/// The monitors attached right now.
///
/// An empty result means they could not be read — a headless run, or a platform
/// without a probe — and [`evaluate`] then leaves the window alone rather than
/// guessing.
#[cfg(windows)]
pub fn monitors(frame: &eframe::Frame) -> Vec<Monitor> {
    use winit::platform::windows::MonitorHandleExtWindows as _;

    let Some(window) = frame.winit_window() else {
        return Vec::new();
    };
    let primary = window.primary_monitor().map(|m| m.hmonitor());
    window
        .available_monitors()
        .map(|m| {
            let (pos, size) = (m.position(), m.size());
            let bounds = Rect::from_min_size(
                pos2(pos.x as f32, pos.y as f32),
                vec2(size.width as f32, size.height as f32),
            );
            Monitor {
                bounds,
                // Falling back to the full screen only loses the taskbar inset;
                // the placement itself still works.
                work: work_area(m.hmonitor()).unwrap_or(bounds),
                scale: m.scale_factor() as f32,
                primary: Some(m.hmonitor()) == primary,
            }
        })
        .collect()
}

#[cfg(not(windows))]
pub fn monitors(_frame: &eframe::Frame) -> Vec<Monitor> {
    Vec::new()
}

/// `MONITORINFO::rcWork`: the monitor minus the taskbar and any other appbar.
/// Windows reports it in pixels with the reservation already applied, so it is
/// correct at 100%, 150% or any other scaling percentage without further work.
#[cfg(windows)]
fn work_area(monitor: isize) -> Option<Rect> {
    use windows_sys::Win32::Graphics::Gdi::{GetMonitorInfoW, MONITORINFO};

    // SAFETY: MONITORINFO is a plain C struct of integers, so an all-zero value
    // is valid; `cbSize` is what tells the call how much it may write.
    let mut info: MONITORINFO = unsafe { std::mem::zeroed() };
    info.cbSize = size_of::<MONITORINFO>() as u32;
    // SAFETY: `monitor` is a live HMONITOR from winit's enumeration, and `info`
    // is a correctly sized, correctly declared MONITORINFO the call only writes.
    if unsafe { GetMonitorInfoW(monitor, &mut info) } == 0 {
        return None;
    }
    let w = info.rcWork;
    Some(Rect::from_min_max(
        pos2(w.left as f32, w.top as f32),
        pos2(w.right as f32, w.bottom as f32),
    ))
}

fn as_px(r: Rect) -> String {
    format!("({:.0},{:.0})-({:.0},{:.0})", r.min.x, r.min.y, r.max.x, r.max.y)
}

/// Windows always reports exactly one primary monitor; the fallback only
/// matters if that ever fails to come through.
fn primary(monitors: &[Monitor]) -> Option<&Monitor> {
    monitors.iter().find(|m| m.primary).or_else(|| monitors.first())
}

fn window_on(pos: Pos2, logical_size: Vec2, zoom: f32, m: &Monitor) -> Rect {
    Rect::from_min_size(pos, logical_size * m.scale * zoom)
}

/// The safe fallback: inset from the top-right of the work area, which keeps
/// clear of the taskbar wherever the user has docked it.
fn corner_of(m: &Monitor, logical_size: Vec2, zoom: f32) -> Pos2 {
    let size = logical_size * m.scale * zoom;
    let margin = MARGIN_PT * m.scale;
    clamp_into(m.work, pos2(m.work.max.x - size.x - margin, m.work.min.y + margin), size)
}

/// Nearest position that keeps a `size` window inside `work`. A window too
/// large to fit is pinned to the top-left rather than pushed off the other way.
fn clamp_into(work: Rect, pos: Pos2, size: Vec2) -> Pos2 {
    pos2(
        pos.x.clamp(work.min.x, (work.max.x - size.x).max(work.min.x)),
        pos.y.clamp(work.min.y, (work.max.y - size.y).max(work.min.y)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A medium overlay, in points — the size the legacy config file recorded.
    const OVERLAY: Vec2 = vec2(196.0, 91.0);

    fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect::from_min_size(pos2(x, y), vec2(w, h))
    }

    /// A monitor with `taskbar` pixels reserved along its bottom edge.
    fn monitor(bounds: Rect, taskbar: f32, scale: f32, primary: bool) -> Monitor {
        let work = Rect::from_min_max(bounds.min, pos2(bounds.max.x, bounds.max.y - taskbar));
        Monitor { bounds, work, scale, primary }
    }

    /// A single 2560×1440 screen with a 48 px taskbar.
    fn single_screen() -> Vec<Monitor> {
        vec![monitor(rect(0.0, 0.0, 2560.0, 1440.0), 48.0, 1.0, true)]
    }

    fn moved(outcome: Outcome) -> (Pos2, Reason) {
        match outcome {
            Outcome::Move { to, reason } => (to, reason),
            Outcome::Keep => panic!("expected a move, got Keep"),
        }
    }

    #[test]
    fn a_position_inside_the_work_area_is_left_alone() {
        assert_eq!(evaluate(pos2(100.0, 100.0), OVERLAY, 1.0, &single_screen()), Outcome::Keep);
    }

    #[test]
    fn unreadable_monitors_mean_no_opinion() {
        assert_eq!(evaluate(pos2(7000.0, 2400.0), OVERLAY, 1.0, &[]), Outcome::Keep);
    }

    /// A position far outside the only attached screen.
    #[test]
    fn a_position_outside_every_monitor_goes_to_the_primary_top_right_corner() {
        let (to, reason) = moved(evaluate(pos2(7000.0, 2400.0), OVERLAY, 1.0, &single_screen()));
        assert_eq!(reason, Reason::OffScreen);
        // Work area is 2560 wide; 24 pt of margin sits between it and the overlay.
        assert_eq!(to, pos2(2560.0 - 196.0 - 24.0, 24.0));
    }

    /// The point of the exercise: the screen the overlay lived on is gone.
    #[test]
    fn a_position_on_a_disconnected_monitor_is_recovered_onto_the_primary() {
        // Saved while a second screen sat to the right of this one.
        let (to, reason) = moved(evaluate(pos2(4200.0, 300.0), OVERLAY, 1.0, &single_screen()));
        assert_eq!(reason, Reason::OffScreen);
        assert_eq!(to, pos2(2340.0, 24.0));
        let work = single_screen()[0].work;
        assert!(work.contains_rect(Rect::from_min_size(to, OVERLAY)), "must land inside the work area");
    }

    #[test]
    fn a_position_on_a_secondary_monitor_is_not_dragged_to_the_primary() {
        let monitors = vec![
            monitor(rect(0.0, 0.0, 1920.0, 1080.0), 48.0, 1.0, true),
            monitor(rect(1920.0, 0.0, 2560.0, 1440.0), 0.0, 1.0, false),
        ];
        assert_eq!(evaluate(pos2(2000.0, 100.0), OVERLAY, 1.0, &monitors), Outcome::Keep);
    }

    #[test]
    fn a_monitor_left_of_the_primary_has_negative_coordinates_and_still_counts() {
        let monitors = vec![
            monitor(rect(0.0, 0.0, 1920.0, 1080.0), 48.0, 1.0, true),
            monitor(rect(-1920.0, -120.0, 1920.0, 1080.0), 0.0, 1.0, false),
        ];
        assert_eq!(evaluate(pos2(-1800.0, -50.0), OVERLAY, 1.0, &monitors), Outcome::Keep);
        // Further left than that screen reaches: nothing is visible any more.
        assert_eq!(moved(evaluate(pos2(-5000.0, -50.0), OVERLAY, 1.0, &monitors)).1, Reason::OffScreen);
    }

    #[test]
    fn a_window_over_the_taskbar_is_nudged_up_and_keeps_its_x() {
        // Bottom edge at 1431: still on the screen, but under the taskbar.
        let (to, reason) = moved(evaluate(pos2(800.0, 1340.0), OVERLAY, 1.0, &single_screen()));
        assert_eq!(reason, Reason::ClippedWorkArea);
        // 1440 tall minus a 48 px taskbar leaves the bottom of the work area at 1392.
        assert_eq!(to, pos2(800.0, 1392.0 - 91.0));
    }

    #[test]
    fn a_window_hanging_off_an_edge_but_still_grabbable_is_only_nudged_back() {
        let monitors = vec![monitor(rect(0.0, 0.0, 1920.0, 1080.0), 48.0, 1.0, true)];
        let (to, reason) = moved(evaluate(pos2(1820.0, 100.0), OVERLAY, 1.0, &monitors));
        assert_eq!(reason, Reason::ClippedWorkArea);
        assert_eq!(to, pos2(1920.0 - 196.0, 100.0));
    }

    #[test]
    fn a_window_with_only_a_sliver_showing_is_treated_as_lost() {
        let monitors = vec![monitor(rect(0.0, 0.0, 1920.0, 1080.0), 48.0, 1.0, true)];
        // 20 px of the overlay remain on screen: too little to grab.
        assert_eq!(moved(evaluate(pos2(1900.0, 100.0), OVERLAY, 1.0, &monitors)).1, Reason::OffScreen);
    }

    /// The taskbar is already excluded in pixels, but the corner margin and the
    /// window's own size are not: both have to follow the scaling factor.
    #[test]
    fn the_corner_margin_and_window_size_follow_the_monitor_scaling() {
        let monitors = vec![monitor(rect(0.0, 0.0, 3840.0, 2160.0), 72.0, 1.5, true)];
        let (to, _) = moved(evaluate(pos2(9000.0, 9000.0), OVERLAY, 1.0, &monitors));
        // 196 pt is 294 px at 150%, and the 24 pt margin is 36 px.
        assert_eq!(to, pos2(3840.0 - 294.0 - 36.0, 36.0));
    }

    #[test]
    fn the_egui_zoom_factor_is_part_of_the_window_size() {
        let monitors = vec![monitor(rect(0.0, 0.0, 1920.0, 1080.0), 0.0, 1.0, true)];
        let (to, reason) = moved(evaluate(pos2(1600.0, 100.0), OVERLAY, 2.0, &monitors));
        assert_eq!(reason, Reason::ClippedWorkArea);
        // At zoom 2 the overlay is 392 px wide, so it no longer fits at x=1600.
        assert_eq!(to, pos2(1920.0 - 392.0, 100.0));
    }

    #[test]
    fn a_window_taller_than_the_work_area_is_pinned_to_its_top_left() {
        let monitors = vec![monitor(rect(0.0, 0.0, 300.0, 200.0), 40.0, 1.0, true)];
        let (to, _) = moved(evaluate(pos2(7000.0, 7000.0), vec2(400.0, 300.0), 1.0, &monitors));
        assert_eq!(to, pos2(0.0, 0.0), "an oversized window starts at the corner rather than off it");
    }

    #[test]
    fn the_first_monitor_stands_in_when_none_is_flagged_primary() {
        let monitors = vec![monitor(rect(0.0, 0.0, 1920.0, 1080.0), 48.0, 1.0, false)];
        let (to, _) = moved(evaluate(pos2(7000.0, 7000.0), OVERLAY, 1.0, &monitors));
        assert_eq!(to, pos2(1920.0 - 196.0 - 24.0, 24.0));
    }

    #[test]
    fn describe_names_every_monitor_with_its_work_area_and_scaling() {
        let monitors = vec![
            monitor(rect(0.0, 0.0, 1920.0, 1080.0), 48.0, 1.0, true),
            monitor(rect(1920.0, 0.0, 2560.0, 1440.0), 0.0, 1.5, false),
        ];
        let text = describe(&monitors);
        assert!(text.contains("primary"), "{text}");
        assert!(text.contains("1032"), "the work area bottom should be visible: {text}");
        assert!(text.contains("1.5"), "the scaling factor should be visible: {text}");
    }
}
