//! Facts about the current process token.

use windows::Win32::Foundation::{E_UNEXPECTED, HANDLE};
use windows::Win32::Security::Authorization::ConvertSidToStringSidW;
use windows::Win32::Security::{
    GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TOKEN_USER, TokenElevation, TokenUser,
};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows::core::PWSTR;

use super::{LocalWideString, OwnedHandle};

fn process_token() -> windows::core::Result<OwnedHandle> {
    let mut token = HANDLE::default();
    // SAFETY: the pseudo-handle of our own process is always valid; the token
    // handle is owned (and closed) by OwnedHandle.
    unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token)? };
    Ok(OwnedHandle(token))
}

/// True if the process runs with an elevated (administrator) token.
pub fn is_elevated() -> bool {
    let Ok(token) = process_token() else {
        return false;
    };
    let mut elevation = TOKEN_ELEVATION::default();
    let mut len = 0u32;
    // SAFETY: the out-buffer is a TOKEN_ELEVATION and its exact size is passed.
    let ok = unsafe {
        GetTokenInformation(
            token.0,
            TokenElevation,
            Some((&raw mut elevation).cast()),
            size_of::<TOKEN_ELEVATION>() as u32,
            &mut len,
        )
    };
    ok.is_ok() && elevation.TokenIsElevated != 0
}

/// The string SID (`S-1-5-21-...`) of the account the process runs as.
///
/// Under UAC the elevated and the filtered token belong to the same account,
/// so an elevated helper reports the same SID as the app that launched it,
/// unless the user typed *another* account's credentials into the prompt.
pub fn current_user_sid() -> windows::core::Result<String> {
    let token = process_token()?;
    let mut len = 0u32;
    // SAFETY: a None buffer only queries the required size (the call "fails" with
    // ERROR_INSUFFICIENT_BUFFER, which is expected).
    let _ = unsafe { GetTokenInformation(token.0, TokenUser, None, 0, &mut len) };
    // u64 elements keep the buffer suitably aligned for TOKEN_USER.
    let mut buf = vec![0u64; (len as usize).div_ceil(8).max(1)];
    // SAFETY: `buf` is writable for at least `len` bytes and 8-byte aligned.
    unsafe { GetTokenInformation(token.0, TokenUser, Some(buf.as_mut_ptr().cast()), len, &mut len)? };
    // SAFETY: on success the buffer starts with a TOKEN_USER whose SID points into `buf`, which is still alive.
    let user = unsafe { &*buf.as_ptr().cast::<TOKEN_USER>() };
    let mut text = PWSTR::null();
    // SAFETY: the SID is valid (see above); the output string is freed by LocalWideString.
    unsafe { ConvertSidToStringSidW(user.User.Sid, &mut text)? };
    LocalWideString(text).to_os_string().into_string().map_err(|_| E_UNEXPECTED.into())
}
