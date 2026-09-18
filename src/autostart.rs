//! "Start with Windows": the per-user `Run` key.
//!
//! The registry is the only record. A mirrored config field could drift from
//! it: the user can remove the entry in Settings, switch it off in Task
//! Manager, or move the exe. So the menu's check mark is read from the
//! registry, and means exactly "this exe will be started at the next login".
//!
//! Task Manager does not delete a `Run` value when a startup app is switched
//! off; it writes a flag under `Explorer\StartupApproved\Run`. Both keys are
//! read, and switching autostart on clears that flag.

use std::io;
use std::path::Path;

const VALUE: &str = "ChronoDesk";
const SYSTEM_ROOT: &str = r"Software\Microsoft\Windows\CurrentVersion";

pub struct Autostart {
    run_key: String,
    approved_key: String,
}

impl Autostart {
    /// The keys Windows itself reads at login.
    pub fn system() -> Self {
        // Test runs point this at a scratch key, so a script can flip the
        // menu item without registering a build directory to start at login.
        #[cfg(feature = "instrument")]
        if let Some(root) = std::env::var("CHRONODESK_RUN_KEY").ok().filter(|root| !root.is_empty()) {
            return Self::under(&root);
        }
        Self {
            run_key: format!(r"{SYSTEM_ROOT}\Run"),
            approved_key: format!(r"{SYSTEM_ROOT}\Explorer\StartupApproved\Run"),
        }
    }

    #[cfg(any(test, feature = "instrument"))]
    fn under(root: &str) -> Self {
        Self { run_key: format!(r"{root}\Run"), approved_key: format!(r"{root}\StartupApproved\Run") }
    }

    /// Whether the platform has anything to toggle.
    pub fn available() -> bool {
        cfg!(windows)
    }

    /// True when `exe` is what will be started at the next login.
    pub fn enabled(&self, exe: &Path) -> bool {
        let registered = registry::read_string(&self.run_key, VALUE).is_some_and(|command| points_at(&command, exe));
        registered && approved(&registry::read_bytes(&self.approved_key, VALUE).unwrap_or_default())
    }

    pub fn set(&self, exe: &Path, on: bool) -> io::Result<()> {
        if on {
            registry::write_string(&self.run_key, VALUE, &command_line(exe))?;
        } else {
            registry::delete_value(&self.run_key, VALUE)?;
        }
        // Either way a leftover "disabled" flag has no business staying: it
        // would silently veto the entry just written, or outlive the one removed.
        registry::delete_value(&self.approved_key, VALUE)
    }
}

/// The command stored in the registry: the path quoted, since Windows splits
/// an unquoted one at its first space.
fn command_line(exe: &Path) -> String {
    format!("\"{}\"", exe.display())
}

/// Whether a stored command starts `exe`, whatever its quoting, case or
/// trailing arguments.
fn points_at(command: &str, exe: &Path) -> bool {
    let command = command.trim();
    let program = match command.strip_prefix('"') {
        Some(rest) => rest.split('"').next().unwrap_or_default(),
        // Unquoted: a path with spaces is ambiguous, so take it whole first.
        None if same_path(command, exe) => return true,
        None => command.split_whitespace().next().unwrap_or_default(),
    };
    same_path(program, exe)
}

fn same_path(text: &str, exe: &Path) -> bool {
    let fold = |s: &str| s.replace('/', "\\").to_lowercase();
    !text.is_empty() && fold(text) == fold(&exe.to_string_lossy())
}

/// Reads a `StartupApproved` flag. No value means nobody has objected; in the
/// value Task Manager writes, an odd first byte (`03`, `07`) means "disabled".
fn approved(flag: &[u8]) -> bool {
    flag.first().is_none_or(|byte| byte % 2 == 0)
}

#[cfg(windows)]
mod registry {
    use std::io;

    use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
    use windows_sys::Win32::System::Registry::{
        HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_BINARY, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegGetValueW,
        RegSetKeyValueW,
    };

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// The raw value, or `None` when it is missing or of another type.
    fn read(key: &str, value: &str, kind: u32) -> Option<Vec<u8>> {
        let (key, value) = (wide(key), wide(value));
        let mut len = 0u32;
        // SAFETY: a null buffer with a length pointer asks for the size only.
        let status = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                value.as_ptr(),
                kind,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut len,
            )
        };
        if status != ERROR_SUCCESS {
            return None;
        }
        let mut data = vec![0u8; len as usize];
        // SAFETY: `data` is `len` bytes long, which is what the call is told.
        let status = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                value.as_ptr(),
                kind,
                std::ptr::null_mut(),
                data.as_mut_ptr().cast(),
                &mut len,
            )
        };
        (status == ERROR_SUCCESS).then(|| {
            data.truncate(len as usize);
            data
        })
    }

    pub fn read_string(key: &str, value: &str) -> Option<String> {
        let bytes = read(key, value, RRF_RT_REG_SZ)?;
        let units: Vec<u16> = bytes.as_chunks::<2>().0.iter().map(|pair| u16::from_le_bytes(*pair)).collect();
        let end = units.iter().position(|unit| *unit == 0).unwrap_or(units.len());
        Some(String::from_utf16_lossy(&units[..end]))
    }

    pub fn read_bytes(key: &str, value: &str) -> Option<Vec<u8>> {
        read(key, value, RRF_RT_REG_BINARY)
    }

    /// Creates the key if it is not there yet.
    pub fn write_string(key: &str, value: &str, text: &str) -> io::Result<()> {
        let (key, value, text) = (wide(key), wide(value), wide(text));
        // SAFETY: all three are NUL-terminated; the length is in bytes and
        // includes the terminator, as `REG_SZ` expects.
        let status = unsafe {
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                value.as_ptr(),
                REG_SZ,
                text.as_ptr().cast(),
                (text.len() * 2) as u32,
            )
        };
        if status == ERROR_SUCCESS { Ok(()) } else { Err(io::Error::from_raw_os_error(status as i32)) }
    }

    /// Deleting what is not there is a success.
    pub fn delete_value(key: &str, value: &str) -> io::Result<()> {
        let (key, value) = (wide(key), wide(value));
        // SAFETY: both strings are NUL-terminated and outlive the call.
        let status = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), value.as_ptr()) };
        match status {
            ERROR_SUCCESS | ERROR_FILE_NOT_FOUND => Ok(()),
            other => Err(io::Error::from_raw_os_error(other as i32)),
        }
    }

    #[cfg(test)]
    pub fn write_bytes(key: &str, value: &str, bytes: &[u8]) {
        use windows_sys::Win32::System::Registry::REG_BINARY;
        let (key, value) = (wide(key), wide(value));
        // SAFETY: `bytes` is valid for its own length.
        let status = unsafe {
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                value.as_ptr(),
                REG_BINARY,
                bytes.as_ptr().cast(),
                bytes.len() as u32,
            )
        };
        assert_eq!(status, ERROR_SUCCESS);
    }

    #[cfg(test)]
    pub fn delete_tree(key: &str) {
        use windows_sys::Win32::System::Registry::RegDeleteTreeW;
        let key = wide(key);
        // SAFETY: the string is NUL-terminated and outlives the calls.
        unsafe {
            RegDeleteTreeW(HKEY_CURRENT_USER, key.as_ptr());
            windows_sys::Win32::System::Registry::RegDeleteKeyW(HKEY_CURRENT_USER, key.as_ptr());
        }
    }
}

#[cfg(not(windows))]
mod registry {
    use std::io;

    pub fn read_string(_key: &str, _value: &str) -> Option<String> {
        None
    }

    pub fn read_bytes(_key: &str, _value: &str) -> Option<Vec<u8>> {
        None
    }

    pub fn write_string(_key: &str, _value: &str, _text: &str) -> io::Result<()> {
        Err(io::ErrorKind::Unsupported.into())
    }

    pub fn delete_value(_key: &str, _value: &str) -> io::Result<()> {
        Err(io::ErrorKind::Unsupported.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXE: &str = r"C:\Program Files\ChronoDesk\chronodesk.exe";

    #[test]
    fn the_stored_command_is_quoted() {
        assert_eq!(command_line(Path::new(EXE)), r#""C:\Program Files\ChronoDesk\chronodesk.exe""#);
    }

    #[test]
    fn a_command_is_recognised_however_it_is_written() {
        let exe = Path::new(EXE);
        assert!(points_at(&command_line(exe), exe));
        assert!(points_at(r#""c:/program files/chronodesk/CHRONODESK.EXE" --minimized"#, exe));
        assert!(points_at(EXE, exe), "unquoted, spaces and all");
        assert!(points_at(r"C:\tools\chronodesk.exe --flag", Path::new(r"C:\tools\chronodesk.exe")));
    }

    #[test]
    fn another_exe_is_not_ours() {
        let exe = Path::new(EXE);
        assert!(!points_at(r#""C:\old\chronodesk.exe""#, exe), "a copy left behind somewhere else");
        assert!(!points_at("", exe));
        assert!(!points_at("\"", exe));
    }

    #[test]
    fn task_manager_flags() {
        assert!(approved(&[]), "no flag: nobody objected");
        assert!(approved(&[2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]));
        assert!(approved(&[6, 0, 0, 0]));
        assert!(!approved(&[3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]));
        assert!(!approved(&[7, 0, 0, 0]));
    }

    /// A scratch key under HKCU, never the real `Run` key, removed on drop.
    #[cfg(windows)]
    struct Scratch(String);

    #[cfg(windows)]
    impl Scratch {
        fn new(tag: &str) -> Self {
            Self(format!(r"Software\ChronoDesk-test-{}-{tag}", std::process::id()))
        }
    }

    #[cfg(windows)]
    impl Drop for Scratch {
        fn drop(&mut self) {
            registry::delete_tree(&self.0);
        }
    }

    #[cfg(windows)]
    #[test]
    fn switching_on_and_off_round_trips_through_the_registry() {
        let scratch = Scratch::new("roundtrip");
        let (autostart, exe) = (Autostart::under(&scratch.0), Path::new(EXE));
        assert!(!autostart.enabled(exe));
        autostart.set(exe, true).unwrap();
        assert!(autostart.enabled(exe));
        assert_eq!(registry::read_string(&autostart.run_key, VALUE).as_deref(), Some(command_line(exe).as_str()));
        autostart.set(exe, false).unwrap();
        assert!(!autostart.enabled(exe));
        autostart.set(exe, false).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn an_entry_for_a_moved_exe_is_off_and_switching_on_repoints_it() {
        let scratch = Scratch::new("moved");
        let (autostart, exe) = (Autostart::under(&scratch.0), Path::new(EXE));
        autostart.set(Path::new(r"C:\old\chronodesk.exe"), true).unwrap();
        assert!(!autostart.enabled(exe));
        autostart.set(exe, true).unwrap();
        assert!(autostart.enabled(exe));
    }

    #[cfg(windows)]
    #[test]
    fn disabled_in_task_manager_is_off_and_switching_on_clears_the_veto() {
        let scratch = Scratch::new("veto");
        let (autostart, exe) = (Autostart::under(&scratch.0), Path::new(EXE));
        autostart.set(exe, true).unwrap();
        registry::write_bytes(&autostart.approved_key, VALUE, &[3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        assert!(!autostart.enabled(exe));
        autostart.set(exe, true).unwrap();
        assert!(autostart.enabled(exe));
        assert!(registry::read_bytes(&autostart.approved_key, VALUE).is_none());
    }
}
