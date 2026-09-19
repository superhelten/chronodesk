//! The sound of a finished timer.
//!
//! No audio file and no audio stack: the system's own notification sound,
//! played by Windows, asynchronously, at the volume and under the mute and
//! sound scheme the user already chose. With the scheme set to "No sounds"
//! the timer is silent, which is that user's stated wish.

/// Returns at once; the sound plays on the system's side.
pub fn play() {
    #[cfg(windows)]
    {
        // Declared here rather than switching on `Win32_UI_WindowsAndMessaging`,
        // the largest module in `windows-sys`, for one function.
        #[link(name = "user32")]
        unsafe extern "system" {
            fn MessageBeep(kind: u32) -> i32;
        }
        /// "Asterisk" in the sound scheme: the information chime, not the error one.
        const MB_ICONASTERISK: u32 = 0x40;
        // SAFETY: takes a plain integer, touches no memory of ours.
        unsafe { MessageBeep(MB_ICONASTERISK) };
    }
}
