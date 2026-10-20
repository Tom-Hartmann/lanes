//! Thin safe wrappers over the Win32 calls this crate needs. Apart from the
//! Task Scheduler COM calls in `admin::scheduler`, every `unsafe` block of the
//! crate lives below this module.

pub mod acl;
pub mod com;
pub mod registry;
pub mod shell;
pub mod token;
pub mod trust;

use std::ffi::{OsStr, OsString};
use std::os::windows::ffi::{OsStrExt, OsStringExt};

use windows::Win32::Foundation::{CloseHandle, HANDLE, HLOCAL, LocalFree};
use windows::core::PWSTR;

/// Encodes a string as a NUL-terminated UTF-16 buffer for `PCWSTR` arguments.
/// The caller keeps the buffer alive for as long as the pointer is in use.
pub fn wide(s: impl AsRef<OsStr>) -> Vec<u16> {
    s.as_ref().encode_wide().chain(Some(0)).collect()
}

/// An owned kernel handle, closed on drop.
pub struct OwnedHandle(pub HANDLE);

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        // SAFETY: the handle came from a successful Open/Create call and is closed once.
        let _ = unsafe { CloseHandle(self.0) };
    }
}

/// A string allocated by the system with `LocalAlloc`, freed on drop.
struct LocalWideString(PWSTR);

impl LocalWideString {
    fn to_os_string(&self) -> OsString {
        if self.0.is_null() {
            return OsString::new();
        }
        // SAFETY: non-null and NUL-terminated, as returned by the API that allocated it.
        OsString::from_wide(unsafe { self.0.as_wide() })
    }
}

impl Drop for LocalWideString {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: allocated with LocalAlloc by the API that returned it; freed once.
            unsafe { LocalFree(Some(HLOCAL(self.0.0.cast()))) };
        }
    }
}
