//! Audio: WASAPI capture, the lane mixer and DSP.

pub mod chime;
pub mod devices;
pub mod dsp;
pub mod mixer;
pub mod wasapi;

use std::sync::OnceLock;

/// Per-process loopback needs Windows 10 2004 (build 19041).
pub fn process_loopback_supported() -> bool {
    os_build() >= 19041
}

/// Windows build number via RtlGetVersion (GetVersionEx lies to unmanifested apps).
pub fn os_build() -> u32 {
    static BUILD: OnceLock<u32> = OnceLock::new();
    *BUILD.get_or_init(|| {
        #[repr(C)]
        struct OsVersionInfo {
            size: u32,
            major: u32,
            minor: u32,
            build: u32,
            platform: u32,
            csd: [u16; 128],
        }
        #[link(name = "ntdll")]
        unsafe extern "system" {
            fn RtlGetVersion(info: *mut OsVersionInfo) -> i32;
        }
        let mut info = OsVersionInfo { size: size_of::<OsVersionInfo>() as u32, major: 0, minor: 0, build: 0, platform: 0, csd: [0; 128] };
        // SAFETY: correctly sized struct with its size field set.
        if unsafe { RtlGetVersion(&mut info) } == 0 { info.build } else { 0 }
    })
}
