//! A word from a later launch to the overlay that is already running.
//!
//! The single-instance guard makes a second launch step aside, which on its
//! own looks like nothing happened: the overlay the user was looking for is
//! still wherever it was, perhaps behind a full-screen window or on a screen
//! that is switched off. So before it leaves, the second launch asks the first
//! to show itself. The installer uses the same channel to ask a running overlay
//! to quit before its exe is replaced.
//!
//! Two named events per config file, next to the guard's mutex and keyed the
//! same way. An event carries no data, cannot be left behind by a crash, and
//! costs the listener nothing while it waits: no socket, no polling, no frames.

use std::path::Path;

use crate::instance;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Signal {
    /// Come to the front and make yourself seen.
    Show,
    /// Save and exit.
    Quit,
}

impl Signal {
    const ALL: [Self; 2] = [Self::Show, Self::Quit];

    fn suffix(self) -> &'static str {
        match self {
            Self::Show => "show",
            Self::Quit => "quit",
        }
    }
}

/// The event's name for a config file: the guard's name with the signal added.
fn name_for(config: &Path, signal: Signal) -> String {
    format!("{}-{}", instance::name_for(config), signal.suffix())
}

/// Starts listening on behalf of the instance that owns `config`. `deliver`
/// runs on the listener's thread, once per signal.
pub fn listen(config: &Path, deliver: impl Fn(Signal) + Send + 'static) {
    let names = Signal::ALL.map(|signal| name_for(config, signal));
    imp::listen(names, move |index| deliver(Signal::ALL[index]));
}

/// True when an instance was there to hear it.
pub fn send(config: &Path, signal: Signal) -> bool {
    imp::send(&name_for(config, signal))
}

#[cfg(windows)]
mod imp {
    use windows_sys::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0};
    use windows_sys::Win32::System::Threading::{
        CreateEventW, EVENT_MODIFY_STATE, INFINITE, OpenEventW, SetEvent, WaitForMultipleObjects,
    };

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

    pub fn listen(names: [String; 2], deliver: impl Fn(usize) + Send + 'static) {
        // Auto-reset, initially unset: one `SetEvent` wakes the wait once.
        let handles = names.map(|name| {
            let name = wide(&name);
            // SAFETY: `name` is NUL-terminated and outlives the call.
            unsafe { CreateEventW(std::ptr::null(), 0, 0, name.as_ptr()) }
        });
        if handles.contains(&0) {
            // The overlay works without it; a second launch just leaves quietly.
            eprintln!("ChronoDesk: instance signals unavailable");
            return;
        }
        // The handles live as long as the process, like the thread waiting on them.
        std::thread::spawn(move || {
            loop {
                // SAFETY: `handles` holds two valid event handles owned by this thread.
                let woken = unsafe { WaitForMultipleObjects(2, handles.as_ptr(), 0, INFINITE) };
                match woken.checked_sub(WAIT_OBJECT_0) {
                    Some(index @ 0..2) => deliver(index as usize),
                    _ => return,
                }
            }
        });
    }

    pub fn send(name: &str) -> bool {
        let name = wide(name);
        // SAFETY: `name` is NUL-terminated; the handle is closed before returning.
        unsafe {
            let handle = OpenEventW(EVENT_MODIFY_STATE, 0, name.as_ptr());
            if handle == 0 {
                return false;
            }
            let set = SetEvent(handle) != 0;
            CloseHandle(handle);
            set
        }
    }
}

#[cfg(not(windows))]
mod imp {
    pub fn listen(_names: [String; 2], _deliver: impl Fn(usize) + Send + 'static) {}

    pub fn send(_name: &str) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_signal_has_its_own_name_beside_the_guard() {
        let config = Path::new(r"C:\a\app.ron");
        let (show, quit) = (name_for(config, Signal::Show), name_for(config, Signal::Quit));
        assert_ne!(show, quit);
        assert!(show.starts_with(&instance::name_for(config)));
        assert_ne!(show, name_for(Path::new(r"C:\b\app.ron"), Signal::Show));
    }

    #[cfg(windows)]
    #[test]
    fn a_signal_reaches_the_listener_and_nobody_else() {
        use std::sync::mpsc;
        use std::time::Duration;

        let config = std::env::temp_dir().join(format!("chronodesk-signal-test-{}", std::process::id())).join("app.ron");
        assert!(!send(&config, Signal::Show), "nobody is listening yet");

        let (tx, rx) = mpsc::channel();
        listen(&config, move |signal| tx.send(signal).unwrap());
        assert!(send(&config, Signal::Show));
        assert_eq!(rx.recv_timeout(Duration::from_secs(2)), Ok(Signal::Show));
        assert!(send(&config, Signal::Quit));
        assert_eq!(rx.recv_timeout(Duration::from_secs(2)), Ok(Signal::Quit));
        assert!(rx.recv_timeout(Duration::from_millis(100)).is_err(), "one signal, one delivery");
    }
}
