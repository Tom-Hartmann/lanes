//! Clip-saved feedback: a chime on a chosen output device, a small on-screen
//! notice in a chosen corner (click-through, never takes focus) and an
//! optional Windows notification. One save of several monitor files gives one
//! round of feedback.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use lanes_core::audio::chime;
use tauri::AppHandle;
use tauri_plugin_notification::NotificationExt;

use crate::overlay;
use crate::settings::Settings;

/// Values for the notice text placeholders.
pub struct ClipInfo<'a> {
    pub preset: &'a str,
    pub secs: f64,
    pub game: Option<&'a str>,
    /// A window on the monitor the notice should appear on (the game).
    pub near: Option<isize>,
}

/// `{preset}`, `{duration}` and `{game}` substituted.
pub fn render_text(template: &str, clip: &ClipInfo<'_>) -> String {
    let text = template
        .replace("{preset}", clip.preset)
        .replace("{duration}", &format!("{:.0}", clip.secs))
        .replace("{game}", clip.game.unwrap_or("Desktop"));
    let text = text.trim();
    if text.is_empty() { "Clip saved".into() } else { text.chars().take(120).collect() }
}

pub fn clip_saved(app: &AppHandle, settings: &Settings, clip: &ClipInfo<'_>) {
    // A multi-monitor save reports one file per monitor within milliseconds.
    static LAST: Mutex<Option<Instant>> = Mutex::new(None);
    {
        let mut last = LAST.lock().unwrap_or_else(|e| e.into_inner());
        if last.is_some_and(|t| t.elapsed() < Duration::from_millis(1500)) {
            return;
        }
        *last = Some(Instant::now());
    }
    let g = &settings.general;
    if g.clip_sound {
        sound(settings);
    }
    if g.clip_overlay {
        notice(settings, clip);
    }
    if g.clip_toast {
        toast(app, clip);
    }
}

pub fn sound(settings: &Settings) {
    chime::play(settings.general.clip_sound_device.clone(), settings.general.clip_sound_volume);
}

pub fn notice(settings: &Settings, clip: &ClipInfo<'_>) {
    overlay::show(render_text(&settings.general.clip_overlay_text, clip), settings.general.clip_overlay_corner, clip.near);
}

pub fn toast(app: &AppHandle, clip: &ClipInfo<'_>) {
    let body = format!("{} · {:.0} s", clip.preset, clip.secs);
    if let Err(e) = app.notification().builder().title("Clip saved").body(body).show() {
        log::debug!("toast failed: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholders_are_filled() {
        let clip = ClipInfo { preset: "Last 30 seconds", secs: 30.4, game: Some("Blade & Soul"), near: None };
        assert_eq!(render_text("{game}: {preset} ({duration}s)", &clip), "Blade & Soul: Last 30 seconds (30s)");
        let no_game = ClipInfo { game: None, ..clip };
        assert_eq!(render_text("{game}", &no_game), "Desktop");
        assert_eq!(render_text("   ", &no_game), "Clip saved");
    }
}
