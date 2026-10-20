//! Clip-saved feedback: a short system sound and a Windows toast. Toasts are
//! shown by the notification platform and never take focus from the game.

use tauri::AppHandle;
use tauri_plugin_notification::NotificationExt;
use windows::Win32::Media::Audio::{PlaySoundW, SND_ALIAS, SND_ASYNC, SND_NODEFAULT};
use windows::core::w;

use crate::settings::Settings;

pub fn clip_saved(app: &AppHandle, settings: &Settings, preset: &str, secs: f64) {
    if settings.general.clip_sound {
        // SAFETY: static alias string; asynchronous playback returns immediately.
        unsafe {
            let _ = PlaySoundW(w!("SystemNotification"), None, SND_ALIAS | SND_ASYNC | SND_NODEFAULT);
        }
    }
    if settings.general.clip_toast {
        let result = app.notification().builder().title("Clip saved").body(format!("{preset} · {secs:.0} s")).show();
        if let Err(e) = result {
            log::debug!("toast failed: {e}");
        }
    }
}
