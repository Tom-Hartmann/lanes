//! Top-level window and process enumeration, used for window capture and game
//! detection. Everything here is cheap enough to run every couple of seconds.

use serde::Serialize;
use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, RECT};
use windows::Win32::Graphics::Dwm::{DWMWA_CLOAKED, DwmGetWindowAttribute};
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Threading::{
    OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GWL_EXSTYLE, GWL_STYLE, GetForegroundWindow, GetWindow, GetWindowLongW, GetWindowRect,
    GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId, GW_OWNER, IsIconic, IsWindowVisible, WS_CAPTION,
    WS_EX_TOOLWINDOW,
};
use windows::core::BOOL;

use crate::util::from_wide;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowInfo {
    pub hwnd: isize,
    pub title: String,
    /// Executable file name, e.g. "cs2.exe".
    pub exe: String,
    pub pid: u32,
    /// Covers its whole monitor without a caption (fullscreen or borderless fullscreen).
    pub fullscreen: bool,
    /// Window rectangle in desktop pixels: left, top, right, bottom.
    #[serde(skip)]
    pub rect: (i32, i32, i32, i32),
}

/// Visible, capturable top-level windows with a title.
pub fn list() -> Vec<WindowInfo> {
    let mut hwnds: Vec<HWND> = Vec::new();
    // SAFETY: the callback only pushes into the Vec passed through LPARAM,
    // which outlives the EnumWindows call.
    unsafe {
        let _ = EnumWindows(Some(collect), LPARAM(&mut hwnds as *mut _ as isize));
    }
    let own_pid = std::process::id();
    hwnds
        .into_iter()
        .filter_map(info)
        .filter(|w| w.pid != own_pid && !w.title.is_empty())
        .collect()
}

unsafe extern "system" fn collect(hwnd: HWND, lparam: LPARAM) -> BOOL {
    // SAFETY: lparam is the &mut Vec<HWND> passed by `list`.
    let out = unsafe { &mut *(lparam.0 as *mut Vec<HWND>) };
    if is_capturable(hwnd) {
        out.push(hwnd);
    }
    true.into()
}

fn is_capturable(hwnd: HWND) -> bool {
    // SAFETY: read-only queries on a window handle (stale handles just fail).
    unsafe {
        if !IsWindowVisible(hwnd).as_bool() || IsIconic(hwnd).as_bool() {
            return false;
        }
        if GetWindow(hwnd, GW_OWNER).is_ok_and(|o| !o.is_invalid()) {
            return false;
        }
        if GetWindowLongW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_TOOLWINDOW.0 != 0 {
            return false;
        }
        let mut cloaked = 0u32;
        if DwmGetWindowAttribute(hwnd, DWMWA_CLOAKED, &mut cloaked as *mut _ as *mut _, 4).is_ok() && cloaked != 0 {
            return false;
        }
        GetWindowTextLengthW(hwnd) > 0
    }
}

/// Details of one window, or `None` if it vanished.
pub fn info(hwnd: HWND) -> Option<WindowInfo> {
    let mut pid = 0u32;
    // SAFETY: valid out-pointer; a stale handle yields pid 0.
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    if pid == 0 {
        return None;
    }
    let mut buf = [0u16; 512];
    // SAFETY: buffer length passed matches the buffer.
    let len = unsafe { GetWindowTextW(hwnd, &mut buf) } as usize;
    let mut r = RECT::default();
    // SAFETY: valid out-pointer.
    let _ = unsafe { GetWindowRect(hwnd, &mut r) };
    Some(WindowInfo {
        rect: (r.left, r.top, r.right, r.bottom),
        hwnd: hwnd.0 as isize,
        title: String::from_utf16_lossy(&buf[..len]),
        exe: process_exe(pid).unwrap_or_default(),
        pid,
        fullscreen: is_fullscreen(hwnd),
    })
}

/// The foreground window, if it is a normal application window.
pub fn foreground() -> Option<WindowInfo> {
    // SAFETY: no arguments.
    let hwnd = unsafe { GetForegroundWindow() };
    if hwnd.is_invalid() { None } else { info(hwnd) }
}

/// True if the window covers its whole monitor and has no caption.
pub fn is_fullscreen(hwnd: HWND) -> bool {
    // SAFETY: read-only queries with valid out-pointers.
    unsafe {
        let mut rect = RECT::default();
        if GetWindowRect(hwnd, &mut rect).is_err() {
            return false;
        }
        let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
        let mut mi = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
        if !GetMonitorInfoW(monitor, &mut mi).as_bool() {
            return false;
        }
        let m = mi.rcMonitor;
        let covers = rect.left <= m.left && rect.top <= m.top && rect.right >= m.right && rect.bottom >= m.bottom;
        let captioned = GetWindowLongW(hwnd, GWL_STYLE) as u32 & WS_CAPTION.0 == WS_CAPTION.0;
        covers && !captioned
    }
}

/// Executable file name of a process ("game.exe"), if it can be queried.
pub fn process_exe(pid: u32) -> Option<String> {
    process_path(pid).map(|p| p.rsplit('\\').next().unwrap_or(&p).to_string())
}

/// Full image path of a process.
pub fn process_path(pid: u32) -> Option<String> {
    // SAFETY: the handle is closed below; buffer size is passed in/out.
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, windows::core::PWSTR(buf.as_mut_ptr()), &mut len).is_ok();
        let _ = CloseHandle(h);
        ok.then(|| String::from_utf16_lossy(&buf[..len as usize]))
    }
}

#[derive(Clone, Debug)]
pub struct ProcessEntry {
    pub pid: u32,
    pub parent_pid: u32,
    /// Executable file name as reported by the snapshot.
    pub exe: String,
}

/// Snapshot of all running processes.
pub fn processes() -> Vec<ProcessEntry> {
    let mut out = Vec::new();
    // SAFETY: snapshot handle closed below; entry has dwSize set as required.
    unsafe {
        let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else { return out };
        let mut entry = PROCESSENTRY32W { dwSize: size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        if Process32FirstW(snap, &mut entry).is_ok() {
            loop {
                out.push(ProcessEntry {
                    pid: entry.th32ProcessID,
                    parent_pid: entry.th32ParentProcessID,
                    exe: from_wide(&entry.szExeFile),
                });
                if Process32NextW(snap, &mut entry).is_err() {
                    break;
                }
            }
        }
        let _ = CloseHandle(snap);
    }
    out
}

/// Root processes of an application: processes named `exe` whose parent is
/// not also `exe` (Discord, browsers and launchers spawn many children).
pub fn root_pids(procs: &[ProcessEntry], exe: &str) -> Vec<u32> {
    let named: Vec<&ProcessEntry> = procs.iter().filter(|p| p.exe.eq_ignore_ascii_case(exe)).collect();
    named
        .iter()
        .filter(|p| !named.iter().any(|q| q.pid == p.parent_pid))
        .map(|p| p.pid)
        .collect()
}

/// The largest visible window of a process tree's root, for window capture by exe name.
pub fn main_window_of(exe: &str) -> Option<WindowInfo> {
    list()
        .into_iter()
        .filter(|w| w.exe.eq_ignore_ascii_case(exe))
        .max_by_key(|w| i64::from(w.rect.2 - w.rect.0) * i64::from(w.rect.3 - w.rect.1))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(pid: u32, parent: u32, exe: &str) -> ProcessEntry {
        ProcessEntry { pid, parent_pid: parent, exe: exe.into() }
    }

    #[test]
    fn root_pids_skips_children_of_same_exe() {
        let procs = vec![
            p(1, 0, "explorer.exe"),
            p(10, 1, "Discord.exe"),
            p(11, 10, "Discord.exe"),
            p(12, 10, "Discord.exe"),
            p(20, 1, "discord.exe"), // a second, independent instance
        ];
        let mut roots = root_pids(&procs, "DISCORD.EXE");
        roots.sort();
        assert_eq!(roots, vec![10, 20]);
        assert!(root_pids(&procs, "nope.exe").is_empty());
    }

    #[test]
    fn snapshot_contains_this_process() {
        let me = std::process::id();
        assert!(processes().iter().any(|p| p.pid == me));
        assert!(process_exe(me).is_some_and(|e| e.ends_with(".exe")));
    }
}
