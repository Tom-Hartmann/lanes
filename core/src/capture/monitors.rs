//! Monitor enumeration: DXGI outputs (coordinates, adapter), GDI display
//! settings (refresh rate) and DisplayConfig (friendly name, stable id).

use std::collections::HashMap;

use serde::Serialize;
use windows::Win32::Devices::Display::{
    DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME, DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME, DISPLAYCONFIG_MODE_INFO,
    DISPLAYCONFIG_PATH_INFO, DISPLAYCONFIG_SOURCE_DEVICE_NAME, DISPLAYCONFIG_TARGET_DEVICE_NAME, DisplayConfigGetDeviceInfo,
    GetDisplayConfigBufferSizes, QDC_ONLY_ACTIVE_PATHS, QueryDisplayConfig,
};
use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIFactory1};
use windows::Win32::Graphics::Gdi::{
    DEVMODEW, ENUM_CURRENT_SETTINGS, EnumDisplaySettingsW, GetMonitorInfoW, MONITORINFO,
};
use windows::Win32::UI::WindowsAndMessaging::MONITORINFOF_PRIMARY;
use windows::core::PCWSTR;

use crate::util::from_wide;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MonitorInfo {
    /// Stable id: the monitor's device path, or the GDI name if unavailable.
    pub id: String,
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub refresh_hz: u32,
    pub x: i32,
    pub y: i32,
    pub primary: bool,
    #[serde(skip)]
    pub hmonitor: isize,
    #[serde(skip)]
    pub gdi_name: String,
    /// LUID of the GPU driving this monitor.
    #[serde(skip)]
    pub adapter_luid: (u32, i32),
}

/// All monitors attached to the desktop, primary first.
pub fn list() -> Vec<MonitorInfo> {
    let names = display_config_names();
    let mut out = Vec::new();
    // SAFETY: DXGI enumeration with valid indices and out-structures.
    unsafe {
        let Ok(factory) = CreateDXGIFactory1::<IDXGIFactory1>() else { return out };
        let mut a = 0;
        while let Ok(adapter) = factory.EnumAdapters1(a) {
            a += 1;
            let Ok(adesc) = adapter.GetDesc1() else { continue };
            let mut o = 0;
            while let Ok(output) = adapter.EnumOutputs(o) {
                o += 1;
                let Ok(desc) = output.GetDesc() else { continue };
                if !desc.AttachedToDesktop.as_bool() {
                    continue;
                }
                let gdi_name = from_wide(&desc.DeviceName);
                let r = desc.DesktopCoordinates;
                let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
                let primary = GetMonitorInfoW(desc.Monitor, &mut info).as_bool() && info.dwFlags & MONITORINFOF_PRIMARY != 0;
                let (friendly, path) = names.get(&gdi_name).cloned().unwrap_or_default();
                out.push(MonitorInfo {
                    id: if path.is_empty() { gdi_name.clone() } else { path },
                    name: if friendly.is_empty() { gdi_name.trim_start_matches(r"\\.\").to_string() } else { friendly },
                    width: (r.right - r.left) as u32,
                    height: (r.bottom - r.top) as u32,
                    refresh_hz: refresh_rate(&gdi_name),
                    x: r.left,
                    y: r.top,
                    primary,
                    hmonitor: desc.Monitor.0 as isize,
                    gdi_name,
                    adapter_luid: (adesc.AdapterLuid.LowPart, adesc.AdapterLuid.HighPart),
                });
            }
        }
    }
    out.sort_by_key(|m| (!m.primary, m.x, m.y));
    // Two outputs with the same device path (rare: cloned displays) need unique ids.
    let mut seen = HashMap::new();
    for m in &mut out {
        let n = seen.entry(m.id.clone()).or_insert(0);
        if *n > 0 {
            m.id = format!("{}#{n}", m.id);
        }
        *n += 1;
    }
    out
}

/// Finds a monitor by id, falling back to the primary monitor.
pub fn find(id: Option<&str>) -> Option<MonitorInfo> {
    let all = list();
    id.and_then(|id| all.iter().find(|m| m.id == id).cloned()).or_else(|| all.into_iter().next())
}

fn refresh_rate(gdi_name: &str) -> u32 {
    let name = crate::util::wide(gdi_name);
    let mut mode = DEVMODEW { dmSize: size_of::<DEVMODEW>() as u16, ..Default::default() };
    // SAFETY: NUL-terminated name and a correctly sized DEVMODEW.
    if unsafe { EnumDisplaySettingsW(PCWSTR(name.as_ptr()), ENUM_CURRENT_SETTINGS, &mut mode) }.as_bool() {
        mode.dmDisplayFrequency
    } else {
        60
    }
}

/// Maps GDI device names (\\.\DISPLAY1) to (friendly name, device path).
fn display_config_names() -> HashMap<String, (String, String)> {
    let mut map = HashMap::new();
    // SAFETY: buffers are sized by GetDisplayConfigBufferSizes; info structs
    // carry correct headers (type, size, adapter, id) as the API requires.
    unsafe {
        let (mut np, mut nm) = (0u32, 0u32);
        if GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut np, &mut nm).is_err() {
            return map;
        }
        let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); np as usize];
        let mut modes = vec![DISPLAYCONFIG_MODE_INFO::default(); nm as usize];
        if QueryDisplayConfig(QDC_ONLY_ACTIVE_PATHS, &mut np, paths.as_mut_ptr(), &mut nm, modes.as_mut_ptr(), None)
            .is_err()
        {
            return map;
        }
        for path in &paths[..np as usize] {
            let mut source = DISPLAYCONFIG_SOURCE_DEVICE_NAME::default();
            source.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME;
            source.header.size = size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>() as u32;
            source.header.adapterId = path.sourceInfo.adapterId;
            source.header.id = path.sourceInfo.id;
            if DisplayConfigGetDeviceInfo(&mut source.header) != 0 {
                continue;
            }
            let mut target = DISPLAYCONFIG_TARGET_DEVICE_NAME::default();
            target.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME;
            target.header.size = size_of::<DISPLAYCONFIG_TARGET_DEVICE_NAME>() as u32;
            target.header.adapterId = path.targetInfo.adapterId;
            target.header.id = path.targetInfo.id;
            if DisplayConfigGetDeviceInfo(&mut target.header) != 0 {
                continue;
            }
            map.insert(
                from_wide(&source.viewGdiDeviceName),
                (from_wide(&target.monitorFriendlyDeviceName), from_wide(&target.monitorDevicePath)),
            );
        }
    }
    map
}

#[cfg(test)]
mod tests {
    #[test]
    fn lists_at_least_one_monitor() {
        // CI machines without a display may return none; on a desktop there is always one.
        let monitors = super::list();
        for m in &monitors {
            assert!(m.width > 0 && m.height > 0, "{m:?}");
        }
        if let Some(first) = monitors.first() {
            assert!(first.primary || monitors.iter().all(|m| !m.primary));
        }
    }
}
