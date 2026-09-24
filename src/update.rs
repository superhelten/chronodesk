//! Finding out that a newer release exists, and installing it on request.
//!
//! Once a day at most the app asks GitHub where `releases/latest` points, with
//! redirects switched off, and reads the version from the `Location` header:
//! no API, no JSON, no body, and nothing sent beyond a `ChronoDesk/<version>`
//! user agent. A newer version shows up at the top of the menu.
//!
//! Clicking it downloads the release's `SHA256SUMS.txt`, its signature and the
//! setup, and runs the setup only if the signature was made by the release key
//! built into this exe and the setup matches its listed checksum. The private
//! key is kept by whoever makes releases, not on GitHub, so a compromised
//! account can publish an exe but not one this accepts; anything that does
//! not verify is thrown away and the release page is opened instead. Nothing
//! happens without the click.
//!
//! Everything goes through what Windows already has: WinHTTP, with its TLS and
//! the system's proxy settings, and CNG for the hash and the signature, rather
//! than crates that would bring their own and double the size of the exe.

use std::fmt;
use std::io;
use std::time::Duration;

/// Where the releases live, for the menu item to open.
pub const RELEASES_URL: &str = "https://github.com/superhelten/chronodesk/releases";
const HOST: &str = "github.com";
const LATEST_PATH_DIR: &str = "/superhelten/chronodesk/releases";
const LATEST_PATH: &str = "/superhelten/chronodesk/releases/latest";
/// How often a successful check is repeated, in seconds.
pub const CHECK_EVERY_S: u64 = 24 * 3600;
/// After a failed check, the next try. Often the machine is simply not online
/// yet: an overlay that starts with Windows checks before the network is up.
pub const RETRY_AFTER: Duration = Duration::from_secs(3600);
/// Long enough for a slow connection, short enough that a stuck one does not
/// keep a thread around for minutes.
const TIMEOUT_MS: i32 = 10_000;
/// The largest setup the app will download; the real one is under 6 MB.
const MAX_SETUP_BYTES: usize = 32 << 20;
const MAX_SMALL_BYTES: usize = 16 << 10;
const SETUP_NAME: &str = "ChronoDesk-Setup.exe";
/// What a downloaded setup is saved as, in the temp folder, before it runs.
const DOWNLOAD_PREFIX: &str = "ChronoDesk-Setup-";

/// The public half of the release signing key: ECDSA P-256, X then Y. The
/// private half never leaves the machine releases are signed on; see
/// `scripts/sign-release.ps1`.
const RELEASE_KEY: [u8; 64] = hex_array(
    "97349a9e16a5b957c8cde36a490d1450aa3204e819a53b0ef78fc10d8bd7fd9ec0751a913ff3731eb67c5d0e3583501c56112ba9be9ccd06e3648146e7d966d7",
);

const fn hex_array<const N: usize>(text: &str) -> [u8; N] {
    const fn digit(c: u8) -> u8 {
        match c {
            b'0'..=b'9' => c - b'0',
            b'a'..=b'f' => c - b'a' + 10,
            _ => panic!("not lowercase hex"),
        }
    }
    let bytes = text.as_bytes();
    assert!(bytes.len() == 2 * N, "wrong length");
    let mut out = [0u8; N];
    let mut i = 0;
    while i < N {
        out[i] = digit(bytes[2 * i]) << 4 | digit(bytes[2 * i + 1]);
        i += 1;
    }
    out
}

fn parse_hex(text: &str) -> Option<Vec<u8>> {
    let text = text.trim();
    if !text.len().is_multiple_of(2) {
        return None;
    }
    (0..text.len()).step_by(2).map(|i| u8::from_str_radix(text.get(i..i + 2)?, 16).ok()).collect()
}

/// The checksum `SHA256SUMS.txt` lists for `name`.
fn checksum_for(sums: &str, name: &str) -> Option<Vec<u8>> {
    sums.lines().find_map(|line| {
        let (hash, file) = line.split_once(char::is_whitespace)?;
        (file.trim_start_matches([' ', '*']) == name).then(|| parse_hex(hash)).flatten()
    })
}

/// Why a downloaded update was not installed.
#[derive(Debug)]
pub enum Refused {
    Download(io::Error),
    /// The checksums were not signed with the release key.
    Signature,
    /// The setup does not match its signed checksum.
    Checksum,
}

impl fmt::Display for Refused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Download(err) => write!(f, "download failed: {err}"),
            Self::Signature => f.write_str("the release is not signed with the release key"),
            Self::Checksum => f.write_str("the setup does not match its signed checksum"),
        }
    }
}

/// The whole check a download has to pass: the checksums carry a signature by
/// `key`, and the setup is what they list.
fn verified(sums: &[u8], signature: &[u8], setup: &[u8], key: &[u8; 64]) -> Result<(), Refused> {
    let signature = std::str::from_utf8(signature).ok().and_then(parse_hex).ok_or(Refused::Signature)?;
    if !imp::verify_p256(key, sums, &signature) {
        return Err(Refused::Signature);
    }
    let sums = std::str::from_utf8(sums).map_err(|_| Refused::Checksum)?;
    let expected = checksum_for(sums, SETUP_NAME).ok_or(Refused::Checksum)?;
    if imp::sha256(setup).as_slice() != expected.as_slice() {
        return Err(Refused::Checksum);
    }
    Ok(())
}

/// Downloads the setup of `version` and verifies it. On success it is saved in
/// the temp folder, ready to run; nothing that failed is left on disk.
pub fn download_setup(version: Version) -> Result<std::path::PathBuf, Refused> {
    let base = format!("{LATEST_PATH_DIR}/download/v{version}/");
    let get = |name: &str, limit| {
        let path = format!("{base}{name}");
        imp::get(Target { secure: true, host: HOST, port: 443, path: &path }, limit).map_err(Refused::Download)
    };
    let sums = get("SHA256SUMS.txt", MAX_SMALL_BYTES)?;
    let signature = get("SHA256SUMS.txt.sig", MAX_SMALL_BYTES)?;
    // Checked before the large download: an unsigned release costs 6 MB nobody needs.
    let early = std::str::from_utf8(&signature).ok().and_then(parse_hex);
    if !early.is_some_and(|sig| imp::verify_p256(&RELEASE_KEY, &sums, &sig)) {
        return Err(Refused::Signature);
    }
    let setup = get(SETUP_NAME, MAX_SETUP_BYTES)?;
    verified(&sums, &signature, &setup, &RELEASE_KEY)?;
    let path = std::env::temp_dir().join(format!("{DOWNLOAD_PREFIX}{version}.exe"));
    std::fs::write(&path, &setup).map_err(Refused::Download)?;
    Ok(path)
}

/// Removes setups downloaded by earlier updates. One still running (the
/// update that started this very launch, perhaps) stays until the next time.
pub fn clean_downloads() {
    let Ok(entries) = std::fs::read_dir(std::env::temp_dir()) else { return };
    for entry in entries.filter_map(Result::ok) {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with(DOWNLOAD_PREFIX) && name.ends_with(".exe") {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

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
        WinHttpCloseHandle, WinHttpConnect, WinHttpOpen, WinHttpOpenRequest, WinHttpQueryHeaders, WinHttpReadData,
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

    /// A request whose response has arrived. The fields close in the order
    /// they are declared: the request before its connection before its session.
    struct Exchange {
        request: Handle,
        _connection: Handle,
        _session: Handle,
        status: u32,
    }

    /// Sends a GET and waits for the response headers.
    fn exchange(target: Target<'_>, follow_redirects: bool) -> io::Result<Exchange> {
        let agent = wide(concat!("ChronoDesk/", env!("CARGO_PKG_VERSION")));
        let (host, verb, path) = (wide(target.host), wide("GET"), wide(target.path));
        // SAFETY: every string is NUL-terminated and outlives the call that
        // reads it; every handle is closed by `Handle`.
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
            if !follow_redirects {
                let no_redirects: u32 = WINHTTP_DISABLE_REDIRECTS;
                check(WinHttpSetOption(
                    request.0,
                    WINHTTP_OPTION_DISABLE_FEATURE,
                    (&raw const no_redirects).cast(),
                    4,
                ))?;
            }
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
            Ok(Exchange { request, _connection: connection, _session: session, status })
        }
    }

    /// The body of a GET, following redirects; anything but a 200 is an error,
    /// and so is a body larger than `limit`.
    pub(super) fn get(target: Target<'_>, limit: usize) -> io::Result<Vec<u8>> {
        let exchange = exchange(target, true)?;
        if exchange.status != 200 {
            return Err(io::Error::other(format!("HTTP {} for {}", exchange.status, target.path)));
        }
        let mut body = Vec::new();
        let mut chunk = vec![0u8; 64 << 10];
        loop {
            let mut read = 0u32;
            // SAFETY: `chunk` is valid for its length; the request is open.
            check(unsafe { WinHttpReadData(exchange.request.0, chunk.as_mut_ptr().cast(), chunk.len() as u32, &mut read) })?;
            if read == 0 {
                return Ok(body);
            }
            if body.len() + read as usize > limit {
                return Err(io::Error::other(format!("{} is larger than {limit} bytes", target.path)));
            }
            body.extend_from_slice(&chunk[..read as usize]);
        }
    }

    /// Sends a GET without following redirects and returns where the answer
    /// redirects to. Any other answer is an error.
    pub(super) fn redirect_location(target: Target<'_>) -> io::Result<String> {
        let exchange = exchange(target, false)?;
        let (request, status) = (&exchange.request, exchange.status);
        // SAFETY: the buffer is valid for the size passed; the request is open.
        unsafe {
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

    pub(super) fn sha256(data: &[u8]) -> [u8; 32] {
        use windows_sys::Win32::Security::Cryptography::{BCRYPT_SHA256_ALG_HANDLE, BCryptHash};
        let mut out = [0u8; 32];
        // SAFETY: the pseudo-handle needs no opening; both buffers are valid
        // for the lengths passed.
        let status = unsafe {
            BCryptHash(BCRYPT_SHA256_ALG_HANDLE, null(), 0, data.as_ptr(), data.len() as u32, out.as_mut_ptr(), 32)
        };
        assert_eq!(status, 0, "SHA-256 is always available");
        out
    }

    /// An ECDSA P-256 signature (r then s) over the SHA-256 of `message`.
    pub(super) fn verify_p256(key: &[u8; 64], message: &[u8], signature: &[u8]) -> bool {
        use windows_sys::Win32::Security::Cryptography::{
            BCRYPT_ECCKEY_BLOB, BCRYPT_ECCPUBLIC_BLOB, BCRYPT_ECDSA_P256_ALGORITHM, BCRYPT_ECDSA_PUBLIC_P256_MAGIC,
            BCryptCloseAlgorithmProvider, BCryptDestroyKey, BCryptImportKeyPair, BCryptOpenAlgorithmProvider,
            BCryptVerifySignature,
        };
        if signature.len() != 64 {
            return false;
        }
        let header = BCRYPT_ECCKEY_BLOB { dwMagic: BCRYPT_ECDSA_PUBLIC_P256_MAGIC, cbKey: 32 };
        let mut blob = Vec::with_capacity(8 + 64);
        blob.extend_from_slice(&header.dwMagic.to_le_bytes());
        blob.extend_from_slice(&header.cbKey.to_le_bytes());
        blob.extend_from_slice(key);
        let hash = sha256(message);
        // SAFETY: the provider and key handles are closed on every path below;
        // every buffer is valid for the length passed with it.
        unsafe {
            let mut algorithm = null_mut();
            if BCryptOpenAlgorithmProvider(&mut algorithm, BCRYPT_ECDSA_P256_ALGORITHM, null(), 0) != 0 {
                return false;
            }
            let mut handle = null_mut();
            let imported = BCryptImportKeyPair(
                algorithm,
                null_mut(),
                BCRYPT_ECCPUBLIC_BLOB,
                &mut handle,
                blob.as_ptr(),
                blob.len() as u32,
                0,
            ) == 0;
            let valid = imported
                && BCryptVerifySignature(handle, null(), hash.as_ptr(), 32, signature.as_ptr(), 64, 0) == 0;
            if imported {
                BCryptDestroyKey(handle);
            }
            BCryptCloseAlgorithmProvider(algorithm, 0);
            valid
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

    pub(super) fn get(_target: Target<'_>, _limit: usize) -> io::Result<Vec<u8>> {
        Err(io::ErrorKind::Unsupported.into())
    }

    pub(super) fn sha256(_data: &[u8]) -> [u8; 32] {
        [0; 32]
    }

    pub(super) fn verify_p256(_key: &[u8; 64], _message: &[u8], _signature: &[u8]) -> bool {
        false
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

    /// Made with `scripts/sign-release.ps1 -Message`, by the key whose public
    /// half is built in: .NET's signature format is the one CNG checks.
    #[cfg(windows)]
    #[test]
    fn a_signature_by_the_release_key_verifies_and_nothing_else_does() {
        let message = b"ChronoDesk update signature test";
        let signature = parse_hex("ed3168e53c9dcd3df3eba67c625261e56f0bed0bbfe7c8f28d3a82b97f53425150d374a98097296465044539b7b1650cb167bc9d3771ea6dee9af4f7a7453274").unwrap();
        assert!(imp::verify_p256(&RELEASE_KEY, message, &signature));
        assert!(!imp::verify_p256(&RELEASE_KEY, b"ChronoDesk update signature tesT", &signature), "other message");
        let mut forged = signature.clone();
        forged[10] ^= 1;
        assert!(!imp::verify_p256(&RELEASE_KEY, message, &forged), "altered signature");
        let mut other_key = RELEASE_KEY;
        other_key[0] ^= 1;
        assert!(!imp::verify_p256(&other_key, message, &signature), "another key");
        assert!(!imp::verify_p256(&RELEASE_KEY, message, &signature[..63]), "truncated");
    }

    #[cfg(windows)]
    #[test]
    fn sha256_matches_the_standard_vector() {
        let abc = imp::sha256(b"abc");
        assert_eq!(parse_hex("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad").unwrap(), abc);
    }

    /// A release as `sign-release.ps1` leaves it: checksums, their signature,
    /// and a setup that matches; then each part tampered with in turn.
    #[cfg(windows)]
    #[test]
    fn a_setup_installs_only_with_signed_checksums_that_it_matches() {
        let setup = b"fake setup, for the test";
        let hash = "2edf152799420a3c60462a7d8ab14c2dc3f623dd220d5d5a7d95a026af9eccc7";
        let sums = format!("{hash}  ChronoDesk-Setup.exe\n{hash}  ChronoDesk.exe\n");
        let signature = "5c64f7a1f3ada53e1cdfe91f6ba0b62abe0ecc1bd8ccea12e95e21b4ac1f696f16dcbaa5b43fd2445a3e5169ff7297a2a740cbd21d853de633c5cd1924813f1a\n";

        assert!(verified(sums.as_bytes(), signature.as_bytes(), setup, &RELEASE_KEY).is_ok());
        assert!(matches!(
            verified(sums.as_bytes(), signature.as_bytes(), b"fake setup, for the tesT", &RELEASE_KEY),
            Err(Refused::Checksum)
        ));
        let swapped = sums.replace(hash, &"0".repeat(64));
        assert!(matches!(verified(swapped.as_bytes(), signature.as_bytes(), setup, &RELEASE_KEY), Err(Refused::Signature)));
        assert!(matches!(verified(sums.as_bytes(), b"", setup, &RELEASE_KEY), Err(Refused::Signature)));
        assert!(matches!(verified(sums.as_bytes(), b"not hex", setup, &RELEASE_KEY), Err(Refused::Signature)));
    }

    #[test]
    fn checksums_are_found_by_file_name() {
        let sums = "aa  ChronoDesk-Setup.exe\nbb *ChronoDesk.exe\n";
        assert_eq!(checksum_for(sums, "ChronoDesk-Setup.exe"), Some(vec![0xaa]));
        assert_eq!(checksum_for(sums, "ChronoDesk.exe"), Some(vec![0xbb]));
        assert_eq!(checksum_for(sums, "ChronoDesk"), None);
    }

    /// The body of a download, after a redirect like GitHub's to its file host.
    #[cfg(windows)]
    #[test]
    fn a_download_follows_the_redirect_and_reads_the_whole_body() {
        use std::io::{Read as _, Write as _};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let body: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
        let sent = body.clone();
        let server = std::thread::spawn(move || {
            for answer in 0..2 {
                let (mut stream, _) = listener.accept().unwrap();
                let _ = stream.read(&mut [0u8; 2048]).unwrap();
                if answer == 0 {
                    let redirect = format!(
                        "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:{port}/files/setup\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    );
                    stream.write_all(redirect.as_bytes()).unwrap();
                } else {
                    let head = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", sent.len());
                    stream.write_all(head.as_bytes()).unwrap();
                    stream.write_all(&sent).unwrap();
                }
            }
        });
        let target = Target { secure: false, host: "127.0.0.1", port, path: "/o/r/releases/download/v9.9.9/setup" };
        assert_eq!(imp::get(target, 1 << 20).unwrap(), body);
        server.join().unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn a_download_larger_than_its_limit_is_refused() {
        use std::io::{Read as _, Write as _};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let _ = stream.read(&mut [0u8; 2048]).unwrap();
            let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100000\r\nConnection: close\r\n\r\n");
            let _ = stream.write_all(&[7u8; 100_000]);
        });
        let target = Target { secure: false, host: "127.0.0.1", port, path: "/big" };
        assert!(imp::get(target, 1000).is_err());
        server.join().unwrap();
    }

    /// The whole chain on the real release: published by CI, signed by hand,
    /// verified here. Run by hand with `--ignored` after signing a release.
    #[cfg(windows)]
    #[test]
    #[ignore = "network"]
    fn the_latest_release_downloads_and_verifies() {
        let path = download_setup(latest().unwrap()).unwrap();
        assert!(std::fs::metadata(&path).unwrap().len() > 1 << 20);
        std::fs::remove_file(path).unwrap();
    }

    /// Goes out to GitHub; run by hand with `--ignored`.
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
