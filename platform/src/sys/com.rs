//! COM apartment management.

use std::marker::PhantomData;

use windows::Win32::Foundation::RPC_E_CHANGED_MODE;
use windows::Win32::System::Com::{
    CoInitializeEx, CoInitializeSecurity, CoUninitialize, COINIT_MULTITHREADED, EOAC_NONE,
    RPC_C_AUTHN_LEVEL_PKT_PRIVACY, RPC_C_IMP_LEVEL_IMPERSONATE,
};

/// Keeps COM initialized on the current thread for its lifetime.
///
/// Joins the multithreaded apartment. If the thread already lives in a
/// single-threaded apartment (Tauri's main thread does), COM reports
/// `RPC_E_CHANGED_MODE`; COM is usable as it is, so we just don't uninitialize.
/// Not `Send`: `CoUninitialize` must run on the thread that initialized.
pub struct ComApartment {
    uninit: bool,
    _not_send: PhantomData<*const ()>,
}

impl ComApartment {
    pub fn init() -> windows::core::Result<Self> {
        // SAFETY: no pointers involved; a successful call (S_OK or S_FALSE) is
        // balanced by CoUninitialize in Drop on this same thread.
        let hr = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        if hr == RPC_E_CHANGED_MODE {
            return Ok(Self { uninit: false, _not_send: PhantomData });
        }
        hr.ok()?;
        Ok(Self { uninit: true, _not_send: PhantomData })
    }
}

impl Drop for ComApartment {
    fn drop(&mut self) {
        if self.uninit {
            // SAFETY: balances the successful CoInitializeEx in `init` on the same thread (the type is !Send).
            unsafe { CoUninitialize() };
        }
    }
}

/// Sets process-wide COM security so calls into the Task Scheduler service
/// may impersonate us (it registers the task as the calling user).
///
/// Only the short-lived `--admin-task` helper calls this, before anything else
/// in the process touches COM. Inside the full app it would be too late
/// (`RPC_E_TOO_LATE`) and would affect WebView2, so it is never called there;
/// failures are logged and otherwise ignored because the defaults usually work.
pub fn init_process_security() {
    // SAFETY: all pointer arguments are None; must be called after CoInitializeEx,
    // which the caller guarantees by holding a ComApartment.
    let result = unsafe {
        CoInitializeSecurity(
            None,
            -1,
            None,
            None,
            RPC_C_AUTHN_LEVEL_PKT_PRIVACY,
            RPC_C_IMP_LEVEL_IMPERSONATE,
            None,
            EOAC_NONE,
            None,
        )
    };
    if let Err(e) = result {
        log::debug!("CoInitializeSecurity: {e}");
    }
}
