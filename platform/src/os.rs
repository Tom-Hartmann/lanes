//! Operating system version queries.

use windows::Wdk::System::SystemServices::RtlGetVersion;
use windows::Win32::System::SystemInformation::OSVERSIONINFOW;

/// First Windows 10 build (2004 / 20H1) with process-loopback audio capture
/// (`AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK`).
const PROCESS_LOOPBACK_MIN_BUILD: u32 = 19041;

/// The OS build number, e.g. 26100. Returns 0 if it cannot be read.
///
/// Uses `RtlGetVersion` from ntdll: `GetVersionEx` reports the version the
/// executable's manifest claims compatibility with, not the real one.
pub fn build_number() -> u32 {
    let mut info = OSVERSIONINFOW { dwOSVersionInfoSize: size_of::<OSVERSIONINFOW>() as u32, ..Default::default() };
    // SAFETY: valid, correctly sized out-structure.
    let status = unsafe { RtlGetVersion(&mut info) };
    if status.is_ok() { info.dwBuildNumber } else { 0 }
}

/// True if per-process audio loopback capture is available (build 19041+).
pub fn supports_process_loopback() -> bool {
    build_number() >= PROCESS_LOOPBACK_MIN_BUILD
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_plausible_build_number() {
        // Lanes targets Windows 10/11; every supported build is above 10240.
        let build = build_number();
        assert!(build >= 10240, "build {build}");
        assert_eq!(supports_process_loopback(), build >= 19041);
    }
}
