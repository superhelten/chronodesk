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
use crate::board::Layout;
use crate::clock::ClockFormat;
use crate::icon;
use crate::layout::Font;
use crate::market::{Labels, Market};
use crate::night::NightMode;
use crate::theme::Palette;

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
    ToggleClockFormat,
    ToggleDate,
    /// The old two-way typeface / seven-segment switch.
    ToggleFont,
    SetFont(Font),
    SetPalette(Palette),
    SetNight(NightMode),
    /// Adds the exchange to the market board, or removes it.
    ToggleMarket(Market),
    ToggleBoardLayout,
    ToggleBoardLabels,
    ToggleRing,
    ToggleTimerSound,
    ToggleOnTop,
    /// Registers this exe to start at login, or removes the entry.
    ToggleAutostart,
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
        "12h" => Command::ToggleClockFormat,
        "date" => Command::ToggleDate,
        "digital" => Command::ToggleFont,
        "ring" => Command::ToggleRing,
        "timersound" => Command::ToggleTimerSound,
        "horizontal" => Command::ToggleBoardLayout,
        "codes" => Command::ToggleBoardLabels,
        "ontop" => Command::ToggleOnTop,
        "autostart" => Command::ToggleAutostart,
        "quit" => Command::Quit,
        _ => {
            let (kind, value) = id.split_once(':')?;
            match kind {
                "mode" => Command::SetMode(Mode::ALL.into_iter().find(|m| m.id() == value)?),
                "size" => Command::SetSize(Size::ALL.into_iter().find(|s| s.id() == value)?),
                "timer" => Command::SetTimerMinutes(value.parse().ok()?),
                "font" => Command::SetFont(Font::ALL.into_iter().find(|f| f.id() == value)?),
                "palette" => Command::SetPalette(Palette::ALL.into_iter().find(|p| p.id() == value)?),
                "night" => Command::SetNight(NightMode::ALL.into_iter().find(|n| n.id() == value)?),
                "market" => Command::ToggleMarket(Market::ALL.into_iter().find(|m| m.id() == value)?),
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
    pub font: Font,
    pub backdrop: bool,
    pub text_outline: bool,
    pub chroma: bool,
    pub show_seconds: bool,
    pub clock_format: ClockFormat,
    pub show_date: bool,
    pub palette: Palette,
    pub night: NightMode,
    pub markets: Vec<Market>,
    pub board_layout: Layout,
    pub board_labels: Labels,
    pub seconds_ring: bool,
    pub timer_sound: bool,
    pub always_on_top: bool,
    pub autostart: bool,
    pub autostart_available: bool,
}

pub struct Tray {
    icon: Option<TrayIcon>,
    menu: Menu,
    lock: CheckMenuItem,
    modes: Vec<(Mode, CheckMenuItem)>,
    presets: Vec<(u64, CheckMenuItem)>,
    markets: Vec<(Market, CheckMenuItem)>,
    horizontal: CheckMenuItem,
    codes: CheckMenuItem,
    ring: CheckMenuItem,
    timer_sound: CheckMenuItem,
    start_pause: MenuItem,
    reset: MenuItem,
    sizes: Vec<(Size, CheckMenuItem)>,
    faces: Vec<(Font, CheckMenuItem)>,
    palettes: Vec<(Palette, CheckMenuItem)>,
    nights: Vec<(NightMode, CheckMenuItem)>,
    backdrop: CheckMenuItem,
    outline: CheckMenuItem,
    chroma: CheckMenuItem,
    seconds: CheckMenuItem,
    twelve_hour: CheckMenuItem,
    date: CheckMenuItem,
    on_top: CheckMenuItem,
    autostart: CheckMenuItem,
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
        let markets: Vec<_> =
            Market::ALL.into_iter().map(|m| (m, check(&format!("market:{}", m.id()), m.label()))).collect();
        let start_pause = MenuItem::with_id("startpause", "Start", true, None);
        let reset = MenuItem::with_id("reset", "Reset", true, None);
        let sizes: Vec<_> =
            Size::ALL.into_iter().map(|s| (s, check(&format!("size:{}", s.id()), s.label()))).collect();
        let palettes: Vec<_> =
            Palette::ALL.into_iter().map(|p| (p, check(&format!("palette:{}", p.id()), p.label()))).collect();
        let nights: Vec<_> =
            NightMode::ALL.into_iter().map(|n| (n, check(&format!("night:{}", n.id()), n.label()))).collect();
        let faces: Vec<_> =
            Font::ALL.into_iter().map(|f| (f, check(&format!("font:{}", f.id()), f.label()))).collect();
        let ring = check("ring", "Seconds ring");
        let timer_sound = check("timersound", "Sound when finished");
        let horizontal = check("horizontal", "One line");
        let codes = check("codes", "Exchange codes");
        let backdrop = check("backdrop", "Backdrop");
        let outline = check("outline", "Text outline");
        let chroma = check("chroma", "Chroma key background (#00FF00)");
        let seconds = check("seconds", "Show seconds");
        let twelve_hour = check("12h", "12-hour clock");
        let date = check("date", "Show date");
        let on_top = check("ontop", "Always on top");
        let autostart = check("autostart", "Start with Windows");
        let quit = MenuItem::with_id("quit", "Quit ChronoDesk", true, None);

        let mut preset_refs: Vec<&dyn IsMenuItem> = presets.iter().map(|(_, i)| i as &dyn IsMenuItem).collect();
        let (m1, t1) = (PredefinedMenuItem::separator(), PredefinedMenuItem::separator());
        preset_refs.extend([&t1 as &dyn IsMenuItem, &timer_sound]);
        let mut market_refs: Vec<&dyn IsMenuItem> = vec![&horizontal, &codes, &m1];
        market_refs.extend(markets.iter().map(|(_, i)| i as &dyn IsMenuItem));
        let size_refs: Vec<&dyn IsMenuItem> = sizes.iter().map(|(_, i)| i as &dyn IsMenuItem).collect();
        let face_refs: Vec<&dyn IsMenuItem> = faces.iter().map(|(_, i)| i as &dyn IsMenuItem).collect();
        let palette_refs: Vec<&dyn IsMenuItem> = palettes.iter().map(|(_, i)| i as &dyn IsMenuItem).collect();
        let night_refs: Vec<&dyn IsMenuItem> = nights.iter().map(|(_, i)| i as &dyn IsMenuItem).collect();
        let timer_menu = Submenu::with_items("Timer duration", true, &preset_refs).expect("timer submenu");
        let market_menu = Submenu::with_items("Exchanges", true, &market_refs).expect("market submenu");
        let size_menu = Submenu::with_items("Size", true, &size_refs).expect("size submenu");
        let face_menu = Submenu::with_items("Face", true, &face_refs).expect("face submenu");
        let palette_menu = Submenu::with_items("Colours", true, &palette_refs).expect("palette submenu");
        let night_menu = Submenu::with_items("Night mode", true, &night_refs).expect("night submenu");

        // Everything about how the overlay looks lives in one submenu, so the
        // top level stays the short list it was.
        let sep = PredefinedMenuItem::separator;
        let a1 = sep();
        let appearance_items: [&dyn IsMenuItem; 12] = [
            &size_menu,
            &face_menu,
            &palette_menu,
            &night_menu,
            &a1,
            &ring,
            &backdrop,
            &outline,
            &chroma,
            &seconds,
            &twelve_hour,
            &date,
        ];
        let appearance_menu = Submenu::with_items("Appearance", true, &appearance_items).expect("appearance submenu");

        let menu = Menu::new();
        let (s1, s2, s3, s4) = (sep(), sep(), sep(), sep());
        let mut items: Vec<&dyn IsMenuItem> = vec![&lock, &s1];
        items.extend(modes.iter().map(|(_, i)| i as &dyn IsMenuItem));
        items.extend([
            &timer_menu as &dyn IsMenuItem,
            &market_menu,
            &s2,
            &start_pause,
            &reset,
            &s3,
            &appearance_menu,
            &on_top,
            &autostart,
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
            markets,
            horizontal,
            codes,
            ring,
            timer_sound,
            start_pause,
            reset,
            sizes,
            faces,
            palettes,
            nights,
            backdrop,
            outline,
            chroma,
            seconds,
            twelve_hour,
            date,
            on_top,
            autostart,
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
        for (market, item) in &self.markets {
            item.set_checked(state.markets.contains(market));
            // The board is never left empty, so its last row cannot be unchecked.
            item.set_enabled(!(state.markets.len() == 1 && state.markets[0] == *market));
        }
        self.horizontal.set_checked(state.board_layout == Layout::Horizontal);
        self.codes.set_checked(state.board_labels == Labels::Code);
        self.ring.set_checked(state.seconds_ring);
        self.timer_sound.set_checked(state.timer_sound);
        // The board has no seconds to show.
        self.ring.set_enabled(state.mode != Mode::Market);
        let has_controls = state.mode.has_controls();
        self.start_pause.set_text(state.start_label);
        self.start_pause.set_enabled(has_controls);
        self.reset.set_enabled(has_controls);
        for (size, item) in &self.sizes {
            item.set_checked(*size == state.size);
        }
        for (font, item) in &self.faces {
            item.set_checked(*font == state.font);
        }
        for (palette, item) in &self.palettes {
            item.set_checked(*palette == state.palette);
        }
        for (night, item) in &self.nights {
            item.set_checked(*night == state.night);
        }
        self.twelve_hour.set_checked(state.clock_format == ClockFormat::H12);
        self.date.set_checked(state.show_date);
        self.backdrop.set_checked(state.backdrop);
        self.outline.set_checked(state.text_outline);
        // An outline under a backdrop would be invisible anyway, and under a
        // chroma key it is switched off to keep the key colour clean.
        self.outline.set_enabled(!state.backdrop && !state.chroma);
        self.chroma.set_checked(state.chroma);
        self.seconds.set_checked(state.show_seconds);
        self.on_top.set_checked(state.always_on_top);
        self.autostart.set_checked(state.autostart);
        self.autostart.set_enabled(state.autostart_available);

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
        assert_eq!(parse_command("autostart"), Some(Command::ToggleAutostart));
        assert_eq!(parse_command("timer:25"), Some(Command::SetTimerMinutes(25)));
        for mode in Mode::ALL {
            assert_eq!(parse_command(&format!("mode:{}", mode.id())), Some(Command::SetMode(mode)));
        }
        for size in Size::ALL {
            assert_eq!(parse_command(&format!("size:{}", size.id())), Some(Command::SetSize(size)));
        }
    }

    #[test]
    fn parses_appearance_menu_ids() {
        assert_eq!(parse_command("12h"), Some(Command::ToggleClockFormat));
        assert_eq!(parse_command("digital"), Some(Command::ToggleFont));
        for font in Font::ALL {
            assert_eq!(parse_command(&format!("font:{}", font.id())), Some(Command::SetFont(font)));
        }
        assert_eq!(parse_command("font:serif"), None);
        assert_eq!(parse_command("date"), Some(Command::ToggleDate));
        for palette in Palette::ALL {
            assert_eq!(parse_command(&format!("palette:{}", palette.id())), Some(Command::SetPalette(palette)));
        }
        for mode in NightMode::ALL {
            assert_eq!(parse_command(&format!("night:{}", mode.id())), Some(Command::SetNight(mode)));
        }
        assert_eq!(parse_command("palette:neon"), None);
        assert_eq!(parse_command("night:dusk"), None);
    }

    #[test]
    fn parses_market_ids() {
        assert_eq!(parse_command("mode:market"), Some(Command::SetMode(Mode::Market)));
        for market in Market::ALL {
            assert_eq!(parse_command(&format!("market:{}", market.id())), Some(Command::ToggleMarket(market)));
        }
        assert_eq!(parse_command("market:atlantis"), None);
        assert_eq!(parse_command("horizontal"), Some(Command::ToggleBoardLayout));
        assert_eq!(parse_command("codes"), Some(Command::ToggleBoardLabels));
        assert_eq!(parse_command("ring"), Some(Command::ToggleRing));
        assert_eq!(parse_command("timersound"), Some(Command::ToggleTimerSound));
    }

    #[test]
    fn only_the_running_modes_have_controls() {
        assert!(!Mode::Clock.has_controls());
        assert!(!Mode::Market.has_controls());
        assert!(Mode::Stopwatch.has_controls());
        assert!(Mode::Timer.has_controls());
    }

    #[test]
    fn rejects_unknown_ids() {
        assert_eq!(parse_command(""), None);
        assert_eq!(parse_command("mode:nope"), None);
        assert_eq!(parse_command("timer:abc"), None);
        assert_eq!(parse_command("bogus:1"), None);
    }
}
