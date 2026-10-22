//! User-facing actions. Hotkeys, the tray menu and UI buttons all go through
//! [`dispatch`], so every trigger behaves the same way.

use lanes_core::Timestamp;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::settings::Settings;
use crate::state::AppState;

pub const SAVE_CLIP_PREFIX: &str = "save_clip:";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    ToggleRecording,
    TogglePause,
    ToggleReplay,
    SaveClip(String),
    Bookmark,
}

impl Action {
    /// The fixed (non-preset) actions, in display order.
    pub const FIXED: [&'static str; 4] = ["toggle_recording", "toggle_pause", "toggle_replay", "bookmark"];

    pub fn parse(id: &str) -> Option<Self> {
        Some(match id {
            "toggle_recording" => Action::ToggleRecording,
            "toggle_pause" => Action::TogglePause,
            "toggle_replay" => Action::ToggleReplay,
            "bookmark" => Action::Bookmark,
            _ => Action::SaveClip(id.strip_prefix(SAVE_CLIP_PREFIX)?.to_string()),
        })
    }

    pub fn id(&self) -> String {
        match self {
            Action::ToggleRecording => "toggle_recording".into(),
            Action::TogglePause => "toggle_pause".into(),
            Action::ToggleReplay => "toggle_replay".into(),
            Action::Bookmark => "bookmark".into(),
            Action::SaveClip(p) => format!("{SAVE_CLIP_PREFIX}{p}"),
        }
    }

    pub fn label(&self, settings: &Settings) -> String {
        match self {
            Action::ToggleRecording => "Start / stop recording".into(),
            Action::TogglePause => "Pause / resume recording".into(),
            Action::ToggleReplay => "Start / stop replay buffer".into(),
            Action::Bookmark => "Bookmark this moment".into(),
            Action::SaveClip(id) => {
                let name = settings.clip_presets.iter().find(|p| &p.id == id).map_or(id.as_str(), |p| p.name.as_str());
                format!("Save clip: {name}")
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Trigger {
    Hotkey,
    Tray,
    Ui,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ActionEvent {
    action: String,
    trigger: Trigger,
    /// When the trigger happened, on the shared QPC clock (ms).
    at_ms: i64,
    detail: Option<String>,
}

/// Runs an action. Never blocks: the engine works on its own thread.
pub fn dispatch(app: &AppHandle, action: &Action, trigger: Trigger, at: Timestamp, detail: Option<String>) {
    let state = app.state::<AppState>();
    match &detail {
        Some(d) => log::info!("{} via {trigger:?} ({d})", action.id()),
        None => log::info!("{} via {trigger:?}", action.id()),
    }
    state.with_engine(|e| match action {
        Action::ToggleRecording => e.toggle_recording(),
        Action::TogglePause => e.toggle_pause(),
        Action::ToggleReplay => e.toggle_replay(),
        Action::SaveClip(preset) => e.save_clip(preset),
        Action::Bookmark => e.bookmark(),
    });
    let _ = app.emit("action", ActionEvent { action: action.id(), trigger, at_ms: at.as_millis(), detail });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_roundtrip() {
        for id in ["toggle_recording", "toggle_pause", "toggle_replay", "bookmark", "save_clip:short"] {
            assert_eq!(Action::parse(id).unwrap().id(), id);
        }
        assert_eq!(Action::parse("nope"), None);
        assert_eq!(Action::parse("save_clip:x"), Some(Action::SaveClip("x".into())));
    }

    #[test]
    fn labels_use_preset_names() {
        let s = Settings::default();
        assert_eq!(Action::SaveClip("short".into()).label(&s), "Save clip: Last 30 seconds");
        assert_eq!(Action::SaveClip("gone".into()).label(&s), "Save clip: gone");
    }
}
