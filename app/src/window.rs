//! The UI window is created on demand and destroyed on close, which shuts down
//! its WebView2 processes. The backend keeps running in the tray, so the UI
//! costs nothing while the user is gaming. While open, the WebView2
//! processes run at below-normal priority so they never compete with a game.

use std::time::Duration;

use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::System::Threading::{BELOW_NORMAL_PRIORITY_CLASS, OpenProcess, PROCESS_SET_INFORMATION, SetPriorityClass};

pub const MAIN: &str = "main";

/// Focuses the UI window, creating it if it is closed.
pub fn show_main(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(MAIN) {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
        return;
    }
    let built = WebviewWindowBuilder::new(app, MAIN, WebviewUrl::App("index.html".into()))
        .title("Lanes")
        .inner_size(1180.0, 760.0)
        .min_inner_size(820.0, 520.0)
        // Tauri's native file-drop handler swallows HTML5 drag & drop, which
        // the Audio Lanes page uses.
        .disable_drag_drop_handler()
        .build();
    match built {
        Ok(_) => {
            log::debug!("UI window opened");
            lower_webview_priority();
        }
        Err(e) => log::error!("could not open the UI window: {e}"),
    }
}

/// Sets every WebView2 process started by us to below-normal priority.
fn lower_webview_priority() {
    let _ = std::thread::Builder::new().name("lanes-ui-priority".into()).spawn(|| {
        // WebView2 spawns its processes asynchronously; catch late ones too.
        for delay in [1, 3, 8] {
            std::thread::sleep(Duration::from_secs(delay));
            let procs = lanes_core::capture::windows_list::processes();
            let me = std::process::id();
            let descends_from_me = |mut pid: u32| {
                for _ in 0..8 {
                    match procs.iter().find(|p| p.pid == pid) {
                        Some(p) if p.parent_pid == me => return true,
                        Some(p) => pid = p.parent_pid,
                        None => return false,
                    }
                }
                false
            };
            for p in procs.iter().filter(|p| p.exe.eq_ignore_ascii_case("msedgewebview2.exe") && descends_from_me(p.pid)) {
                // SAFETY: handle opened with only the right we need and closed right after.
                unsafe {
                    if let Ok(h) = OpenProcess(PROCESS_SET_INFORMATION, false, p.pid) {
                        let _ = SetPriorityClass(h, BELOW_NORMAL_PRIORITY_CLASS);
                        let _ = CloseHandle(h);
                    }
                }
            }
        }
    });
}
