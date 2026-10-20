//! The single monotonic clock shared by video, audio and input events.
//!
//! Every frame, audio packet and hotkey press is stamped with
//! [`Timestamp::now`], which reads `QueryPerformanceCounter`. WGC frames
//! (`SystemRelativeTime`) and WASAPI packets (`qpcPosition`) already report QPC
//! time, so all streams share one timeline without any drift correction.

use std::sync::OnceLock;
use std::time::Duration;

/// A point on the QPC timeline, stored in 100 ns ticks (the unit used by
/// WGC, WASAPI and Media Foundation).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Timestamp(pub i64);

impl Timestamp {
    /// Ticks per second for the 100 ns unit.
    pub const TICKS_PER_SEC: i64 = 10_000_000;

    /// The current time.
    pub fn now() -> Self {
        Self::from_qpc(sys::counter())
    }

    /// Converts a raw `QueryPerformanceCounter` value (for example the
    /// `qpcPosition` WASAPI reports) to a timestamp.
    pub fn from_qpc(counter: i64) -> Self {
        let freq = frequency();
        // Split the multiplication so it cannot overflow for large uptimes.
        let secs = counter / freq;
        let rem = counter % freq;
        Self(secs * Self::TICKS_PER_SEC + rem * Self::TICKS_PER_SEC / freq)
    }

    /// The time elapsed from `earlier` to `self`, or zero if `earlier` is later.
    pub fn saturating_since(self, earlier: Timestamp) -> Duration {
        let ticks = self.0.saturating_sub(earlier.0).max(0) as u64;
        Duration::from_nanos(ticks * 100)
    }

    /// The timestamp as whole milliseconds, for logs and UI.
    pub fn as_millis(self) -> i64 {
        self.0 / 10_000
    }
}

fn frequency() -> i64 {
    static FREQ: OnceLock<i64> = OnceLock::new();
    *FREQ.get_or_init(sys::frequency)
}

#[cfg(windows)]
mod sys {
    use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};

    pub fn counter() -> i64 {
        let mut value = 0i64;
        // SAFETY: valid out-pointer; QPC cannot fail on Windows XP and later.
        let _ = unsafe { QueryPerformanceCounter(&mut value) };
        value
    }

    pub fn frequency() -> i64 {
        let mut value = 0i64;
        // SAFETY: valid out-pointer; the frequency is fixed at boot and never zero.
        let _ = unsafe { QueryPerformanceFrequency(&mut value) };
        value.max(1)
    }
}

/// Fallback so the crate still builds and tests on non-Windows hosts.
#[cfg(not(windows))]
mod sys {
    use std::sync::OnceLock;
    use std::time::Instant;

    pub fn counter() -> i64 {
        static START: OnceLock<Instant> = OnceLock::new();
        START.get_or_init(Instant::now).elapsed().as_nanos() as i64
    }

    pub fn frequency() -> i64 {
        1_000_000_000
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn now_is_monotonic() {
        let mut prev = Timestamp::now();
        for _ in 0..10_000 {
            let next = Timestamp::now();
            assert!(next >= prev);
            prev = next;
        }
    }

    #[test]
    fn tracks_wall_time() {
        let start = Timestamp::now();
        std::thread::sleep(Duration::from_millis(50));
        let elapsed = Timestamp::now().saturating_since(start);
        assert!(elapsed >= Duration::from_millis(45), "{elapsed:?}");
        assert!(elapsed < Duration::from_millis(500), "{elapsed:?}");
    }

    #[test]
    fn saturating_since_never_negative() {
        assert_eq!(Timestamp(5).saturating_since(Timestamp(10)), Duration::ZERO);
        assert_eq!(Timestamp(10).saturating_since(Timestamp(5)), Duration::from_nanos(500));
    }

    #[test]
    fn from_qpc_converts_whole_seconds() {
        let freq = frequency();
        assert_eq!(Timestamp::from_qpc(freq * 3), Timestamp(3 * Timestamp::TICKS_PER_SEC));
    }
}
