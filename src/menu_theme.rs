//! Dark menus when Windows is set to dark apps.
//!
//! The menu is an ordinary Win32 pop-up menu, and Windows draws those light
//! unless the process says it can take dark ones. That is said through
//! `SetPreferredAppMode` and made to stick with `FlushMenuThemes`, two
//! functions `uxtheme.dll` exports by ordinal only (135 and 136), which is
//! how Explorer, Notepad++ and the Windows Terminal ask for the same thing.
//! Neither the menu crate nor winit calls them: winit darkens only the title
//! bar, and the menu crate only menu bars.
//!
//! Which way to go is read from the documented place, the user's *Choose
//! your default app mode* setting (`AppsUseLightTheme`), and read again now
//! and then so a change of mind follows without a restart.

/// What `SetPreferredAppMode` is asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    ForceDark,
    ForceLight,
}

/// The registry value that says whether apps are to be light: missing (before
/// Windows 10 1809, or never set) counts as light, as Windows itself does.
pub fn mode_for(apps_use_light_theme: Option<u32>) -> Mode {
    match apps_use_light_theme {
        Some(0) => Mode::ForceDark,
        _ => Mode::ForceLight,
    }
}

const PERSONALIZE: &str = r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize";

/// The mode the user's setting asks for right now.
pub fn current() -> Mode {
    mode_for(crate::registry::read_dword(PERSONALIZE, "AppsUseLightTheme"))
}

/// Makes the menus this process opens from now on follow `mode`.
pub fn apply(mode: Mode) {
    imp::apply(mode);
}

#[cfg(windows)]
mod imp {
    use super::Mode;

    // Declared here rather than switching on `Win32_System_LibraryLoader` for
    // two functions, as the chime does for its one.
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn LoadLibraryW(name: *const u16) -> isize;
        fn GetProcAddress(module: isize, name: *const u8) -> Option<unsafe extern "system" fn()>;
    }

    /// `PreferredAppMode` in uxtheme: 0 default, 1 allow dark, 2 force dark,
    /// 3 force light.
    const FORCE_DARK: i32 = 2;
    const FORCE_LIGHT: i32 = 3;
    const SET_PREFERRED_APP_MODE: usize = 135;
    const FLUSH_MENU_THEMES: usize = 136;

    pub fn apply(mode: Mode) {
        let name: Vec<u16> = "uxtheme.dll".encode_utf16().chain(std::iter::once(0)).collect();
        // SAFETY: a NUL-terminated name; uxtheme is a system DLL that is
        // already loaded in any GUI process, so this only takes a reference.
        let module = unsafe { LoadLibraryW(name.as_ptr()) };
        if module == 0 {
            return;
        }
        // An ordinal is passed in place of a name, as MAKEINTRESOURCEA does.
        // SAFETY: the module handle is valid; a missing export gives `None`.
        let (set, flush) = unsafe {
            (
                GetProcAddress(module, SET_PREFERRED_APP_MODE as *const u8),
                GetProcAddress(module, FLUSH_MENU_THEMES as *const u8),
            )
        };
        let (Some(set), Some(flush)) = (set, flush) else { return };
        // SAFETY: on Windows 10 1903 and later these ordinals are
        // `PreferredAppMode SetPreferredAppMode(PreferredAppMode)` and
        // `void FlushMenuThemes()`. On 1809, 135 was `AllowDarkModeForApp(BOOL)`,
        // which takes the same integer and reads any non-zero as yes.
        unsafe {
            let set: unsafe extern "system" fn(i32) -> i32 = std::mem::transmute(set);
            set(if mode == Mode::ForceDark { FORCE_DARK } else { FORCE_LIGHT });
            flush();
        }
    }
}

#[cfg(not(windows))]
mod imp {
    pub fn apply(_mode: super::Mode) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dark_only_when_windows_says_apps_are_not_light() {
        assert_eq!(mode_for(Some(0)), Mode::ForceDark);
        assert_eq!(mode_for(Some(1)), Mode::ForceLight);
        assert_eq!(mode_for(None), Mode::ForceLight, "unset is light, as in Windows");
    }

    /// Both ways and back, on the real uxtheme: it must neither crash nor
    /// leave the test process in a state another test would notice.
    #[cfg(windows)]
    #[test]
    fn asking_for_either_mode_is_harmless() {
        apply(Mode::ForceDark);
        apply(Mode::ForceLight);
        apply(current());
    }
}
