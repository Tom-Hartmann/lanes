//! Threading and COM helpers shared by the capture, audio and encode paths.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize};
use windows::Win32::System::Threading::{
    AvRevertMmThreadCharacteristics, AvSetMmThreadCharacteristicsW, CREATE_WAITABLE_TIMER_HIGH_RESOLUTION,
    CreateWaitableTimerExW, GetCurrentThread, INFINITE, SetThreadPriority, SetWaitableTimer, THREAD_PRIORITY,
    TIMER_ALL_ACCESS, WaitForSingleObject,
};
use windows::core::w;

/// Scheduling class for a worker thread.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Priority {
    /// Background work: thumbnails, remuxing, clip writing.
    Low,
    Normal,
    /// Encoders and capture: above normal so a busy game cannot starve them.
    AboveNormal,
    /// Audio: registered with MMCSS ("Pro Audio") for glitch-free capture.
    Audio,
}

/// Spawns a named worker. Panics inside `f` are caught and logged instead of
/// tearing down the process; the closure's result is `None` in that case.
pub fn spawn<T: Send + 'static>(
    name: &str,
    priority: Priority,
    f: impl FnOnce() -> T + Send + 'static,
) -> std::io::Result<JoinHandle<Option<T>>> {
    let thread_name = name.to_owned();
    thread::Builder::new().name(name.to_owned()).spawn(move || {
        let _mmcss = apply_priority(priority);
        match catch_unwind(AssertUnwindSafe(f)) {
            Ok(v) => Some(v),
            Err(panic) => {
                let msg = panic
                    .downcast_ref::<&str>()
                    .map(|s| s.to_string())
                    .or_else(|| panic.downcast_ref::<String>().cloned())
                    .unwrap_or_else(|| "unknown panic".into());
                log::error!("thread {thread_name} panicked: {msg}");
                None
            }
        }
    })
}

fn apply_priority(priority: Priority) -> Option<MmcssGuard> {
    let level = match priority {
        Priority::Low => -1,      // THREAD_PRIORITY_BELOW_NORMAL
        Priority::Normal => 0,    // THREAD_PRIORITY_NORMAL
        Priority::AboveNormal => 1, // THREAD_PRIORITY_ABOVE_NORMAL
        Priority::Audio => return MmcssGuard::register(),
    };
    // SAFETY: pseudo-handle of the current thread; plain value argument.
    let _ = unsafe { SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY(level)) };
    None
}

/// MMCSS registration for the current thread, reverted on drop.
struct MmcssGuard(HANDLE);

impl MmcssGuard {
    fn register() -> Option<Self> {
        let mut index = 0u32;
        // SAFETY: valid task name and out-pointer.
        match unsafe { AvSetMmThreadCharacteristicsW(w!("Pro Audio"), &mut index) } {
            Ok(h) => Some(Self(h)),
            Err(e) => {
                log::warn!("MMCSS registration failed: {e}");
                None
            }
        }
    }
}

impl Drop for MmcssGuard {
    fn drop(&mut self) {
        // SAFETY: handle came from AvSetMmThreadCharacteristicsW on this thread.
        let _ = unsafe { AvRevertMmThreadCharacteristics(self.0) };
    }
}

/// COM initialised as multithreaded apartment for the current thread.
pub struct ComGuard {
    owned: bool,
}

impl ComGuard {
    pub fn mta() -> Self {
        // SAFETY: balanced by CoUninitialize in Drop when this call succeeded.
        let hr = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        // RPC_E_CHANGED_MODE: the thread is already STA; COM still works for our use.
        Self { owned: hr.is_ok() }
    }
}

impl Drop for ComGuard {
    fn drop(&mut self) {
        if self.owned {
            // SAFETY: matches the successful CoInitializeEx above.
            unsafe { CoUninitialize() };
        }
    }
}

/// A high-resolution waitable timer for frame pacing. Sleeping on it costs
/// no CPU and is accurate to well under a millisecond on Windows 10 1803+.
pub struct PacingTimer(HANDLE);

// SAFETY: a kernel handle; the timer is only waited on by its owning thread.
unsafe impl Send for PacingTimer {}

impl PacingTimer {
    pub fn new() -> windows::core::Result<Self> {
        // SAFETY: no name, default security; the handle is closed in Drop.
        let h = unsafe { CreateWaitableTimerExW(None, None, CREATE_WAITABLE_TIMER_HIGH_RESOLUTION, TIMER_ALL_ACCESS.0) }?;
        Ok(Self(h))
    }

    /// Sleeps for `d` (returns immediately for zero).
    pub fn sleep(&self, d: Duration) {
        if d.is_zero() {
            return;
        }
        // Negative = relative time in 100 ns units.
        let due = -((d.as_nanos() / 100).max(1) as i64);
        // SAFETY: valid timer handle and due-time pointer.
        unsafe {
            if SetWaitableTimer(self.0, &due, 0, None, None, false).is_ok() {
                let _ = WaitForSingleObject(self.0, INFINITE) == WAIT_OBJECT_0;
            } else {
                thread::sleep(d);
            }
        }
    }
}

impl Drop for PacingTimer {
    fn drop(&mut self) {
        // SAFETY: handle from CreateWaitableTimerExW, closed once.
        let _ = unsafe { CloseHandle(self.0) };
    }
}

/// Converts decibels to a linear gain factor.
pub fn db_to_gain(db: f32) -> f32 {
    if db <= -100.0 { 0.0 } else { 10f32.powf(db / 20.0) }
}

/// Converts a linear amplitude to dBFS, floored at -100.
pub fn gain_to_db(gain: f32) -> f32 {
    if gain <= 1e-5 { -100.0 } else { (20.0 * gain.log10()).max(-100.0) }
}

/// Wide-string helper for Win32 APIs.
pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Reads a NUL-terminated (or full-length) UTF-16 buffer.
pub fn from_wide(buf: &[u16]) -> String {
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..len])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn db_conversions() {
        assert!((db_to_gain(0.0) - 1.0).abs() < 1e-6);
        assert!((db_to_gain(-6.0) - 0.501).abs() < 1e-3);
        assert_eq!(db_to_gain(-200.0), 0.0);
        assert!((gain_to_db(1.0)).abs() < 1e-6);
        assert_eq!(gain_to_db(0.0), -100.0);
    }

    #[test]
    fn spawn_catches_panics() {
        let h = spawn("t", Priority::Normal, || -> u32 { panic!("boom") }).unwrap();
        assert_eq!(h.join().unwrap(), None);
        let h = spawn("t", Priority::Low, || 7).unwrap();
        assert_eq!(h.join().unwrap(), Some(7));
    }

    #[test]
    fn pacing_timer_sleeps() {
        let t = PacingTimer::new().unwrap();
        let start = std::time::Instant::now();
        t.sleep(Duration::from_millis(5));
        assert!(start.elapsed() >= Duration::from_millis(4));
    }

    #[test]
    fn wide_roundtrip() {
        assert_eq!(from_wide(&wide("héllo")), "héllo");
    }
}
