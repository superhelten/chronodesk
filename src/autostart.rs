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

use crate::registry;

const VALUE: &str = "ChronoDesk";
const SYSTEM_ROOT: &str = r"Software\Microsoft\Windows\CurrentVersion";

pub struct Autostart {
    run_key: String,
    approved_key: String,
}

/// Test runs point this at a scratch key, so a script can flip the menu item,
/// or run the installer, without touching what Windows reads at login.
/// Instrument builds only: a release build ignores the variable.
fn scratch_root() -> Option<String> {
    #[cfg(feature = "instrument")]
    return std::env::var("CHRONODESK_RUN_KEY").ok().filter(|root| !root.is_empty());
    #[cfg(not(feature = "instrument"))]
    None
}

/// The key the app's other registrations (the uninstall entry) live under.
pub fn registry_root() -> String {
    scratch_root().unwrap_or_else(|| SYSTEM_ROOT.to_owned())
}

impl Autostart {
    /// The keys Windows itself reads at login.
    pub fn system() -> Self {
        if let Some(root) = scratch_root() {
            return Self {
                run_key: format!(r"{root}\Run"),
                approved_key: format!(r"{root}\StartupApproved\Run"),
            };
        }
        Self {
            run_key: format!(r"{SYSTEM_ROOT}\Run"),
            approved_key: format!(r"{SYSTEM_ROOT}\Explorer\StartupApproved\Run"),
        }
    }

    #[cfg(test)]
    pub(crate) fn under(root: &str) -> Self {
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

    /// For the installer: an entry that exists, wherever it points, is made to
    /// name `exe`. Nothing else is touched. No entry stays no entry, and a
    /// Task Manager veto stays a veto, since neither is the installer's call.
    /// True when the entry was rewritten.
    pub fn repoint(&self, exe: &Path) -> io::Result<bool> {
        match registry::read_string(&self.run_key, VALUE) {
            Some(command) if !points_at(&command, exe) => {
                registry::write_string(&self.run_key, VALUE, &command_line(exe))?;
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    /// For the uninstaller: removes the entry if it starts `exe`, and leaves
    /// one that belongs to some other copy alone.
    pub fn remove_if_ours(&self, exe: &Path) -> io::Result<()> {
        match registry::read_string(&self.run_key, VALUE) {
            Some(command) if points_at(&command, exe) => self.set(exe, false),
            _ => Ok(()),
        }
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
    fn repointing_moves_an_entry_but_never_creates_one_or_lifts_a_veto() {
        let scratch = Scratch::new("repoint");
        let (autostart, exe) = (Autostart::under(&scratch.0), Path::new(EXE));
        assert!(!autostart.repoint(exe).unwrap());
        assert!(registry::read_string(&autostart.run_key, VALUE).is_none(), "no entry stays no entry");

        autostart.set(Path::new(r"C:\old\chronodesk.exe"), true).unwrap();
        registry::write_bytes(&autostart.approved_key, VALUE, &[3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        assert!(autostart.repoint(exe).unwrap());
        assert_eq!(registry::read_string(&autostart.run_key, VALUE).as_deref(), Some(command_line(exe).as_str()));
        assert!(!autostart.enabled(exe), "switched off in Task Manager stays switched off");
        assert!(!autostart.repoint(exe).unwrap(), "already there");

        autostart.remove_if_ours(Path::new(r"C:\elsewhere\chronodesk.exe")).unwrap();
        assert!(registry::read_string(&autostart.run_key, VALUE).is_some(), "not ours to remove");
        autostart.remove_if_ours(exe).unwrap();
        assert!(registry::read_string(&autostart.run_key, VALUE).is_none());
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
