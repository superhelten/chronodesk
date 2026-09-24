//! Installing ChronoDesk is the app copying itself somewhere permanent.
//!
//! There is no separate installer to build, sign or keep in step with the app:
//! the download *is* the app. Named `ChronoDesk-Setup.exe` (or run with
//! `--install`) it copies itself to `%LOCALAPPDATA%\ChronoDesk\chronodesk.exe`,
//! registers that copy with Windows and starts it. Everything is per user, so
//! nothing asks for elevation:
//!
//! * the exe and its icon file in `%LOCALAPPDATA%\ChronoDesk`;
//! * a Start menu shortcut;
//! * an entry under *Installed apps*, whose uninstall command is the installed
//!   exe with `--uninstall`;
//! * a "Start with Windows" entry that already exists (pointing at a build
//!   directory, a download folder) is repointed at the installed copy. One that
//!   does not exist is not created: starting at login stays the user's choice,
//!   made in the menu, and from the installed copy it names the fixed path.
//!
//! Running the setup again is safe at any time. With the same version installed
//! nothing is copied and the running overlay is simply asked to show itself
//! (not with `--quiet`, which then does nothing at all);
//! with another version the running overlay is asked to quit, the exe is
//! replaced and the new one started; run with `--quiet` while the overlay is
//! closed, it leaves it closed. A running exe cannot be overwritten on
//! Windows but it can be renamed, so the old one is moved aside first and the
//! swap works even if some other copy of it is still running.
//!
//! Settings are not part of the installation: they stay in
//! `%APPDATA%\chronodesk` through upgrades and after an uninstall.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::autostart::Autostart;
use crate::signal::{self, Signal};
use crate::{icon, instance, registry};

const EXE_NAME: &str = "chronodesk.exe";
const ICON_NAME: &str = "chronodesk.ico";
/// The exe a running overlay was started from, until it can be deleted.
const OLD_NAME: &str = "chronodesk.exe.old";
const NEW_NAME: &str = "chronodesk.exe.new";
/// How long a running overlay gets to save and exit before its exe is replaced.
const QUIT_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Install,
    Uninstall,
}

/// What this launch was asked to be. A file called `…setup….exe` installs when
/// it is double-clicked; the flags are for everything else.
pub fn requested(args: &[String], exe: Option<&Path>) -> Option<Action> {
    if args.iter().any(|arg| arg == "--uninstall") {
        return Some(Action::Uninstall);
    }
    let named_setup = exe
        .and_then(Path::file_stem)
        .is_some_and(|stem| stem.to_string_lossy().to_lowercase().contains("setup"));
    (named_setup || args.iter().any(|arg| arg == "--install")).then_some(Action::Install)
}

/// Where an installation lives.
pub struct Layout {
    pub dir: PathBuf,
    /// The Start menu shortcut, where the platform has one.
    shortcut: Option<PathBuf>,
    uninstall_key: String,
    autostart: Autostart,
}

impl Layout {
    pub fn system() -> Option<Self> {
        // A test run installs into a scratch directory and a scratch registry
        // key, so it can be as thorough as it likes. Instrument builds only: a
        // release build installs where Windows expects it, whatever is set.
        #[cfg(feature = "instrument")]
        if let Some(root) = std::env::var_os("CHRONODESK_INSTALL_ROOT").filter(|root| !root.is_empty()) {
            let root = PathBuf::from(root);
            return Some(Self {
                dir: root.join("app"),
                shortcut: Some(root.join("start menu").join("ChronoDesk.lnk")),
                uninstall_key: format!(r"{}\Uninstall\ChronoDesk", crate::autostart::registry_root()),
                autostart: Autostart::system(),
            });
        }
        let local = PathBuf::from(std::env::var_os("LOCALAPPDATA")?);
        let programs = std::env::var_os("APPDATA")
            .map(|roaming| PathBuf::from(roaming).join(r"Microsoft\Windows\Start Menu\Programs"));
        Some(Self {
            dir: local.join("ChronoDesk"),
            shortcut: programs.map(|dir| dir.join("ChronoDesk.lnk")),
            uninstall_key: format!(r"{}\Uninstall\ChronoDesk", crate::autostart::registry_root()),
            autostart: Autostart::system(),
        })
    }

    pub fn exe(&self) -> PathBuf {
        self.dir.join(EXE_NAME)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Installed {
    /// The exe was copied into place.
    Copied,
    /// The same exe was there already; only the registration was refreshed.
    AlreadyCurrent,
}

/// Copies `source` into place and registers it. `config` names the overlay
/// that has to let go of the old exe first.
pub fn install(layout: &Layout, source: &Path, config: Option<&Path>) -> io::Result<Installed> {
    fs::create_dir_all(&layout.dir)?;
    let target = layout.exe();
    let outcome = if same_file(source, &target) || same_contents(source, &target) {
        Installed::AlreadyCurrent
    } else {
        if let Some(config) = config {
            quit_running(config, QUIT_TIMEOUT);
        }
        replace_exe(source, &layout.dir)?;
        Installed::Copied
    };

    let icon = layout.dir.join(ICON_NAME);
    fs::write(&icon, icon::ico(&[16, 24, 32, 48, 64]))?;
    if let Some(shortcut) = &layout.shortcut {
        if let Some(dir) = shortcut.parent() {
            fs::create_dir_all(dir)?;
        }
        shortcut::create(shortcut, &target, &icon, "Transparent desktop clock, stopwatch, timer and market board")?;
    }
    register(layout, &target, &icon)?;
    layout.autostart.repoint(&target)?;
    Ok(outcome)
}

/// Removes what [`install`] put there. The directory itself can only go once
/// the exe in it has exited, so when that exe is the one running this, the
/// last step is left to a command that outlives it (see [`run`]).
pub fn uninstall(layout: &Layout, config: Option<&Path>) -> io::Result<()> {
    uninstall_waiting(layout, config, QUIT_TIMEOUT)
}

fn uninstall_waiting(layout: &Layout, config: Option<&Path>, timeout: Duration) -> io::Result<()> {
    // An overlay that stays up keeps its exe, which the clean-up after exit
    // could then not delete; with the entry gone there would be nothing left
    // to remove it with. So nothing is touched until it is gone.
    if let Some(config) = config
        && !quit_running(config, timeout)
    {
        return Err(io::Error::other("ChronoDesk is still running; quit it from its tray icon first"));
    }
    // Every step is tried, whatever happened to the one before.
    let steps = [
        layout.autostart.remove_if_ours(&layout.exe()),
        layout.shortcut.as_deref().map_or(Ok(()), remove_if_there),
        remove_if_there(&layout.dir.join(ICON_NAME)),
    ];
    // Left by an upgrade while the exe it replaced was still running. Not worth
    // failing over: the clean-up after exit takes whatever is still here.
    let _ = remove_if_there(&layout.dir.join(NEW_NAME));
    remove_leftovers(&layout.dir);
    steps.into_iter().collect::<io::Result<()>>()?;
    // Last, so that anything that could not be removed can still be
    // uninstalled again from Settings.
    registry::delete_tree(&layout.uninstall_key)
}

fn remove_if_there(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Err(err) if err.kind() != io::ErrorKind::NotFound => Err(err),
        _ => Ok(()),
    }
}

/// Asks the overlay on `config` to exit and waits until its guard is free.
/// False when it is still there afterwards: a build from before it listened.
fn quit_running(config: &Path, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if instance::acquire(config).is_some() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        // Asked again each time: an overlay that is only just starting may
        // hold its guard a moment before it can hear anything.
        signal::send(config, Signal::Quit);
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// New exe in, old exe out of the way. At every point there is a complete
/// `chronodesk.exe` or none, never half of one.
fn replace_exe(source: &Path, dir: &Path) -> io::Result<()> {
    let (target, new) = (dir.join(EXE_NAME), dir.join(NEW_NAME));
    fs::copy(source, &new)?;
    // Left by earlier upgrades if their exe was still running then. One that
    // still is cannot be removed or replaced, so the exe steps aside to a name
    // that is free.
    remove_leftovers(dir);
    let old = (1..)
        .map(|n| if n == 1 { dir.join(OLD_NAME) } else { dir.join(format!("{OLD_NAME}{n}")) })
        .find(|path| !path.exists())
        .expect("a free name");
    if target.exists() {
        fs::rename(&target, &old)?;
    }
    if let Err(err) = fs::rename(&new, &target) {
        let _ = fs::rename(&old, &target);
        let _ = fs::remove_file(&new);
        return Err(err);
    }
    // Fails while the old exe is still running; the next install or the
    // uninstaller picks it up.
    let _ = fs::remove_file(&old);
    Ok(())
}

/// The exes earlier upgrades moved aside: `chronodesk.exe.old`, and
/// `chronodesk.exe.old2` and on when one of those was still running.
fn leftovers(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(dir) else { return Vec::new() };
    let mut found: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().starts_with(OLD_NAME))
        .map(|entry| entry.path())
        .collect();
    found.sort();
    found
}

fn remove_leftovers(dir: &Path) {
    for path in leftovers(dir) {
        let _ = fs::remove_file(path);
    }
}

fn same_file(a: &Path, b: &Path) -> bool {
    matches!((fs::canonicalize(a), fs::canonicalize(b)), (Ok(a), Ok(b)) if a == b)
}

fn same_contents(a: &Path, b: &Path) -> bool {
    let same_len = matches!((fs::metadata(a), fs::metadata(b)), (Ok(a), Ok(b)) if a.len() == b.len());
    same_len && matches!((fs::read(a), fs::read(b)), (Ok(a), Ok(b)) if a == b)
}

/// The entry under Settings → Apps → Installed apps.
fn register(layout: &Layout, exe: &Path, icon: &Path) -> io::Result<()> {
    let key = &layout.uninstall_key;
    let size_kb = fs::metadata(exe).map_or(0, |meta| meta.len().div_ceil(1024) as u32);
    registry::write_string(key, "DisplayName", "ChronoDesk")?;
    registry::write_string(key, "DisplayVersion", env!("CARGO_PKG_VERSION"))?;
    registry::write_string(key, "Publisher", "ChronoDesk")?;
    registry::write_string(key, "DisplayIcon", &icon.display().to_string())?;
    registry::write_string(key, "InstallLocation", &layout.dir.display().to_string())?;
    registry::write_string(key, "UninstallString", &format!("\"{}\" --uninstall", exe.display()))?;
    registry::write_string(key, "QuietUninstallString", &format!("\"{}\" --uninstall --quiet", exe.display()))?;
    registry::write_dword(key, "EstimatedSize", size_kb)?;
    registry::write_dword(key, "NoModify", 1)?;
    registry::write_dword(key, "NoRepair", 1)
}

/// Carries out `action` for this process and returns its exit code. `--quiet`
/// keeps it from putting up a message box, for scripts and for Windows' own
/// quiet uninstall.
pub fn run(action: Action, args: &[String], config: Option<&Path>) -> i32 {
    let quiet = args.iter().any(|arg| arg == "--quiet");
    let report = |text: &str, failed: bool| {
        if failed {
            eprintln!("ChronoDesk: {text}");
        }
        if !quiet {
            message(text, failed);
        }
    };
    let (Some(layout), Ok(current)) = (Layout::system(), std::env::current_exe()) else {
        report("This system has no per-user application folder to install into.", true);
        return 1;
    };
    match action {
        Action::Install => {
            let upgrade = layout.exe().exists();
            let was_running = config.is_some_and(|config| instance::acquire(config).is_none());
            match install(&layout, &current, config) {
                // Nothing was replaced and the overlay is running the same exe:
                // a quiet run has nothing to add, and asking it to show itself
                // would take the focus from whoever is using the machine.
                Ok(Installed::AlreadyCurrent) if quiet && was_running => 0,
                Ok(_) => {
                    // An overlay that neither quit nor answers is a build from
                    // before it listened; the new exe would only step aside for it.
                    let deaf = config.is_some_and(|config| {
                        instance::acquire(config).is_none() && !signal::send(config, Signal::Show)
                    });
                    if deaf {
                        report(
                            "ChronoDesk was installed, but an older copy is still running. \
                             Quit it from its tray icon, then start ChronoDesk from the Start menu.",
                            false,
                        );
                        return 0;
                    }
                    // A quiet upgrade puts back what it found: an overlay the user
                    // had closed stays closed. A first installation starts it,
                    // since seeing it is what installing it was for.
                    if quiet && upgrade && !was_running {
                        return 0;
                    }
                    // The installed copy takes it from here: it becomes the overlay,
                    // or finds one running and asks it to show itself. Anything else
                    // on the command line was meant for it.
                    let passed_on = args.iter().filter(|arg| !matches!(arg.as_str(), "--install" | "--quiet"));
                    // Started in its own folder: it runs for days, and would
                    // otherwise hold on to wherever the setup was run from, a USB
                    // stick or a Downloads folder the user wants to delete.
                    let started = std::process::Command::new(layout.exe()).args(passed_on).current_dir(&layout.dir).spawn();
                    if let Err(err) = started {
                        report(&format!("ChronoDesk was installed but could not be started: {err}"), true);
                        return 1;
                    }
                    0
                }
                Err(err) => {
                    report(&format!("ChronoDesk could not be installed in {}: {err}", layout.dir.display()), true);
                    1
                }
            }
        }
        Action::Uninstall => match uninstall(&layout, config) {
            Ok(()) => {
                // The message first: it blocks until it is dismissed, and the
                // exe cannot be deleted for as long as this process is alive.
                report("ChronoDesk has been removed. Your settings were left in place.", false);
                remove_dir_after_exit(&layout.dir);
                0
            }
            Err(err) => {
                report(
                    &format!(
                        "ChronoDesk could not be removed completely: {err}\n\n\
                         Close whatever is using it and uninstall it again from Settings."
                    ),
                    true,
                );
                1
            }
        },
    }
}

/// The exe cannot delete itself while it runs, so a command that outlives it
/// does: it tries once a second for half a minute, however long the process
/// takes to go. It names the files it expects and removes the directory only
/// if that left it empty: never a recursive delete of a path that came from
/// the environment.
fn remove_dir_after_exit(dir: &Path) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let (exe, new) = (dir.join(EXE_NAME), dir.join(NEW_NAME));
        let old = dir.join(format!("{OLD_NAME}*"));
        let script = format!(
            "/d /c (for /l %i in (1,1,30) do @if exist \"{exe}\" (del /f /q \"{exe}\" 2>nul & ping -n 2 127.0.0.1 >nul)) \
             & del /f /q \"{old}\" \"{new}\" 2>nul & rmdir \"{dir}\"",
            exe = exe.display(),
            old = old.display(),
            new = new.display(),
            dir = dir.display(),
        );
        // By full path: a bare name is looked up next to this exe first, and
        // the uninstaller may be run from a folder anyone could drop a cmd.exe in.
        let cmd = std::env::var_os("SystemRoot")
            .map_or_else(|| PathBuf::from("cmd.exe"), |root| PathBuf::from(root).join("System32").join("cmd.exe"));
        let spawned = std::process::Command::new(cmd)
            .raw_arg(script)
            .current_dir(std::env::temp_dir())
            .creation_flags(CREATE_NO_WINDOW)
            .spawn();
        if let Err(err) = spawned {
            eprintln!("ChronoDesk: could not schedule the removal of {}: {err}", dir.display());
        }
    }
    #[cfg(not(windows))]
    let _ = dir;
}

fn message(text: &str, failed: bool) {
    #[cfg(windows)]
    {
        // Declared here for the same reason as the chime: one function is not
        // worth `Win32_UI_WindowsAndMessaging`.
        #[link(name = "user32")]
        unsafe extern "system" {
            fn MessageBoxW(owner: isize, text: *const u16, caption: *const u16, kind: u32) -> i32;
        }
        const MB_ICONERROR: u32 = 0x10;
        const MB_ICONINFORMATION: u32 = 0x40;
        let wide = |s: &str| s.encode_utf16().chain(std::iter::once(0)).collect::<Vec<u16>>();
        let (text, caption) = (wide(text), wide("ChronoDesk"));
        // SAFETY: both strings are NUL-terminated and outlive the call.
        unsafe { MessageBoxW(0, text.as_ptr(), caption.as_ptr(), if failed { MB_ICONERROR } else { MB_ICONINFORMATION }) };
    }
    #[cfg(not(windows))]
    let _ = (text, failed);
}

/// A `.lnk` file is written by the shell's own `ShellLink` object, which means
/// COM. `windows-sys` offers no interface types, so the two vtables used here
/// are spelled out by slot number, in the order `shobjidl_core.h` and
/// `objidl.h` declare them.
#[cfg(windows)]
mod shortcut {
    use std::ffi::c_void;
    use std::io;
    use std::path::Path;

    #[repr(C)]
    struct Guid(u32, u16, u16, [u8; 8]);

    const fn shell_guid(first: u32) -> Guid {
        Guid(first, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46])
    }
    const CLSID_SHELL_LINK: Guid = shell_guid(0x0002_1401);
    const IID_SHELL_LINK_W: Guid = shell_guid(0x0002_14F9);
    const IID_PERSIST_FILE: Guid = shell_guid(0x0000_010B);

    #[link(name = "ole32")]
    unsafe extern "system" {
        fn CoInitializeEx(reserved: *const c_void, model: u32) -> i32;
        fn CoUninitialize();
        fn CoCreateInstance(class: *const Guid, outer: *mut c_void, context: u32, iid: *const Guid, out: *mut *mut c_void) -> i32;
    }
    const COINIT_APARTMENTTHREADED: u32 = 2;
    const CLSCTX_INPROC_SERVER: u32 = 1;

    // IUnknown
    const QUERY_INTERFACE: usize = 0;
    const RELEASE: usize = 2;
    // IShellLinkW
    const SET_DESCRIPTION: usize = 7;
    const SET_WORKING_DIRECTORY: usize = 9;
    const SET_ICON_LOCATION: usize = 17;
    const SET_PATH: usize = 20;
    // IPersistFile
    const SAVE: usize = 6;

    type Object = *mut c_void;
    type SetString = unsafe extern "system" fn(Object, *const u16) -> i32;

    /// The function in `slot` of the object's vtable, as type `F`.
    ///
    /// SAFETY: `object` must be a live COM interface pointer and `F` the
    /// signature of that slot.
    unsafe fn method<F: Copy>(object: Object, slot: usize) -> F {
        unsafe {
            let vtable = *object.cast::<*const *const c_void>();
            std::mem::transmute_copy(&*vtable.add(slot))
        }
    }

    unsafe fn release(object: Object) {
        unsafe { method::<unsafe extern "system" fn(Object) -> u32>(object, RELEASE)(object) };
    }

    fn check(hresult: i32, what: &str) -> io::Result<()> {
        if hresult >= 0 { Ok(()) } else { Err(io::Error::other(format!("{what} failed (0x{hresult:08X})"))) }
    }

    fn wide(text: &Path) -> Vec<u16> {
        use std::os::windows::ffi::OsStrExt as _;
        text.as_os_str().encode_wide().chain(std::iter::once(0)).collect()
    }

    pub fn create(link: &Path, target: &Path, icon: &Path, description: &str) -> io::Result<()> {
        // SAFETY: COM is initialised for this thread for exactly the span of
        // the calls below, and balanced if and only if this call succeeded.
        let initialised = unsafe { CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED) } >= 0;
        let result = write(link, target, icon, description);
        if initialised {
            // SAFETY: balances the successful `CoInitializeEx` above.
            unsafe { CoUninitialize() };
        }
        result
    }

    fn write(link: &Path, target: &Path, icon: &Path, description: &str) -> io::Result<()> {
        let mut shell_link: Object = std::ptr::null_mut();
        // SAFETY: the GUIDs are the documented ones and `shell_link` receives
        // an owned interface pointer, released below.
        check(
            unsafe { CoCreateInstance(&CLSID_SHELL_LINK, std::ptr::null_mut(), CLSCTX_INPROC_SERVER, &IID_SHELL_LINK_W, &mut shell_link) },
            "creating the shortcut object",
        )?;
        let fill = || -> io::Result<()> {
            let description: Vec<u16> = description.encode_utf16().chain(std::iter::once(0)).collect();
            let directory = target.parent().map(wide).unwrap_or_else(|| vec![0]);
            // SAFETY: `shell_link` is a live `IShellLinkW`; every slot is called
            // with its documented signature and NUL-terminated strings that
            // outlive the call.
            unsafe {
                check(method::<SetString>(shell_link, SET_PATH)(shell_link, wide(target).as_ptr()), "setting the target")?;
                check(method::<SetString>(shell_link, SET_WORKING_DIRECTORY)(shell_link, directory.as_ptr()), "setting the folder")?;
                check(method::<SetString>(shell_link, SET_DESCRIPTION)(shell_link, description.as_ptr()), "setting the description")?;
                type SetIcon = unsafe extern "system" fn(Object, *const u16, i32) -> i32;
                check(method::<SetIcon>(shell_link, SET_ICON_LOCATION)(shell_link, wide(icon).as_ptr(), 0), "setting the icon")?;

                let mut file: Object = std::ptr::null_mut();
                type QueryInterface = unsafe extern "system" fn(Object, *const Guid, *mut Object) -> i32;
                check(
                    method::<QueryInterface>(shell_link, QUERY_INTERFACE)(shell_link, &IID_PERSIST_FILE, &mut file),
                    "asking for the file interface",
                )?;
                type Save = unsafe extern "system" fn(Object, *const u16, i32) -> i32;
                let saved = check(method::<Save>(file, SAVE)(file, wide(link).as_ptr(), 1), "saving the shortcut");
                release(file);
                saved
            }
        };
        let result = fill();
        // SAFETY: `shell_link` was handed to us owned and is released once.
        unsafe { release(shell_link) };
        result
    }
}

#[cfg(not(windows))]
mod shortcut {
    use std::io;
    use std::path::Path;

    pub fn create(_link: &Path, _target: &Path, _icon: &Path, _description: &str) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn a_file_named_setup_installs_and_the_flags_say_the_rest() {
        let app = Path::new(r"C:\Users\me\AppData\Local\ChronoDesk\chronodesk.exe");
        assert_eq!(requested(&[], Some(app)), None, "the installed exe is just the app");
        assert_eq!(requested(&args(&["--instrument"]), Some(app)), None);
        assert_eq!(requested(&[], Some(Path::new(r"C:\Users\me\Downloads\ChronoDesk-Setup.exe"))), Some(Action::Install));
        assert_eq!(requested(&[], Some(Path::new(r"D:\chronodesk-setup (1).exe"))), Some(Action::Install), "as a browser renames it");
        assert_eq!(requested(&args(&["--install"]), Some(app)), Some(Action::Install));
        assert_eq!(requested(&args(&["--uninstall", "--quiet"]), Some(app)), Some(Action::Uninstall));
        // The uninstall command must work from a setup-named file too.
        assert_eq!(requested(&args(&["--uninstall"]), Some(Path::new("ChronoDesk-Setup.exe"))), Some(Action::Uninstall));
        assert_eq!(requested(&[], None), None);
    }

    /// A scratch installation: its own directory and its own registry root.
    #[cfg(windows)]
    struct Scratch {
        root: PathBuf,
        key: String,
    }

    #[cfg(windows)]
    impl Scratch {
        fn new(tag: &str) -> Self {
            let root = std::env::temp_dir().join(format!("chronodesk-install-test-{}-{tag}", std::process::id()));
            let _ = fs::remove_dir_all(&root);
            fs::create_dir_all(&root).unwrap();
            Self { root, key: format!(r"Software\ChronoDesk-install-test-{}-{tag}", std::process::id()) }
        }

        fn layout(&self) -> Layout {
            Layout {
                dir: self.root.join("app"),
                shortcut: Some(self.root.join("start menu").join("ChronoDesk.lnk")),
                uninstall_key: format!(r"{}\Uninstall\ChronoDesk", self.key),
                autostart: Autostart::under(&self.key),
            }
        }

        fn source(&self, name: &str, contents: &[u8]) -> PathBuf {
            let path = self.root.join(name);
            fs::write(&path, contents).unwrap();
            path
        }
    }

    #[cfg(windows)]
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = registry::delete_tree(&self.key);
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    #[cfg(windows)]
    #[test]
    fn install_copies_registers_and_is_idempotent() {
        let scratch = Scratch::new("install");
        let layout = scratch.layout();
        let setup = scratch.source("ChronoDesk-Setup.exe", b"version one");

        assert_eq!(install(&layout, &setup, None).unwrap(), Installed::Copied);
        assert_eq!(fs::read(layout.exe()).unwrap(), b"version one");
        assert!(layout.dir.join(ICON_NAME).exists());
        assert!(layout.shortcut.as_ref().unwrap().exists(), "the shell wrote a shortcut");
        let key = &layout.uninstall_key;
        assert_eq!(registry::read_string(key, "DisplayName").as_deref(), Some("ChronoDesk"));
        assert_eq!(registry::read_string(key, "DisplayVersion").as_deref(), Some(env!("CARGO_PKG_VERSION")));
        let uninstall = registry::read_string(key, "UninstallString").unwrap();
        assert_eq!(uninstall, format!("\"{}\" --uninstall", layout.exe().display()));

        assert_eq!(install(&layout, &setup, None).unwrap(), Installed::AlreadyCurrent, "same bytes: nothing to copy");
        assert_eq!(install(&layout, &layout.exe(), None).unwrap(), Installed::AlreadyCurrent, "the installed exe, asked to install");
        assert!(!layout.dir.join(OLD_NAME).exists() && !layout.dir.join(NEW_NAME).exists());
    }

    #[cfg(windows)]
    #[test]
    fn an_upgrade_replaces_the_exe_even_while_the_old_one_is_held_open() {
        let scratch = Scratch::new("upgrade");
        let layout = scratch.layout();
        install(&layout, &scratch.source("one.exe", b"version one"), None).unwrap();

        // What a running exe looks like to the file system: open, not shared
        // for writing, but shared for delete, which is what lets it be renamed
        // out of the way though it cannot be overwritten.
        use std::os::windows::fs::OpenOptionsExt as _;
        const FILE_SHARE_READ_DELETE: u32 = 0x1 | 0x4;
        let held = fs::OpenOptions::new().read(true).share_mode(FILE_SHARE_READ_DELETE).open(layout.exe()).unwrap();

        assert_eq!(install(&layout, &scratch.source("two.exe", b"version two!"), None).unwrap(), Installed::Copied);
        assert_eq!(fs::read(layout.exe()).unwrap(), b"version two!");
        drop(held);
        assert_eq!(install(&layout, &scratch.source("three.exe", b"version three"), None).unwrap(), Installed::Copied);
        assert!(!layout.dir.join(OLD_NAME).exists(), "the leftover goes with the next upgrade");
    }

    /// Open without sharing delete: what a leftover exe that is still running
    /// looks like to anything that tries to remove or replace it.
    #[cfg(windows)]
    fn hold(path: &Path) -> fs::File {
        use std::os::windows::fs::OpenOptionsExt as _;
        const FILE_SHARE_READ: u32 = 0x1;
        fs::OpenOptions::new().read(true).share_mode(FILE_SHARE_READ).open(path).unwrap()
    }

    /// An older overlay that ignored the quit keeps running as the `.old` exe;
    /// the next upgrade must get past it rather than fail on it.
    #[cfg(windows)]
    #[test]
    fn an_upgrade_gets_past_an_old_exe_that_is_still_running() {
        let scratch = Scratch::new("stale_old");
        let layout = scratch.layout();
        install(&layout, &scratch.source("one.exe", b"version one"), None).unwrap();
        fs::write(layout.dir.join(OLD_NAME), b"version zero, still running").unwrap();
        let held = hold(&layout.dir.join(OLD_NAME));

        assert_eq!(install(&layout, &scratch.source("two.exe", b"version two"), None).unwrap(), Installed::Copied);
        assert_eq!(fs::read(layout.exe()).unwrap(), b"version two");
        assert!(!layout.dir.join(NEW_NAME).exists());

        drop(held);
        install(&layout, &scratch.source("three.exe", b"version three"), None).unwrap();
        assert_eq!(leftovers(&layout.dir), Vec::<PathBuf>::new(), "all of them go once they can");
    }

    #[cfg(windows)]
    #[test]
    fn uninstall_is_not_stopped_by_an_old_exe_that_is_still_running() {
        let scratch = Scratch::new("uninstall_stale");
        let layout = scratch.layout();
        install(&layout, &scratch.source("setup.exe", b"app"), None).unwrap();
        fs::write(layout.dir.join(OLD_NAME), b"still running").unwrap();
        let _held = hold(&layout.dir.join(OLD_NAME));

        uninstall(&layout, None).unwrap();
        assert!(registry::read_string(&layout.uninstall_key, "DisplayName").is_none());
        assert!(!layout.shortcut.as_ref().unwrap().exists());
    }

    /// If something could not be removed, the entry in Settings stays, so the
    /// uninstall can be run again instead of leaving files nothing points to.
    #[cfg(windows)]
    #[test]
    fn an_uninstall_that_fails_keeps_its_entry_and_does_the_rest() {
        let scratch = Scratch::new("uninstall_fail");
        let layout = scratch.layout();
        install(&layout, &scratch.source("setup.exe", b"app"), None).unwrap();
        let shortcut = layout.shortcut.clone().unwrap();
        let held = hold(&shortcut);

        assert!(uninstall(&layout, None).is_err());
        assert_eq!(registry::read_string(&layout.uninstall_key, "DisplayName").as_deref(), Some("ChronoDesk"));
        assert!(!layout.dir.join(ICON_NAME).exists(), "what could go, went");

        drop(held);
        uninstall(&layout, None).unwrap();
        assert!(registry::read_string(&layout.uninstall_key, "DisplayName").is_none());
    }

    #[cfg(windows)]
    #[test]
    fn an_existing_autostart_entry_follows_the_exe_and_none_is_invented() {
        let scratch = Scratch::new("autostart");
        let layout = scratch.layout();
        let setup = scratch.source("setup.exe", b"app");

        install(&layout, &setup, None).unwrap();
        assert!(!layout.autostart.enabled(&layout.exe()), "starting at login stays the user's choice");

        layout.autostart.set(Path::new(r"C:\dev\chronodesk\target\release\chronodesk.exe"), true).unwrap();
        install(&layout, &setup, None).unwrap();
        assert!(layout.autostart.enabled(&layout.exe()), "the entry now names the fixed path");
    }

    #[cfg(windows)]
    #[test]
    fn uninstall_removes_what_install_put_there_and_nothing_else() {
        let scratch = Scratch::new("uninstall");
        let layout = scratch.layout();
        install(&layout, &scratch.source("setup.exe", b"app"), None).unwrap();
        layout.autostart.set(&layout.exe(), true).unwrap();
        let bystander = scratch.source("app\\notes.txt", b"mine");

        uninstall(&layout, None).unwrap();
        assert!(registry::read_string(&layout.uninstall_key, "DisplayName").is_none());
        assert!(!layout.shortcut.as_ref().unwrap().exists());
        assert!(!layout.dir.join(ICON_NAME).exists());
        assert!(!layout.autostart.enabled(&layout.exe()));
        assert!(bystander.exists(), "a file it did not put there stays");
        // The exe goes last, from outside the process; see `remove_dir_after_exit`.
        assert!(layout.exe().exists());
        uninstall(&layout, None).unwrap();
    }

    /// An overlay that does not quit (a build from before it listened, or a
    /// hung one) holds the exe: the uninstall stops before it takes anything.
    #[cfg(windows)]
    #[test]
    fn uninstall_waits_for_the_overlay_and_touches_nothing_while_it_stays() {
        let scratch = Scratch::new("uninstall_running");
        let layout = scratch.layout();
        install(&layout, &scratch.source("setup.exe", b"app"), None).unwrap();
        let config = scratch.root.join("app.ron");
        let _running = instance::acquire(&config).expect("the guard, as a deaf overlay holds it");

        assert!(uninstall_waiting(&layout, Some(&config), Duration::from_millis(300)).is_err());
        assert_eq!(registry::read_string(&layout.uninstall_key, "DisplayName").as_deref(), Some("ChronoDesk"));
        assert!(layout.shortcut.as_ref().unwrap().exists());
    }

    #[cfg(windows)]
    #[test]
    fn uninstall_leaves_an_autostart_entry_for_some_other_copy_alone() {
        let scratch = Scratch::new("foreign");
        let layout = scratch.layout();
        install(&layout, &scratch.source("setup.exe", b"app"), None).unwrap();
        // Registered after the installation, so nothing repointed it.
        let other = Path::new(r"D:\portable\chronodesk.exe");
        layout.autostart.set(other, true).unwrap();
        uninstall(&layout, None).unwrap();
        assert!(layout.autostart.enabled(other));
    }
}
