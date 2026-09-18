//! One overlay per config file.
//!
//! Two copies sharing one `app.ron` would each save their own idea of the
//! settings and the window position, and the last one to exit would win; the
//! user would also get two tray icons that look the same. So a second launch
//! against the same file steps aside.
//!
//! The guard is keyed on the config path rather than being global, because
//! the test scripts run their own instances under `CHRONODESK_CONFIG` while
//! the user's overlay keeps running: different file, different guard.

use std::path::Path;

/// Held for the life of the process; the OS releases it on exit, crash included.
pub struct Guard {
    #[cfg(windows)]
    _handle: isize,
}

#[cfg(windows)]
impl Drop for Guard {
    fn drop(&mut self) {
        if self._handle != 0 {
            // SAFETY: the handle came from `CreateMutexW` and is closed once.
            unsafe { windows_sys::Win32::Foundation::CloseHandle(self._handle) };
        }
    }
}

/// `None` when another instance already owns this config file.
pub fn acquire(config: &Path) -> Option<Guard> {
    acquire_named(&name_for(config))
}

/// The name of the guard for a config file. Stable across builds (the hash is
/// spelled out here, not taken from `std`), so a debug and a release build
/// still see each other.
pub fn name_for(config: &Path) -> String {
    // The file may not exist yet on a first launch, so this cannot lean on
    // `canonicalize`; making it absolute and folding case and separators
    // covers the ways one Windows path gets spelled.
    let absolute = std::path::absolute(config).unwrap_or_else(|_| config.to_path_buf());
    let folded = absolute.to_string_lossy().replace('/', "\\").to_lowercase();
    // `Local\` scopes the name to the login session: another user on the same
    // machine has their own overlay.
    format!("Local\\ChronoDesk-{:016x}", fnv1a(folded.as_bytes()))
}

fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| (hash ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3))
}

#[cfg(windows)]
fn acquire_named(name: &str) -> Option<Guard> {
    use windows_sys::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError};
    use windows_sys::Win32::System::Threading::CreateMutexW;

    let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
    // SAFETY: `wide` is NUL-terminated and outlives the call; a null security
    // descriptor asks for the default one.
    let handle = unsafe { CreateMutexW(std::ptr::null(), 0, wide.as_ptr()) };
    if handle == 0 {
        // Not being able to create the guard is no reason to refuse to start.
        eprintln!("ChronoDesk: single-instance guard unavailable");
        return Some(Guard { _handle: 0 });
    }
    // SAFETY: reads the calling thread's last error, set by the call above.
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        // SAFETY: `handle` is a valid handle this function just received.
        unsafe { CloseHandle(handle) };
        return None;
    }
    Some(Guard { _handle: handle })
}

/// Elsewhere nothing is enforced yet: the macOS path is untested, and a
/// stale lock file would be worse than a second clock.
#[cfg(not(windows))]
fn acquire_named(_name: &str) -> Option<Guard> {
    Some(Guard {})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_file_has_one_name_however_it_is_spelled() {
        let a = name_for(Path::new(r"C:\Users\me\AppData\Roaming\chronodesk\data\app.ron"));
        let b = name_for(Path::new(r"c:/users/ME/appdata/roaming/chronodesk/data/APP.RON"));
        assert_eq!(a, b);
        assert!(a.starts_with("Local\\ChronoDesk-"));
        // A kernel object name may not contain a backslash after the namespace.
        assert!(!a["Local\\".len()..].contains('\\'));
    }

    #[test]
    fn a_relative_path_names_the_same_file_as_its_absolute_form() {
        let relative = Path::new("scratch").join("app.ron");
        let absolute = std::env::current_dir().unwrap().join(&relative);
        assert_eq!(name_for(&relative), name_for(&absolute));
    }

    #[test]
    fn different_files_have_different_names() {
        assert_ne!(name_for(Path::new(r"C:\a\app.ron")), name_for(Path::new(r"C:\b\app.ron")));
    }

    #[test]
    fn the_hash_is_pinned() {
        // FNV-1a test vectors: a change here would make two builds blind to each other.
        assert_eq!(fnv1a(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a(b"a"), 0xaf63_dc4c_8601_ec8c);
    }

    #[cfg(windows)]
    #[test]
    fn a_second_holder_is_turned_away_until_the_first_lets_go() {
        let name = format!("Local\\ChronoDesk-test-{}", std::process::id());
        let first = acquire_named(&name).expect("nobody holds it yet");
        assert!(acquire_named(&name).is_none());
        drop(first);
        assert!(acquire_named(&name).is_some(), "released with its holder");
    }
}
