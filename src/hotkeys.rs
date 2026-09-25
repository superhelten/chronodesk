//! Global hotkeys: the overlay's keys, reachable from any window.
//!
//! The overlay only hears `Space`, `R` and `1`–`4` while it has the focus,
//! which it never takes by itself and cannot have at all while it is locked
//! click-through. So the same keys are registered with Windows under
//! Ctrl+Alt+Shift, plus `L` for the lock, which makes a locked overlay
//! reachable without the tray icon.
//!
//! Three modifiers, because a hotkey belongs to whoever registered it, in
//! every program: Ctrl+Alt alone is AltGr on many keyboards (`@`, `{`, `€`),
//! and a game or an editor may want anything with fewer. A chord another
//! program holds already is skipped and reported, not fought over.
//!
//! `RegisterHotKey` without a window posts `WM_HOTKEY` to the thread that
//! registered it, so the hotkeys live on a thread of their own with a
//! message loop, and hand what they hear to the app the way the tray does:
//! through a channel, with a repaint to wake it.

use std::sync::mpsc::Sender;

use crate::app::Mode;
use crate::tray::Command;

/// What is held with every key.
pub const MODIFIERS: &str = "Ctrl+Alt+Shift";
/// The keys, as the welcome card lists them.
#[cfg(test)]
pub const KEYS: &str = "Space R L 1-4";

/// A key and what it does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Chord {
    /// Windows virtual-key code.
    pub vk: u32,
    /// The key as the menu and the card name it.
    pub key: &'static str,
    pub command: Command,
}

pub const CHORDS: [Chord; 7] = [
    Chord { vk: 0x20, key: "Space", command: Command::StartPause },
    Chord { vk: b'R' as u32, key: "R", command: Command::Reset },
    Chord { vk: b'L' as u32, key: "L", command: Command::ToggleLock },
    Chord { vk: b'1' as u32, key: "1", command: Command::SetMode(Mode::Clock) },
    Chord { vk: b'2' as u32, key: "2", command: Command::SetMode(Mode::Stopwatch) },
    Chord { vk: b'3' as u32, key: "3", command: Command::SetMode(Mode::Timer) },
    Chord { vk: b'4' as u32, key: "4", command: Command::SetMode(Mode::Market) },
];

/// The command for a hotkey id, as `WM_HOTKEY` reports it: the chord's
/// position in [`CHORDS`].
fn command(id: usize) -> Option<Command> {
    CHORDS.get(id).map(|chord| chord.command)
}

pub use imp::Hotkeys;

#[cfg(windows)]
mod imp {
    use std::sync::mpsc;
    use std::thread::JoinHandle;

    use super::{CHORDS, Chord, Command, MODIFIERS, command};

    const MOD_ALT: u32 = 0x1;
    const MOD_CONTROL: u32 = 0x2;
    const MOD_SHIFT: u32 = 0x4;
    /// Held down, a chord fires once rather than at the keyboard's repeat rate.
    const MOD_NOREPEAT: u32 = 0x4000;
    pub(super) const WM_HOTKEY: u32 = 0x0312;
    const WM_QUIT: u32 = 0x0012;
    const PM_NOREMOVE: u32 = 0;

    #[repr(C)]
    #[derive(Default)]
    struct Msg {
        window: isize,
        message: u32,
        wparam: usize,
        lparam: isize,
        time: u32,
        pt_x: i32,
        pt_y: i32,
    }

    #[link(name = "user32")]
    unsafe extern "system" {
        fn RegisterHotKey(window: isize, id: i32, modifiers: u32, vk: u32) -> i32;
        fn UnregisterHotKey(window: isize, id: i32) -> i32;
        fn GetMessageW(msg: *mut Msg, window: isize, first: u32, last: u32) -> i32;
        fn PeekMessageW(msg: *mut Msg, window: isize, first: u32, last: u32, remove: u32) -> i32;
        pub(super) fn PostThreadMessageW(thread: u32, message: u32, wparam: usize, lparam: isize) -> i32;
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentThreadId() -> u32;
    }

    /// Registers `chord` for this thread under `id`.
    pub(super) fn register(id: usize, chord: &Chord) -> bool {
        // SAFETY: no window, plain integers; the id is ours to pick.
        unsafe { RegisterHotKey(0, id as i32, MOD_CONTROL | MOD_ALT | MOD_SHIFT | MOD_NOREPEAT, chord.vk) != 0 }
    }

    pub(super) fn unregister(id: usize) {
        // SAFETY: as above; an id that was never registered is refused, harmlessly.
        unsafe { UnregisterHotKey(0, id as i32) };
    }

    /// The hotkey thread, for as long as this is kept. Dropping it takes the
    /// hotkeys back from Windows.
    pub struct Hotkeys {
        thread: u32,
        handle: Option<JoinHandle<()>>,
        registered: usize,
    }

    impl Hotkeys {
        /// Registers every chord in `chords` (indices into [`CHORDS`]) on a
        /// new thread and sends each press to `on_press`.
        pub fn start(chords: &[usize], on_press: impl Fn(Command) + Send + 'static) -> Option<Self> {
            let chords = chords.to_vec();
            let (ready_tx, ready) = mpsc::channel();
            let handle = std::thread::Builder::new()
                .name("hotkeys".to_owned())
                .spawn(move || {
                    let mut msg = Msg::default();
                    // SAFETY: an out-parameter; the call gives this thread the
                    // message queue that posted messages need to arrive in.
                    unsafe { PeekMessageW(&mut msg, 0, 0, 0, PM_NOREMOVE) };
                    let mut held = Vec::new();
                    for id in chords {
                        let chord = &CHORDS[id];
                        if register(id, chord) {
                            held.push(id);
                        } else {
                            eprintln!("ChronoDesk: {MODIFIERS}+{} is taken by another program", chord.key);
                        }
                    }
                    // SAFETY: no arguments.
                    let _ = ready_tx.send((unsafe { GetCurrentThreadId() }, held.len()));
                    // SAFETY: `msg` is a valid out-parameter; 0 is WM_QUIT, -1 an error.
                    while unsafe { GetMessageW(&mut msg, 0, 0, 0) } > 0 {
                        if msg.message == WM_HOTKEY
                            && let Some(cmd) = command(msg.wparam)
                        {
                            on_press(cmd);
                        }
                    }
                    for id in held {
                        unregister(id);
                    }
                })
                .ok()?;
            let (thread, registered) = ready.recv().ok()?;
            Some(Self { thread, handle: Some(handle), registered })
        }

        /// How many chords this got from Windows.
        pub fn registered(&self) -> usize {
            self.registered
        }

        #[cfg(test)]
        pub(super) fn thread(&self) -> u32 {
            self.thread
        }
    }

    impl Drop for Hotkeys {
        fn drop(&mut self) {
            // SAFETY: posting to a thread id we own; it ends its loop on WM_QUIT.
            unsafe { PostThreadMessageW(self.thread, WM_QUIT, 0, 0) };
            if let Some(handle) = self.handle.take() {
                let _ = handle.join();
            }
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use super::Command;

    pub struct Hotkeys;

    impl Hotkeys {
        pub fn start(_chords: &[usize], _on_press: impl Fn(Command) + Send + 'static) -> Option<Self> {
            None
        }

        pub fn registered(&self) -> usize {
            0
        }
    }
}

/// Every chord, sent to `tx`, with `wake` called after each so a sleeping
/// overlay draws the change.
pub fn start(tx: Sender<Command>, wake: impl Fn() + Send + 'static) -> Option<Hotkeys> {
    let all: Vec<usize> = (0..CHORDS.len()).collect();
    Hotkeys::start(&all, move |cmd| {
        let _ = tx.send(cmd);
        wake();
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tray::Command;

    #[test]
    fn every_chord_is_a_different_key_and_a_different_command() {
        for (i, a) in CHORDS.iter().enumerate() {
            for b in &CHORDS[i + 1..] {
                assert_ne!(a.vk, b.vk, "{} and {}", a.key, b.key);
                assert_ne!(a.command, b.command, "{} and {}", a.key, b.key);
            }
        }
    }

    /// The same keys as inside the overlay, so there is one set to learn.
    #[test]
    fn the_chords_mirror_the_overlays_own_keys() {
        let key = |k: &str| CHORDS.iter().find(|c| c.key == k).map(|c| c.command);
        assert_eq!(key("Space"), Some(Command::StartPause));
        assert_eq!(key("R"), Some(Command::Reset));
        for (digit, mode) in ["1", "2", "3", "4"].into_iter().zip(Mode::ALL) {
            assert_eq!(key(digit), Some(Command::SetMode(mode)));
        }
        assert_eq!(key("L"), Some(Command::ToggleLock));
    }

    #[test]
    fn an_unknown_hotkey_id_is_ignored() {
        assert_eq!(command(0), Some(Command::StartPause));
        assert_eq!(command(CHORDS.len()), None);
    }

    #[test]
    fn the_card_names_every_key() {
        for chord in CHORDS {
            let named = KEYS.split(' ').any(|k| k == chord.key || k.split_once('-').is_some_and(|(a, b)| (a..=b).contains(&chord.key)));
            assert!(named, "{} is missing from '{KEYS}'", chord.key);
        }
    }

    /// The thread's plumbing, without taking any key from the keyboard: it
    /// registers nothing, and a hotkey message is posted to it by hand.
    #[cfg(windows)]
    #[test]
    fn a_hotkey_message_reaches_the_app_and_dropping_ends_the_thread() {
        use std::sync::mpsc;
        use std::time::Duration;

        let (tx, rx) = mpsc::channel();
        let hotkeys = Hotkeys::start(&[], move |cmd| tx.send(cmd).unwrap()).expect("thread started");
        assert_eq!(hotkeys.registered(), 0);
        // SAFETY: posting to the test's own hotkey thread.
        let posted = unsafe { imp::PostThreadMessageW(hotkeys.thread(), imp::WM_HOTKEY, 2, 0) };
        assert_ne!(posted, 0);
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)), Ok(Command::ToggleLock));
        drop(hotkeys);
        assert!(rx.recv_timeout(Duration::from_millis(100)).is_err(), "the sender went with the thread");
    }

    /// Registration itself, on a key no keyboard has, so a test run never
    /// takes a chord someone might press.
    #[cfg(windows)]
    #[test]
    fn a_chord_can_be_registered_and_given_back() {
        const VK_F24: u32 = 0x87;
        let chord = Chord { vk: VK_F24, key: "F24", command: Command::StartPause };
        let id = 0xBEEF;
        assert!(imp::register(id, &chord), "Ctrl+Alt+Shift+F24 was free");
        assert!(!imp::register(id + 1, &chord), "a chord already held is refused");
        imp::unregister(id);
        assert!(imp::register(id + 1, &chord), "and is free again once given back");
        imp::unregister(id + 1);
    }
}
