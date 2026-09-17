//! System tray icon plus the native menu, which is shared between the tray and
//! the overlay's right-click so both always show the same state.

use std::sync::mpsc::{self, Receiver};

use eframe::egui;
use tray_icon::menu::{
    CheckMenuItem, ContextMenu as _, IsMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem,
    Submenu,
};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

use crate::app::{Mode, Size};
use crate::icon;

pub const TIMER_PRESETS: [u64; 9] = [1, 3, 5, 10, 15, 25, 30, 45, 60];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    ToggleLock,
    SetMode(Mode),
    SetTimerMinutes(u64),
    StartPause,
    Reset,
    SetSize(Size),
    ToggleBackdrop,
    ToggleOutline,
    ToggleChroma,
    ToggleSeconds,
    ToggleOnTop,
    Quit,
}

/// Menu item ids are plain strings; this is the single place they are decoded.
pub fn parse_command(id: &str) -> Option<Command> {
    Some(match id {
        "lock" => Command::ToggleLock,
        "startpause" => Command::StartPause,
        "reset" => Command::Reset,
        "backdrop" => Command::ToggleBackdrop,
        "outline" => Command::ToggleOutline,
        "chroma" => Command::ToggleChroma,
        "seconds" => Command::ToggleSeconds,
        "ontop" => Command::ToggleOnTop,
        "quit" => Command::Quit,
        _ => {
            let (kind, value) = id.split_once(':')?;
            match kind {
                "mode" => Command::SetMode(Mode::ALL.into_iter().find(|m| m.id() == value)?),
                "size" => Command::SetSize(Size::ALL.into_iter().find(|s| s.id() == value)?),
                "timer" => Command::SetTimerMinutes(value.parse().ok()?),
                _ => return None,
            }
        }
    })
}

/// Everything the menu needs to reflect.
#[derive(Debug, Clone, PartialEq)]
pub struct MenuState {
    pub locked: bool,
    pub lock_available: bool,
    pub mode: Mode,
    pub timer_minutes: u64,
    pub start_label: &'static str,
    pub size: Size,
    pub backdrop: bool,
    pub text_outline: bool,
    pub chroma: bool,
    pub show_seconds: bool,
    pub always_on_top: bool,
}

pub struct Tray {
    icon: Option<TrayIcon>,
    menu: Menu,
    lock: CheckMenuItem,
    modes: Vec<(Mode, CheckMenuItem)>,
    presets: Vec<(u64, CheckMenuItem)>,
    start_pause: MenuItem,
    reset: MenuItem,
    sizes: Vec<(Size, CheckMenuItem)>,
    backdrop: CheckMenuItem,
    outline: CheckMenuItem,
    chroma: CheckMenuItem,
    seconds: CheckMenuItem,
    on_top: CheckMenuItem,
    last_state: Option<MenuState>,
    icon_locked: Option<bool>,
    rx: Receiver<Command>,
}

impl Tray {
    /// Must be called on the event-loop thread once the loop is running
    /// (i.e. from the eframe app creator).
    pub fn new(ctx: &egui::Context) -> Self {
        let (tx, rx) = mpsc::channel();

        // Setting a handler diverts events away from the crate's own receivers, so
        // everything flows through our channel and wakes egui explicitly — even
        // while the overlay is click-through and receives no input of its own.
        {
            let (tx, ctx) = (tx.clone(), ctx.clone());
            MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
                if let Some(cmd) = parse_command(event.id.as_ref()) {
                    let _ = tx.send(cmd);
                    ctx.request_repaint();
                }
            }));
        }
        {
            let ctx = ctx.clone();
            TrayIconEvent::set_event_handler(Some(move |event: TrayIconEvent| {
                if let TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                } = event
                {
                    let _ = tx.send(Command::ToggleLock);
                    ctx.request_repaint();
                }
            }));
        }

        let check = |id: &str, text: &str| CheckMenuItem::with_id(id, text, true, false, None);

        let lock = check("lock", "Locked (click-through)");
        let modes: Vec<_> =
            Mode::ALL.into_iter().map(|m| (m, check(&format!("mode:{}", m.id()), m.label()))).collect();
        let presets: Vec<_> = TIMER_PRESETS
            .into_iter()
            .map(|min| (min, check(&format!("timer:{min}"), &format!("{min} min"))))
            .collect();
        let start_pause = MenuItem::with_id("startpause", "Start", true, None);
        let reset = MenuItem::with_id("reset", "Reset", true, None);
        let sizes: Vec<_> =
            Size::ALL.into_iter().map(|s| (s, check(&format!("size:{}", s.id()), s.label()))).collect();
        let backdrop = check("backdrop", "Backdrop");
        let outline = check("outline", "Text outline");
        let chroma = check("chroma", "Chroma key background (#00FF00)");
        let seconds = check("seconds", "Show seconds");
        let on_top = check("ontop", "Always on top");
        let quit = MenuItem::with_id("quit", "Quit ChronoDesk", true, None);

        let preset_refs: Vec<&dyn IsMenuItem> = presets.iter().map(|(_, i)| i as &dyn IsMenuItem).collect();
        let size_refs: Vec<&dyn IsMenuItem> = sizes.iter().map(|(_, i)| i as &dyn IsMenuItem).collect();
        let timer_menu = Submenu::with_items("Timer duration", true, &preset_refs).expect("timer submenu");
        let size_menu = Submenu::with_items("Size", true, &size_refs).expect("size submenu");

        let menu = Menu::new();
        let sep = PredefinedMenuItem::separator;
        let (s1, s2, s3, s4) = (sep(), sep(), sep(), sep());
        let mut items: Vec<&dyn IsMenuItem> = vec![&lock, &s1];
        items.extend(modes.iter().map(|(_, i)| i as &dyn IsMenuItem));
        items.extend([
            &timer_menu as &dyn IsMenuItem,
            &s2,
            &start_pause,
            &reset,
            &s3,
            &size_menu,
            &backdrop,
            &outline,
            &chroma,
            &seconds,
            &on_top,
            &s4,
            &quit,
        ]);
        menu.append_items(&items).expect("build menu");

        let icon = TrayIconBuilder::new()
            .with_id("chronodesk")
            .with_menu(Box::new(menu.clone()))
            .with_menu_on_left_click(false)
            .with_tooltip("ChronoDesk")
            .with_icon(tray_icon(false))
            .build()
            .inspect_err(|err| eprintln!("ChronoDesk: tray icon unavailable: {err}"))
            .ok();

        Self {
            icon,
            menu,
            lock,
            modes,
            presets,
            start_pause,
            reset,
            sizes,
            backdrop,
            outline,
            chroma,
            seconds,
            on_top,
            last_state: None,
            icon_locked: None,
            rx,
        }
    }

    /// Click-through without a tray icon would leave no way back, so locking
    /// is only offered when the tray exists.
    pub fn lock_available(&self) -> bool {
        self.icon.is_some()
    }

    pub fn commands(&self) -> impl Iterator<Item = Command> + '_ {
        self.rx.try_iter()
    }

    /// Pushes state into the menu and tray icon. Cheap no-op when unchanged, so
    /// it can run every frame (it also undoes the OS auto-toggling check items).
    pub fn sync(&mut self, state: MenuState) {
        if self.last_state.as_ref() == Some(&state) {
            return;
        }
        self.lock.set_checked(state.locked);
        self.lock.set_enabled(state.lock_available);
        for (mode, item) in &self.modes {
            item.set_checked(*mode == state.mode);
        }
        for (min, item) in &self.presets {
            item.set_checked(*min == state.timer_minutes);
        }
        let has_controls = state.mode != Mode::Clock;
        self.start_pause.set_text(state.start_label);
        self.start_pause.set_enabled(has_controls);
        self.reset.set_enabled(has_controls);
        for (size, item) in &self.sizes {
            item.set_checked(*size == state.size);
        }
        self.backdrop.set_checked(state.backdrop);
        self.outline.set_checked(state.text_outline);
        // An outline under a backdrop would be invisible anyway.
        self.outline.set_enabled(!state.backdrop);
        self.chroma.set_checked(state.chroma);
        self.seconds.set_checked(state.show_seconds);
        self.on_top.set_checked(state.always_on_top);

        if self.icon_locked != Some(state.locked) && let Some(tray) = &self.icon {
            let tooltip = if state.locked {
                "ChronoDesk — locked (click icon to unlock)"
            } else {
                "ChronoDesk — click icon to lock"
            };
            let _ = tray.set_icon(Some(tray_icon(state.locked)));
            let _ = tray.set_tooltip(Some(tooltip));
            self.icon_locked = Some(state.locked);
        }
        self.last_state = Some(state);
    }

    /// Forces the next [`Self::sync`] to rewrite every item, e.g. after the OS
    /// toggled a check item on its own.
    pub fn invalidate(&mut self) {
        self.last_state = None;
    }

    /// Opens the shared menu as a native popup at the cursor. Blocks until closed;
    /// the selection arrives through the normal command channel.
    pub fn show_context_menu(&self, frame: &eframe::Frame) {
        use raw_window_handle::{HasWindowHandle as _, RawWindowHandle};
        let Ok(handle) = frame.window_handle() else { return };
        match handle.as_raw() {
            #[cfg(target_os = "windows")]
            RawWindowHandle::Win32(h) => unsafe {
                self.menu.show_context_menu_for_hwnd(h.hwnd.get(), None);
            },
            #[cfg(target_os = "macos")]
            RawWindowHandle::AppKit(h) => unsafe {
                self.menu.show_context_menu_for_nsview(h.ns_view.as_ptr().cast(), None);
            },
            _ => {}
        }
    }
}

fn tray_icon(locked: bool) -> Icon {
    let img = icon::app_icon(32, locked);
    Icon::from_rgba(img.rgba, img.size, img.size).expect("valid icon")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_all_menu_ids() {
        assert_eq!(parse_command("lock"), Some(Command::ToggleLock));
        assert_eq!(parse_command("quit"), Some(Command::Quit));
        assert_eq!(parse_command("timer:25"), Some(Command::SetTimerMinutes(25)));
        for mode in Mode::ALL {
            assert_eq!(parse_command(&format!("mode:{}", mode.id())), Some(Command::SetMode(mode)));
        }
        for size in Size::ALL {
            assert_eq!(parse_command(&format!("size:{}", size.id())), Some(Command::SetSize(size)));
        }
    }

    #[test]
    fn rejects_unknown_ids() {
        assert_eq!(parse_command(""), None);
        assert_eq!(parse_command("mode:nope"), None);
        assert_eq!(parse_command("timer:abc"), None);
        assert_eq!(parse_command("bogus:1"), None);
    }
}
