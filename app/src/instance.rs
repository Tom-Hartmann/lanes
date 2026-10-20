//! Single-instance lock and elevation detection.
//!
//! Only one Lanes process may run per session, whether it is elevated or not.
//! The lock is a named mutex in the session-local namespace. A second instance
//! signals a named event so the first one shows its window, then exits.
//!
//! Both objects get an explicit security descriptor with a *medium* integrity
//! label. Without it, objects created by an elevated instance carry a high
//! label, and a standard instance could neither open the event to signal it
//! nor tell "already running" apart from other errors.

use std::thread;

use windows::Win32::Foundation::{
    CloseHandle, DUPLICATE_SAME_ACCESS, DuplicateHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, HLOCAL, LocalFree, WAIT_OBJECT_0,
};
use windows::Win32::Security::Authorization::{
    ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows::Win32::Security::{
    GetTokenInformation, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation,
};
use windows::Win32::System::Threading::{
    CreateEventW, CreateMutexW, EVENT_MODIFY_STATE, GetCurrentProcess, INFINITE, OpenEventW, OpenProcessToken,
    SetEvent, WaitForSingleObject,
};
use windows::Win32::UI::WindowsAndMessaging::{ASFW_ANY, AllowSetForegroundWindow};
use windows::core::w;

/// SYSTEM, Administrators and authenticated users get full access; medium
/// integrity processes may write (signal). The namespace is per session, so
/// only processes in the user's own logon session can see these objects.
const SDDL: windows::core::PCWSTR = w!("D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GA;;;AU)S:(ML;;NW;;;ME)");
const MUTEX_NAME: windows::core::PCWSTR = w!("Local\\dev.lanes.app.instance");
const SHOW_EVENT_NAME: windows::core::PCWSTR = w!("Local\\dev.lanes.app.show");

/// Holds the instance lock for the life of the process.
pub struct InstanceLock {
    _mutex: OwnedHandle,
    show_event: OwnedHandle,
}

pub enum Acquire {
    Primary(InstanceLock),
    AlreadyRunning,
}

/// Takes the single-instance lock.
pub fn acquire() -> windows::core::Result<Acquire> {
    let sd = SecurityDescriptor::from_sddl(SDDL)?;
    let sa = sd.attributes();
    // SAFETY: `sa` and the descriptor it points to outlive the call.
    let mutex = match unsafe { CreateMutexW(Some(&sa), false, MUTEX_NAME) } {
        Ok(h) => OwnedHandle(h),
        // The mutex exists but we may not open it: another instance at a
        // different integrity level holds it.
        Err(e) if e.code() == windows::Win32::Foundation::E_ACCESSDENIED => return Ok(Acquire::AlreadyRunning),
        Err(e) => return Err(e),
    };
    // SAFETY: reads the thread's last-error value set by CreateMutexW.
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        return Ok(Acquire::AlreadyRunning);
    }
    // SAFETY: as above. Auto-reset, initially unsignaled.
    let show_event = OwnedHandle(unsafe { CreateEventW(Some(&sa), false, false, SHOW_EVENT_NAME)? });
    Ok(Acquire::Primary(InstanceLock { _mutex: mutex, show_event }))
}

/// Asks the running instance to show its window. Called by a second instance before it exits.
pub fn signal_existing() {
    // SAFETY: plain value arguments; the handle is closed by OwnedHandle.
    unsafe {
        // Let the running instance take the foreground, which Windows would otherwise refuse.
        let _ = AllowSetForegroundWindow(ASFW_ANY);
        if let Ok(event) = OpenEventW(EVENT_MODIFY_STATE, false, SHOW_EVENT_NAME) {
            let event = OwnedHandle(event);
            let _ = SetEvent(event.0);
        }
    }
}

impl InstanceLock {
    /// Calls `on_show` (on a background thread) whenever a second instance starts.
    pub fn listen_for_show(&self, on_show: impl Fn() + Send + 'static) -> std::io::Result<()> {
        let event = OwnedHandle(duplicate(self.show_event.0)?);
        thread::Builder::new().name("lanes-instance".into()).spawn(move || {
            // SAFETY: the thread owns its own duplicate of the event handle.
            while unsafe { WaitForSingleObject(event.raw(), INFINITE) } == WAIT_OBJECT_0 {
                on_show();
            }
        })?;
        Ok(())
    }
}

fn duplicate(handle: HANDLE) -> std::io::Result<HANDLE> {
    let mut copy = HANDLE::default();
    // SAFETY: duplicates a valid handle within our own process; the copy is owned by the caller.
    unsafe {
        let process = GetCurrentProcess();
        DuplicateHandle(process, handle, process, &mut copy, 0, false, DUPLICATE_SAME_ACCESS)?;
    }
    Ok(copy)
}

/// True if the process runs with an elevated (administrator) token.
pub fn is_elevated() -> bool {
    // SAFETY: valid out-pointers sized for TOKEN_ELEVATION; the token handle is closed by OwnedHandle.
    unsafe {
        let mut token = HANDLE::default();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).is_err() {
            return false;
        }
        let token = OwnedHandle(token);
        let mut elevation = TOKEN_ELEVATION::default();
        let mut len = 0u32;
        GetTokenInformation(
            token.0,
            TokenElevation,
            Some(&mut elevation as *mut _ as *mut _),
            size_of::<TOKEN_ELEVATION>() as u32,
            &mut len,
        )
        .is_ok()
            && elevation.TokenIsElevated != 0
    }
}

/// An owned kernel handle, closed on drop.
struct OwnedHandle(HANDLE);

// SAFETY: kernel object handles are process-wide and may be used from any
// thread; the wrapper only exposes them for wait/signal calls.
unsafe impl Send for OwnedHandle {}
unsafe impl Sync for OwnedHandle {}

impl OwnedHandle {
    fn raw(&self) -> HANDLE {
        self.0
    }
}

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        // SAFETY: the handle was returned by a successful Create/Open call and is closed once.
        let _ = unsafe { CloseHandle(self.0) };
    }
}

struct SecurityDescriptor(PSECURITY_DESCRIPTOR);

impl SecurityDescriptor {
    fn from_sddl(sddl: windows::core::PCWSTR) -> windows::core::Result<Self> {
        let mut sd = PSECURITY_DESCRIPTOR::default();
        // SAFETY: valid out-pointer; the descriptor is freed with LocalFree in Drop.
        unsafe { ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl, SDDL_REVISION_1, &mut sd, None)? };
        Ok(Self(sd))
    }

    fn attributes(&self) -> SECURITY_ATTRIBUTES {
        SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: self.0.0,
            bInheritHandle: false.into(),
        }
    }
}

impl Drop for SecurityDescriptor {
    fn drop(&mut self) {
        // SAFETY: allocated by ConvertStringSecurityDescriptorToSecurityDescriptorW.
        unsafe { LocalFree(Some(HLOCAL(self.0.0))) };
    }
}
