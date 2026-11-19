//! Process CPU/memory and disk-space readings for the stats overlay and the
//! disk-space guard.

use std::path::Path;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::FILETIME;
use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS_EX};
use windows::Win32::System::SystemInformation::{GetSystemInfo, SYSTEM_INFO};
use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};
use windows::core::PCWSTR;

/// Free bytes on the volume holding `path` (walks up to an existing ancestor).
pub fn free_disk_bytes(path: &Path) -> Option<u64> {
    let existing = path.ancestors().find(|p| p.exists())?;
    let wide = crate::util::wide(&existing.to_string_lossy());
    let mut free = 0u64;
    // SAFETY: NUL-terminated path and valid out-pointer.
    unsafe { GetDiskFreeSpaceExW(PCWSTR(wide.as_ptr()), Some(&mut free), None, None) }.ok()?;
    Some(free)
}

/// Private bytes of this process.
pub fn memory_bytes() -> u64 {
    let mut counters = PROCESS_MEMORY_COUNTERS_EX { cb: size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32, ..Default::default() };
    // SAFETY: correctly sized struct with cb set; pseudo-handle for this process.
    unsafe {
        let _ = GetProcessMemoryInfo(GetCurrentProcess(), &mut counters as *mut _ as *mut _, counters.cb);
    }
    counters.PrivateUsage as u64
}

/// CPU usage of this process across all cores, as a percentage of the whole machine.
pub struct CpuMeter {
    last_cpu: u64,
    last_wall: Instant,
    cores: u32,
}

impl Default for CpuMeter {
    fn default() -> Self {
        let mut info = SYSTEM_INFO::default();
        // SAFETY: valid out-pointer.
        unsafe { GetSystemInfo(&mut info) };
        Self { last_cpu: process_cpu_100ns(), last_wall: Instant::now(), cores: info.dwNumberOfProcessors.max(1) }
    }
}

impl CpuMeter {
    pub fn sample(&mut self) -> f32 {
        let cpu = process_cpu_100ns();
        let now = Instant::now();
        let wall = now.duration_since(self.last_wall);
        let used = cpu.saturating_sub(self.last_cpu);
        self.last_cpu = cpu;
        self.last_wall = now;
        cpu_percent(used, wall, self.cores)
    }
}

fn cpu_percent(used_100ns: u64, elapsed: Duration, cores: u32) -> f32 {
    if elapsed.is_zero() { return 0.0; }
    // Process accounting and the wall clock are sampled separately. Their
    // different resolution can produce overshoot over short intervals.
    let available = elapsed.as_secs_f64() * 10_000_000.0 * f64::from(cores.max(1));
    (used_100ns as f64 / available * 100.0).clamp(0.0, 100.0) as f32
}

fn process_cpu_100ns() -> u64 {
    let (mut c, mut e, mut k, mut u) = (FILETIME::default(), FILETIME::default(), FILETIME::default(), FILETIME::default());
    // SAFETY: valid out-pointers; pseudo-handle for this process.
    if unsafe { GetProcessTimes(GetCurrentProcess(), &mut c, &mut e, &mut k, &mut u) }.is_err() {
        return 0;
    }
    let ft = |f: FILETIME| (u64::from(f.dwHighDateTime) << 32) | u64::from(f.dwLowDateTime);
    ft(k) + ft(u)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_percentage_handles_short_samples_and_accounting_overshoot() {
        assert_eq!(cpu_percent(0, Duration::from_secs(1), 4), 0.0);
        assert_eq!(cpu_percent(10_000_000, Duration::from_secs(1), 4), 25.0);
        assert_eq!(cpu_percent(40_000_000, Duration::from_secs(1), 4), 100.0);
        // Reproduce the approximately 102.7% reading observed on CI.
        assert_eq!(cpu_percent(2_053_933, Duration::from_millis(100), 2), 100.0);
        assert_eq!(cpu_percent(u64::MAX, Duration::from_nanos(1), 1), 100.0);
        assert_eq!(cpu_percent(1, Duration::ZERO, 1), 0.0);
        assert_eq!(cpu_percent(10_000_000, Duration::from_secs(1), 0), 100.0);
    }

    #[test]
    fn readings_are_sane() {
        assert!(memory_bytes() > 0);
        assert!(free_disk_bytes(&std::env::temp_dir().join("does/not/exist/yet")).is_some());
        let mut m = CpuMeter::default();
        let x: u64 = (0..2_000_000u64).map(|i| i % 7).sum();
        assert!(x > 0);
        let pct = m.sample();
        assert!((0.0..=100.0).contains(&pct), "{pct}");
    }
}
