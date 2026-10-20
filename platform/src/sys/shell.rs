//! Shell helpers: known folders and launching an elevated child process.

use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::path::{Path, PathBuf};

use windows::Win32::Foundation::WAIT_OBJECT_0;
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::System::Threading::{GetExitCodeProcess, INFINITE, WaitForSingleObject};
use windows::Win32::UI::Shell::{
    KF_FLAG_DEFAULT, SEE_MASK_FLAG_NO_UI, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, SHGetKnownFolderPath,
    ShellExecuteExW,
};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows::core::{GUID, PCWSTR, w};

use super::{OwnedHandle, wide};

/// Resolves a known folder such as `FOLDERID_ProgramFiles`.
pub fn known_folder(id: &GUID) -> windows::core::Result<PathBuf> {
    // SAFETY: `id` points to a valid GUID; a None token means "current user".
    let raw = unsafe { SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, None)? };
    // SAFETY: on success `raw` is a NUL-terminated string we own.
    let path = PathBuf::from(OsString::from_wide(unsafe { raw.as_wide() }));
    // SAFETY: allocated by SHGetKnownFolderPath with CoTaskMemAlloc; freed once.
    unsafe { CoTaskMemFree(Some(raw.0.cast())) };
    Ok(path)
}

/// Starts `exe` with the "runas" verb (one UAC prompt), waits for it to exit
/// and returns its exit code. Blocks the calling thread until then.
///
/// A declined prompt surfaces as an error with `HRESULT_FROM_WIN32(ERROR_CANCELLED)`.
pub fn run_elevated_and_wait(exe: &Path, parameters: &str) -> windows::core::Result<u32> {
    let file = wide(exe);
    let params = wide(parameters);
    let dir = exe.parent().map(wide);
    let mut info = SHELLEXECUTEINFOW {
        cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_FLAG_NO_UI,
        lpVerb: w!("runas"),
        lpFile: PCWSTR(file.as_ptr()),
        lpParameters: PCWSTR(params.as_ptr()),
        lpDirectory: dir.as_ref().map_or(PCWSTR::null(), |d| PCWSTR(d.as_ptr())),
        nShow: SW_SHOWNORMAL.0,
        ..Default::default()
    };
    // SAFETY: `info` is fully initialized and every string it points to outlives the call.
    unsafe { ShellExecuteExW(&mut info)? };
    if info.hProcess.is_invalid() {
        // The shell handed the request to an existing process; nothing to wait for.
        return Ok(0);
    }
    let process = OwnedHandle(info.hProcess);
    // SAFETY: `process` is a valid process handle we own (SEE_MASK_NOCLOSEPROCESS).
    let waited = unsafe { WaitForSingleObject(process.0, INFINITE) };
    if waited != WAIT_OBJECT_0 {
        return Err(windows::core::Error::from_thread());
    }
    let mut code = 0u32;
    // SAFETY: valid process handle and out-pointer.
    unsafe { GetExitCodeProcess(process.0, &mut code)? };
    Ok(code)
}
