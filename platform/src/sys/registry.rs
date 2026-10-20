//! String values under `HKEY_CURRENT_USER`.

use std::ffi::{OsStr, OsString};
use std::io;
use std::os::windows::ffi::OsStringExt;

use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_MORE_DATA, ERROR_SUCCESS, WIN32_ERROR};
use windows::Win32::System::Registry::{
    HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW,
};
use windows::core::PCWSTR;

use super::wide;

fn to_io(err: WIN32_ERROR) -> io::Error {
    io::Error::from_raw_os_error(err.0 as i32)
}

/// Reads a `REG_SZ` value. `Ok(None)` if the key or value does not exist.
pub fn get_string(subkey: &str, name: &str) -> io::Result<Option<OsString>> {
    let subkey = wide(subkey);
    let name = wide(name);
    // The value may change between the size query and the read; retry a few times.
    for _ in 0..4 {
        let mut bytes = 0u32;
        // SAFETY: both strings are NUL-terminated and outlive the call; a None
        // data pointer asks only for the size.
        let err = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                PCWSTR(subkey.as_ptr()),
                PCWSTR(name.as_ptr()),
                RRF_RT_REG_SZ,
                None,
                None,
                Some(&mut bytes),
            )
        };
        if err == ERROR_FILE_NOT_FOUND {
            return Ok(None);
        }
        if err != ERROR_SUCCESS {
            return Err(to_io(err));
        }
        let mut buf = vec![0u16; (bytes as usize).div_ceil(2)];
        let mut bytes = (buf.len() * 2) as u32;
        // SAFETY: `buf` is writable for `bytes` bytes; RegGetValueW NUL-terminates REG_SZ data.
        let err = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                PCWSTR(subkey.as_ptr()),
                PCWSTR(name.as_ptr()),
                RRF_RT_REG_SZ,
                None,
                Some(buf.as_mut_ptr().cast()),
                Some(&mut bytes),
            )
        };
        match err {
            ERROR_SUCCESS => {
                buf.truncate((bytes as usize) / 2);
                while buf.last() == Some(&0) {
                    buf.pop();
                }
                return Ok(Some(OsString::from_wide(&buf)));
            }
            ERROR_MORE_DATA => continue,
            ERROR_FILE_NOT_FOUND => return Ok(None),
            err => return Err(to_io(err)),
        }
    }
    Err(to_io(ERROR_MORE_DATA))
}

/// Writes a `REG_SZ` value, creating the key if needed.
pub fn set_string(subkey: &str, name: &str, value: &OsStr) -> io::Result<()> {
    let subkey = wide(subkey);
    let name = wide(name);
    let data = wide(value);
    let bytes = u32::try_from(data.len() * 2).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
    // SAFETY: all buffers are NUL-terminated and outlive the call; `bytes`
    // covers `data` including its terminator, as REG_SZ requires.
    let err = unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            PCWSTR(subkey.as_ptr()),
            PCWSTR(name.as_ptr()),
            REG_SZ.0,
            Some(data.as_ptr().cast()),
            bytes,
        )
    };
    if err == ERROR_SUCCESS { Ok(()) } else { Err(to_io(err)) }
}

/// Deletes a value. Succeeds if it (or its key) does not exist.
pub fn delete_value(subkey: &str, name: &str) -> io::Result<()> {
    let subkey = wide(subkey);
    let name = wide(name);
    // SAFETY: both strings are NUL-terminated and outlive the call.
    let err = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, PCWSTR(subkey.as_ptr()), PCWSTR(name.as_ptr())) };
    match err {
        ERROR_SUCCESS | ERROR_FILE_NOT_FOUND => Ok(()),
        err => Err(to_io(err)),
    }
}
