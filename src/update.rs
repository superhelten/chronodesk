//! Finding out that a newer release exists.
//!
//! This is the app's one network request. Once a day at most it asks GitHub
//! where `releases/latest` points, with redirects switched off, and reads the
//! version from the `Location` header: no API, no JSON, no body, and nothing
//! sent beyond a `ChronoDesk/<version>` user agent. A newer version shows up
//! as an item at the top of the menu that opens the release page. Nothing is
//! downloaded or run: the exe is not signed, so the user fetches it the way
//! they fetched this one, and SmartScreen gets its say.
//!
//! The request goes through WinHTTP, the HTTP stack Windows already has, with
//! its TLS and the system's proxy settings, rather than a crate that would
//! bring its own and double the size of the exe.

use std::fmt;
use std::io;
use std::time::Duration;

/// Where the releases live, for the menu item to open.
pub const RELEASES_URL: &str = "https://github.com/superhelten/chronodesk/releases";
const HOST: &str = "github.com";
const LATEST_PATH: &str = "/superhelten/chronodesk/releases/latest";
/// How often a successful check is repeated, in seconds.
pub const CHECK_EVERY_S: u64 = 24 * 3600;
/// After a failed check, the next try. Often the machine is simply not online
/// yet: an overlay that starts with Windows checks before the network is up.
pub const RETRY_AFTER: Duration = Duration::from_secs(3600);
/// Long enough for a slow connection, short enough that a stuck one does not
/// keep a thread around for minutes.
const TIMEOUT_MS: i32 = 10_000;

/// `major.minor.patch`, compared field by field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version(u32, u32, u32);

impl Version {
    /// `0.2.0` or `v0.2.0`; anything else, pre-release suffixes included, is
    /// not a version this checker offers.
    pub fn parse(text: &str) -> Option<Self> {
        let mut parts = text.strip_prefix('v').unwrap_or(text).split('.').map(|p| p.parse::<u32>().ok());
        let version = Self(parts.next()??, parts.next()??, parts.next()??);
        parts.next().is_none().then_some(version)
    }

    /// The version of this build.
    pub fn current() -> Self {
        Self::parse(env!("CARGO_PKG_VERSION")).expect("Cargo.toml has a plain version")
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.0, self.1, self.2)
    }
}

/// The version `releases/latest` redirects to: the last path segment of a
/// `…/releases/tag/v0.2.0` location.
pub fn version_from_location(location: &str) -> Option<Version> {
    let (_, tag) = location.trim().rsplit_once("/releases/tag/")?;
    Version::parse(tag.split(['?', '#']).next()?)
}

/// Whether a check is due, given when the last successful one was, both in
/// seconds since the Unix epoch. A clock set back past the last check counts
/// as due, or it would not come round again for as long as it was set back.
pub fn due(checked_at: u64, now: u64) -> bool {
    now < checked_at || now - checked_at >= CHECK_EVERY_S
}

/// The version of the latest release, asked of GitHub. Blocks for up to the
/// timeout: call it from a thread of its own.
pub fn latest() -> io::Result<Version> {
    let location = imp::redirect_location(Target { secure: true, host: HOST, port: 443, path: LATEST_PATH })?;
    version_from_location(&location)
        .ok_or_else(|| io::Error::other(format!("no version in the release redirect ({location})")))
}

/// Opens the release page in the default browser.
pub fn open_release_page(version: Version) {
    imp::open(&format!("{RELEASES_URL}/tag/v{version}"));
}

#[derive(Debug, Clone, Copy)]
struct Target<'a> {
    secure: bool,
    host: &'a str,
    port: u16,
    path: &'a str,
}

#[cfg(windows)]
mod imp {
    use std::io;
    use std::ptr::{null, null_mut};

    use windows_sys::Win32::Networking::WinHttp::{
        WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, WINHTTP_DISABLE_REDIRECTS, WINHTTP_FLAG_SECURE,
        WINHTTP_OPTION_DISABLE_FEATURE, WINHTTP_QUERY_FLAG_NUMBER, WINHTTP_QUERY_LOCATION, WINHTTP_QUERY_STATUS_CODE,
        WinHttpCloseHandle, WinHttpConnect, WinHttpOpen, WinHttpOpenRequest, WinHttpQueryHeaders,
        WinHttpReceiveResponse, WinHttpSendRequest, WinHttpSetOption, WinHttpSetTimeouts,
    };

    use super::{TIMEOUT_MS, Target};

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// A WinHTTP handle, closed when dropped, so every early return closes
    /// whatever was opened before it.
    struct Handle(*mut core::ffi::c_void);

    impl Handle {
        fn new(raw: *mut core::ffi::c_void) -> io::Result<Self> {
            if raw.is_null() { Err(io::Error::last_os_error()) } else { Ok(Self(raw)) }
        }
    }

    impl Drop for Handle {
        fn drop(&mut self) {
            // SAFETY: a handle WinHTTP returned and nothing else closes.
            unsafe { WinHttpCloseHandle(self.0) };
        }
    }

    fn check(ok: i32) -> io::Result<()> {
        if ok != 0 { Ok(()) } else { Err(io::Error::last_os_error()) }
    }

    /// Sends a GET without following redirects and returns where the answer
    /// redirects to. Any other answer is an error.
    pub(super) fn redirect_location(target: Target<'_>) -> io::Result<String> {
        let agent = wide(concat!("ChronoDesk/", env!("CARGO_PKG_VERSION")));
        let (host, verb, path) = (wide(target.host), wide("GET"), wide(target.path));
        // SAFETY: every string is NUL-terminated and outlives the call that
        // reads it; every handle is closed by `Handle`, children before parents
        // since they are declared after them.
        unsafe {
            let session =
                Handle::new(WinHttpOpen(agent.as_ptr(), WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, null(), null(), 0))?;
            check(WinHttpSetTimeouts(session.0, TIMEOUT_MS, TIMEOUT_MS, TIMEOUT_MS, TIMEOUT_MS))?;
            let connection = Handle::new(WinHttpConnect(session.0, host.as_ptr(), target.port, 0))?;
            let flags = if target.secure { WINHTTP_FLAG_SECURE } else { 0 };
            let request = Handle::new(WinHttpOpenRequest(
                connection.0,
                verb.as_ptr(),
                path.as_ptr(),
                null(),
                null(),
                null(),
                flags,
            ))?;
            let no_redirects: u32 = WINHTTP_DISABLE_REDIRECTS;
            check(WinHttpSetOption(
                request.0,
                WINHTTP_OPTION_DISABLE_FEATURE,
                (&raw const no_redirects).cast(),
                4,
            ))?;
            check(WinHttpSendRequest(request.0, null(), 0, null(), 0, 0, 0))?;
            check(WinHttpReceiveResponse(request.0, null_mut()))?;

            let mut status: u32 = 0;
            let mut size = 4u32;
            check(WinHttpQueryHeaders(
                request.0,
                WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
                null(),
                (&raw mut status).cast(),
                &mut size,
                null_mut(),
            ))?;
            if !(300..400).contains(&status) {
                return Err(io::Error::other(format!("expected a redirect, got HTTP {status}")));
            }

            let mut buffer = [0u16; 1024];
            let mut size = (buffer.len() * 2) as u32;
            check(WinHttpQueryHeaders(
                request.0,
                WINHTTP_QUERY_LOCATION,
                null(),
                buffer.as_mut_ptr().cast(),
                &mut size,
                null_mut(),
            ))?;
            Ok(String::from_utf16_lossy(&buffer[..size as usize / 2]))
        }
    }

    pub(super) fn open(url: &str) {
        // Declared here rather than switching on `Win32_UI_Shell` for one call.
        #[link(name = "shell32")]
        unsafe extern "system" {
            fn ShellExecuteW(
                window: isize,
                operation: *const u16,
                file: *const u16,
                parameters: *const u16,
                directory: *const u16,
                show: i32,
            ) -> isize;
        }
        const SW_SHOWNORMAL: i32 = 1;
        let (verb, url) = (wide("open"), wide(url));
        // SAFETY: both strings are NUL-terminated and outlive the call.
        unsafe { ShellExecuteW(0, verb.as_ptr(), url.as_ptr(), null(), null(), SW_SHOWNORMAL) };
    }
}

#[cfg(not(windows))]
mod imp {
    use std::io;

    use super::Target;

    pub(super) fn redirect_location(_target: Target<'_>) -> io::Result<String> {
        Err(io::ErrorKind::Unsupported.into())
    }

    pub(super) fn open(_url: &str) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_parse_with_or_without_the_tag_prefix() {
        assert_eq!(Version::parse("0.2.0"), Some(Version(0, 2, 0)));
        assert_eq!(Version::parse("v1.10.3"), Some(Version(1, 10, 3)));
        for bad in ["", "1.2", "1.2.3.4", "v1.2.x", "1.2.3-beta", "latest"] {
            assert_eq!(Version::parse(bad), None, "{bad:?}");
        }
        assert_eq!(Version::current().to_string(), env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn versions_compare_by_number_not_by_text() {
        assert!(Version(0, 10, 0) > Version(0, 9, 9));
        assert!(Version(1, 0, 0) > Version(0, 99, 99));
        assert_eq!(Version(0, 2, 0).to_string(), "0.2.0");
    }

    #[test]
    fn the_version_comes_from_where_latest_redirects_to() {
        let location = "https://github.com/superhelten/chronodesk/releases/tag/v0.2.0";
        assert_eq!(version_from_location(location), Some(Version(0, 2, 0)));
        assert_eq!(version_from_location("https://github.com/superhelten/chronodesk/releases"), None, "no release yet");
        assert_eq!(version_from_location("https://example.com/releases/tag/nightly"), None);
    }

    #[test]
    fn a_check_is_due_once_a_day_and_after_the_clock_went_back() {
        let t = 1_800_000_000;
        assert!(due(0, t), "never checked");
        assert!(!due(t, t + 3600));
        assert!(due(t, t + CHECK_EVERY_S));
        assert!(due(t, t - 60), "clock set back");
    }

    /// The real WinHTTP code against a server on the loopback that answers
    /// like GitHub does: a 302 to the latest tag. Plain HTTP, no network.
    #[cfg(windows)]
    #[test]
    fn the_redirect_is_read_and_not_followed() {
        use std::io::{Read as _, Write as _};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 2048];
            let n = stream.read(&mut request).unwrap();
            let request = String::from_utf8_lossy(&request[..n]).into_owned();
            let answer = "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1/o/r/releases/tag/v9.8.7\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
            stream.write_all(answer.as_bytes()).unwrap();
            request
        });

        let target = Target { secure: false, host: "127.0.0.1", port, path: "/o/r/releases/latest" };
        let location = imp::redirect_location(target).unwrap();
        assert_eq!(version_from_location(&location), Some(Version(9, 8, 7)));

        let request = server.join().unwrap();
        assert!(request.starts_with("GET /o/r/releases/latest HTTP/1.1"), "{request}");
        assert!(request.contains(concat!("User-Agent: ChronoDesk/", env!("CARGO_PKG_VERSION"))), "{request}");
        let headers = request.lines().filter(|l| !l.is_empty()).count();
        assert!(headers <= 5, "nothing sent beyond the essentials:\n{request}");
    }

    /// The one test that goes out to GitHub; run by hand with `--ignored`.
    #[cfg(windows)]
    #[test]
    #[ignore = "network"]
    fn github_says_which_release_is_latest() {
        assert!(latest().unwrap() >= Version(0, 1, 1));
    }

    #[cfg(windows)]
    #[test]
    fn an_answer_that_is_not_a_redirect_is_an_error() {
        use std::io::{Read as _, Write as _};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let _ = stream.read(&mut [0u8; 2048]).unwrap();
            stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
        });
        let target = Target { secure: false, host: "127.0.0.1", port, path: "/o/r/releases/latest" };
        assert!(imp::redirect_location(target).is_err());
        server.join().unwrap();
    }
}
