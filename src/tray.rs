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
use crate::hotkeys;
use crate::icon;
use crate::layout::Font;
use crate::market::{Labels, Market};
use crate::night::{NightMode, TimeOfDay};
use crate::theme::Palette;
use crate::world::City;

pub const TIMER_PRESETS: [u64; 9] = [1, 3, 5, 10, 15, 25, 30, 45, 60];
/// The alarm's minutes in the menu; the config file takes any minute.
pub const ALARM_MINUTE_STEP: u32 = 5;

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
    /// Shows another city's time after the date, or none.
    SetSecondZone(Option<City>),
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
    /// Runs the timer as a Pomodoro cycle, or as a plain timer again.
    TogglePomodoro,
    /// Arms the alarm at the time last picked, or disarms it.
    ToggleAlarm,
    /// Picks the alarm's hour or minute, keeping the other, and arms it.
    SetAlarmHour(u32),
    SetAlarmMinute(u32),
    /// Arms the alarm at an exact time (scripts; the menu picks hour and
    /// minute separately).
    SetAlarm(TimeOfDay),
    ToggleOnTop,
    /// Registers the global hotkeys, or gives them back.
    ToggleHotkeys,
    /// Registers this exe to start at login, or removes the entry.
    ToggleAutostart,
    /// Shows the welcome card again.
    ShowWelcome,
    /// Installs the newer release the update check found, or opens its page
    /// where it cannot be installed from here.
    Update,
    ToggleUpdateCheck,
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
        "pomodoro" => Command::TogglePomodoro,
        "alarm" => Command::ToggleAlarm,
        "horizontal" => Command::ToggleBoardLayout,
        "codes" => Command::ToggleBoardLabels,
        "ontop" => Command::ToggleOnTop,
        "hotkeys" => Command::ToggleHotkeys,
        "autostart" => Command::ToggleAutostart,
        "welcome" => Command::ShowWelcome,
        "update" => Command::Update,
        "checkupdates" => Command::ToggleUpdateCheck,
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
                "zone" if value == "none" => Command::SetSecondZone(None),
                "zone" => Command::SetSecondZone(Some(City::from_id(value)?)),
                "alarm" => Command::SetAlarm(TimeOfDay::parse(value)?),
                "alarmhour" => Command::SetAlarmHour(value.parse().ok().filter(|h| *h < 24)?),
                "alarmmin" => Command::SetAlarmMinute(value.parse().ok().filter(|m| *m < 60)?),
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
    pub second_zone: Option<City>,
    pub palette: Palette,
    pub night: NightMode,
    pub markets: Vec<Market>,
    pub board_layout: Layout,
    pub board_labels: Labels,
    pub seconds_ring: bool,
    pub timer_sound: bool,
    pub pomodoro: bool,
    /// The alarm's time, whether it is armed, and the switch's label.
    pub alarm_at: TimeOfDay,
    pub alarm_on: bool,
    pub alarm_label: String,
    pub always_on_top: bool,
    pub hotkeys: bool,
    pub autostart: bool,
    pub autostart_available: bool,
    pub check_updates: bool,
    /// The item at the top of the menu while an update is on offer, and
    /// whether it can be clicked (not while one is downloading).
    pub update: Option<(String, bool)>,
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
    pomodoro: CheckMenuItem,
    alarm: CheckMenuItem,
    alarm_hours: Vec<(u32, CheckMenuItem)>,
    alarm_minutes: Vec<(u32, CheckMenuItem)>,
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
    /// "None" first, then every city.
    zones: Vec<(Option<City>, CheckMenuItem)>,
    on_top: CheckMenuItem,
    hotkeys: CheckMenuItem,
    autostart: CheckMenuItem,
    check_updates: CheckMenuItem,
    /// Only in the menu while there is an update to offer.
    update: (MenuItem, PredefinedMenuItem),
    update_shown: bool,
    last_state: Option<MenuState>,
    /// What the tray icon and its tooltip last showed: locked, the update,
    /// and the armed alarm.
    icon_state: Option<(bool, Option<String>, Option<String>)>,
    rx: Receiver<Command>,
}

impl Tray {
    /// Must be called on the event-loop thread once the loop is running
    /// (i.e. from the eframe app creator). Without `show_icon` there is a menu
    /// but nothing in the notification area: a scripted test instance would
    /// otherwise put a second, identical icon next to the user's own, and a
    /// click on the wrong one locks the test, or opens a menu that blocks its
    /// event loop for as long as it stays open.
    pub fn new(ctx: &egui::Context, show_icon: bool) -> Self {
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
        let pomodoro = check("pomodoro", "Pomodoro cycle");
        let alarm = check("alarm", "Alarm at 07:00");
        let alarm_hours: Vec<_> = (0..24).map(|h| (h, check(&format!("alarmhour:{h}"), &format!("{h:02}")))).collect();
        let alarm_minutes: Vec<_> = (0..60)
            .step_by(ALARM_MINUTE_STEP as usize)
            .map(|m| (m, check(&format!("alarmmin:{m}"), &format!(":{m:02}"))))
            .collect();
        let horizontal = check("horizontal", "One line");
        let codes = check("codes", "Exchange codes");
        let backdrop = check("backdrop", "Backdrop");
        let outline = check("outline", "Text outline");
        let chroma = check("chroma", "Chroma key background (#00FF00)");
        let seconds = check("seconds", "Show seconds");
        let twelve_hour = check("12h", "12-hour clock");
        let date = check("date", "Show date");
        let zones: Vec<_> = std::iter::once((None, check("zone:none", "None")))
            .chain(City::ALL.into_iter().map(|c| (Some(c), check(&format!("zone:{}", c.id()), c.label()))))
            .collect();
        let on_top = check("ontop", "Always on top");
        let hotkeys = check("hotkeys", &format!("Global hotkeys ({})", hotkeys::MODIFIERS));
        let autostart = check("autostart", "Start with Windows");
        let check_updates = check("checkupdates", "Check for updates");
        let update = (MenuItem::with_id("update", "Update available", true, None), PredefinedMenuItem::separator());
        let tips = MenuItem::with_id("welcome", "Quick tips", true, None);
        let quit = MenuItem::with_id("quit", "Quit ChronoDesk", true, None);

        let mut preset_refs: Vec<&dyn IsMenuItem> = presets.iter().map(|(_, i)| i as &dyn IsMenuItem).collect();
        let (m1, t1) = (PredefinedMenuItem::separator(), PredefinedMenuItem::separator());
        preset_refs.extend([&t1 as &dyn IsMenuItem, &pomodoro, &timer_sound]);
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
        let z1 = PredefinedMenuItem::separator();
        let mut zone_refs: Vec<&dyn IsMenuItem> = zones.iter().map(|(_, i)| i as &dyn IsMenuItem).collect();
        zone_refs.insert(1, &z1);
        let zone_menu = Submenu::with_items("Second time zone", true, &zone_refs).expect("zone submenu");
        let hour_refs: Vec<&dyn IsMenuItem> = alarm_hours.iter().map(|(_, i)| i as &dyn IsMenuItem).collect();
        let minute_refs: Vec<&dyn IsMenuItem> = alarm_minutes.iter().map(|(_, i)| i as &dyn IsMenuItem).collect();
        let hour_menu = Submenu::with_items("Hour", true, &hour_refs).expect("alarm hour submenu");
        let minute_menu = Submenu::with_items("Minute", true, &minute_refs).expect("alarm minute submenu");
        let al1 = PredefinedMenuItem::separator();
        let alarm_menu =
            Submenu::with_items("Alarm", true, &[&alarm as &dyn IsMenuItem, &al1, &hour_menu, &minute_menu])
                .expect("alarm submenu");

        // Everything about how the overlay looks lives in one submenu, so the
        // top level stays the short list it was.
        let sep = PredefinedMenuItem::separator;
        let a1 = sep();
        let appearance_items: [&dyn IsMenuItem; 13] = [
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
            &zone_menu,
        ];
        let appearance_menu = Submenu::with_items("Appearance", true, &appearance_items).expect("appearance submenu");

        let menu = Menu::new();
        let (s1, s2, s3, s4) = (sep(), sep(), sep(), sep());
        let mut items: Vec<&dyn IsMenuItem> = vec![&lock, &s1];
        items.extend(modes.iter().map(|(_, i)| i as &dyn IsMenuItem));
        items.extend([
            &timer_menu as &dyn IsMenuItem,
            &market_menu,
            &alarm_menu,
            &s2,
            &start_pause,
            &reset,
            &s3,
            &appearance_menu,
            &on_top,
            &hotkeys,
            &autostart,
            &check_updates,
            &s4,
            &tips,
            &quit,
        ]);
        menu.append_items(&items).expect("build menu");

        let icon = show_icon
            .then(|| {
                TrayIconBuilder::new()
                    .with_id("chronodesk")
                    .with_menu(Box::new(menu.clone()))
                    .with_menu_on_left_click(false)
                    .with_tooltip("ChronoDesk")
                    .with_icon(tray_icon(false, false))
                    .build()
                    .inspect_err(|err| eprintln!("ChronoDesk: tray icon unavailable: {err}"))
                    .ok()
            })
            .flatten();

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
            pomodoro,
            alarm,
            alarm_hours,
            alarm_minutes,
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
            zones,
            on_top,
            hotkeys,
            autostart,
            check_updates,
            update,
            update_shown: false,
            last_state: None,
            icon_state: None,
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
        self.pomodoro.set_checked(state.pomodoro);
        self.alarm.set_checked(state.alarm_on);
        self.alarm.set_text(&state.alarm_label);
        for (hour, item) in &self.alarm_hours {
            item.set_checked(*hour == state.alarm_at.hour());
        }
        for (minute, item) in &self.alarm_minutes {
            item.set_checked(*minute == state.alarm_at.minute());
        }
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
        for (zone, item) in &self.zones {
            item.set_checked(*zone == state.second_zone);
        }
        self.backdrop.set_checked(state.backdrop);
        self.outline.set_checked(state.text_outline);
        // An outline under a backdrop would be invisible anyway, and under a
        // chroma key it is switched off to keep the key colour clean.
        self.outline.set_enabled(!state.backdrop && !state.chroma);
        // A backdrop is translucent, and keys out as a tint.
        self.backdrop.set_enabled(!state.chroma);
        self.chroma.set_checked(state.chroma);
        self.seconds.set_checked(state.show_seconds);
        self.on_top.set_checked(state.always_on_top);
        self.hotkeys.set_checked(state.hotkeys);
        self.autostart.set_checked(state.autostart);
        self.autostart.set_enabled(state.autostart_available);
        self.check_updates.set_checked(state.check_updates);

        // The offer heads the menu, above everything else, and is gone when
        // there is nothing to offer.
        match (&state.update, self.update_shown) {
            (Some((label, enabled)), shown) => {
                self.update.0.set_text(label);
                self.update.0.set_enabled(*enabled);
                if !shown {
                    let _ = self.menu.insert_items(&[&self.update.0, &self.update.1], 0);
                    self.update_shown = true;
                }
            }
            (None, true) => {
                let _ = self.menu.remove(&self.update.0);
                let _ = self.menu.remove(&self.update.1);
                self.update_shown = false;
            }
            (None, false) => {}
        }

        let armed = state.alarm_on.then(|| state.alarm_label.clone());
        let icon_state = (state.locked, state.update.as_ref().map(|(label, _)| label.clone()), armed);
        if self.icon_state.as_ref() != Some(&icon_state) && let Some(tray) = &self.icon {
            let mut tooltip = if state.locked {
                "ChronoDesk — locked (click icon to unlock)".to_owned()
            } else {
                "ChronoDesk — click icon to lock".to_owned()
            };
            if let Some(armed) = &icon_state.2 {
                tooltip.push_str(&format!("\n{armed}"));
            }
            if let Some((label, _)) = &state.update {
                tooltip.push_str(&format!("\n{label} (right-click)"));
            }
            let _ = tray.set_icon(Some(tray_icon(state.locked, state.update.is_some())));
            let _ = tray.set_tooltip(Some(tooltip));
            self.icon_state = Some(icon_state);
        }
        self.last_state = Some(state);
    }

    /// The top-level items, and whether the update offer is among them.
    #[cfg(test)]
    pub fn top_level(&self) -> (usize, bool) {
        (self.menu.items().len(), self.update_shown)
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

fn tray_icon(locked: bool, update: bool) -> Icon {
    let img = icon::app_icon(32, locked);
    let img = if update { icon::with_badge(img) } else { img };
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
        assert_eq!(parse_command("hotkeys"), Some(Command::ToggleHotkeys));
        assert_eq!(parse_command("welcome"), Some(Command::ShowWelcome));
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
        assert_eq!(parse_command("zone:none"), Some(Command::SetSecondZone(None)));
        for city in City::ALL {
            assert_eq!(parse_command(&format!("zone:{}", city.id())), Some(Command::SetSecondZone(Some(city))));
        }
        assert_eq!(parse_command("zone:atlantis"), None);
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
        assert_eq!(parse_command("pomodoro"), Some(Command::TogglePomodoro));
    }

    #[test]
    fn parses_alarm_ids() {
        assert_eq!(parse_command("alarm"), Some(Command::ToggleAlarm));
        assert_eq!(parse_command("alarm:14:35"), Some(Command::SetAlarm(TimeOfDay::new(14, 35).unwrap())));
        assert_eq!(parse_command("alarm:7:05"), Some(Command::SetAlarm(TimeOfDay::new(7, 5).unwrap())));
        assert_eq!(parse_command("alarmhour:0"), Some(Command::SetAlarmHour(0)));
        assert_eq!(parse_command("alarmhour:23"), Some(Command::SetAlarmHour(23)));
        assert_eq!(parse_command("alarmmin:55"), Some(Command::SetAlarmMinute(55)));
        assert_eq!(parse_command("alarmhour:24"), None);
        assert_eq!(parse_command("alarmmin:60"), None);
        assert_eq!(parse_command("alarm:25:00"), None);
        assert_eq!(parse_command("alarm:noon"), None);
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
