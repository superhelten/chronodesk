//! The few registry calls the app needs, all under `HKEY_CURRENT_USER`: the
//! `Run` key for "Start with Windows" and the uninstall entry the installer
//! writes. Nothing here needs elevation.

#[cfg(windows)]
mod imp {
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

    pub fn write_dword(key: &str, value: &str, number: u32) -> io::Result<()> {
        use windows_sys::Win32::System::Registry::REG_DWORD;
        let (key, value) = (wide(key), wide(value));
        // SAFETY: both strings are NUL-terminated; the data is the four bytes of `number`.
        let status = unsafe {
            RegSetKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), value.as_ptr(), REG_DWORD, (&raw const number).cast(), 4)
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

    /// The key and everything under it; a key that is not there is a success.
    pub fn delete_tree(key: &str) -> io::Result<()> {
        use windows_sys::Win32::System::Registry::{RegDeleteKeyW, RegDeleteTreeW};
        let key = wide(key);
        // SAFETY: the string is NUL-terminated and outlives the calls.
        // `RegDeleteTreeW` empties the key; the key itself goes with the second.
        let status = unsafe {
            match RegDeleteTreeW(HKEY_CURRENT_USER, key.as_ptr()) {
                ERROR_SUCCESS => RegDeleteKeyW(HKEY_CURRENT_USER, key.as_ptr()),
                other => other,
            }
        };
        match status {
            ERROR_SUCCESS | ERROR_FILE_NOT_FOUND => Ok(()),
            other => Err(io::Error::from_raw_os_error(other as i32)),
        }
    }
}

#[cfg(not(windows))]
mod imp {
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

    pub fn write_dword(_key: &str, _value: &str, _number: u32) -> io::Result<()> {
        Err(io::ErrorKind::Unsupported.into())
    }

    pub fn delete_tree(_key: &str) -> io::Result<()> {
        Ok(())
    }
}

pub use imp::*;
