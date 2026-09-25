//! The native window underneath the overlay.

use eframe::egui::{self, Pos2, Vec2, ViewportCommand, pos2};

use super::ChronoApp;

impl ChronoApp {
    pub(super) fn fit_window(&mut self, ctx: &egui::Context, size: Vec2) {
        if self.window_size != Some(size) {
            self.window_size = Some(size);
            ctx.send_viewport_cmd(ViewportCommand::InnerSize(size));
        }
    }
}

/// The window's handle as a plain number, so another thread may hold it.
pub(super) fn native_window(cc: &eframe::CreationContext<'_>) -> isize {
    use raw_window_handle::{HasWindowHandle as _, RawWindowHandle};
    match cc.window_handle().map(|handle| handle.as_raw()) {
        Ok(RawWindowHandle::Win32(handle)) => handle.hwnd.get(),
        _ => 0,
    }
}

/// Safe from any thread: `ShowWindow` posts to the window's own thread.
pub(super) fn restore_if_minimised(window: isize) {
    #[cfg(windows)]
    if window != 0 {
        #[link(name = "user32")]
        unsafe extern "system" {
            fn IsIconic(window: isize) -> i32;
            fn ShowWindowAsync(window: isize, command: i32) -> i32;
        }
        const SW_RESTORE: i32 = 9;
        // SAFETY: both take a window handle by value and tolerate a stale one.
        unsafe {
            if IsIconic(window) != 0 {
                // Async: this runs on the listener's thread, and the window's
                // own thread may be busy; posting does not wait for it.
                ShowWindowAsync(window, SW_RESTORE);
            }
        }
    }
    #[cfg(not(windows))]
    let _ = window;
}

/// Windows 11 draws a 1px border and rounds corners even on undecorated
/// windows, which shows up as a faint box around a transparent overlay.
#[cfg(windows)]
pub(super) fn strip_window_chrome(frame: &eframe::Frame) {
    use winit::platform::windows::{CornerPreference, WindowExtWindows as _};
    if let Some(window) = frame.winit_window() {
        window.set_border_color(None);
        window.set_undecorated_shadow(false);
        window.set_corner_preference(CornerPreference::DoNotRound);
    }
}

#[cfg(not(windows))]
pub(super) fn strip_window_chrome(_frame: &eframe::Frame) {}

/// Where the window is, in physical pixels, asked of the OS directly: no scale
/// factor enters into it, so one that lags a frame behind a move to another
/// monitor cannot skew it.
pub(super) fn window_position_px(ctx: &egui::Context, frame: &eframe::Frame) -> Option<Pos2> {
    if let Some(window) = frame.winit_window() {
        return window.outer_position().ok().map(|px| pos2(px.x as f32, px.y as f32));
    }
    let ppp = ctx.pixels_per_point();
    ctx.input(|i| i.viewport().outer_rect).map(|rect| pos2(rect.min.x * ppp, rect.min.y * ppp))
}

