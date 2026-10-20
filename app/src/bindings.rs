//! The app's hotkey table, kept in sync with `settings.hotkeys`.

use std::sync::Mutex;

use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::actions::{self, Action, Trigger};
use crate::hotkeys::{Backend, Combo, HotkeyService, Mode};
use crate::settings::{HotkeySetting, Settings};

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BindingInfo {
    pub action: String,
    pub label: String,
    pub combo: Option<Combo>,
    pub backend: Backend,
    /// What the binding actually uses; `None` if unbound or failed.
    pub mode: Option<Mode>,
    pub error: Option<String>,
}

/// Owns the hotkey thread.
pub struct Hotkeys {
    service: HotkeyService,
    table: Mutex<Vec<BindingInfo>>,
    app: AppHandle,
}

impl Hotkeys {
    pub fn start(app: AppHandle) -> std::io::Result<Self> {
        let handle = app.clone();
        let service = HotkeyService::start(move |event| {
            if let Some(action) = Action::parse(&event.id) {
                let detail = format!("{}, {:?}", event.combo, event.mode).to_lowercase();
                actions::dispatch(&handle, &action, Trigger::Hotkey, event.at, Some(detail));
            }
        })?;
        Ok(Self { service, table: Mutex::new(Vec::new()), app })
    }

    pub fn list(&self) -> Vec<BindingInfo> {
        self.table.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Makes the registered hotkeys match `settings.hotkeys` (adds, rebinds,
    /// removes) and refreshes labels. Emits `hotkeys`.
    pub fn sync(&self, settings: &Settings) {
        let current = self.list();
        let mut next = Vec::new();
        for h in &settings.hotkeys {
            let Some(action) = Action::parse(&h.action) else { continue };
            let existing = current.iter().find(|b| b.action == h.action);
            let unchanged = existing.is_some_and(|b| b.combo == h.combo && b.backend == h.backend && b.error.is_none());
            let mut info = if unchanged {
                existing.cloned().expect("checked")
            } else {
                self.bind(&h.action, h.combo, h.backend)
            };
            info.label = action.label(settings);
            next.push(info);
        }
        for b in &current {
            if !settings.hotkeys.iter().any(|h| h.action == b.action) {
                self.service.unbind(&b.action);
            }
        }
        *self.table.lock().unwrap_or_else(|e| e.into_inner()) = next.clone();
        let _ = self.app.emit("hotkeys", next);
    }

    /// Rebinds one action. A failed bind keeps the previous combo active and
    /// returns the error; on success the caller persists the new setting.
    pub fn set(&self, settings: &Settings, action: &str, combo: Option<Combo>, backend: Backend) -> Result<HotkeySetting, String> {
        let parsed = Action::parse(action).ok_or_else(|| format!("unknown action \"{action}\""))?;
        let previous = self.list().into_iter().find(|b| b.action == action);
        let mut info = self.bind(action, combo, backend);
        info.label = parsed.label(settings);
        if let Some(error) = info.error.clone() {
            if let Some(prev) = previous.filter(|p| p.mode.is_some()) {
                self.bind(action, prev.combo, prev.backend);
            }
            return Err(error);
        }
        let mut table = self.table.lock().unwrap_or_else(|e| e.into_inner());
        match table.iter_mut().find(|b| b.action == action) {
            Some(slot) => *slot = info,
            None => table.push(info),
        }
        Ok(HotkeySetting { action: action.to_string(), combo, backend })
    }

    fn bind(&self, action: &str, combo: Option<Combo>, backend: Backend) -> BindingInfo {
        let (mode, error) = match combo {
            Some(c) => match self.service.bind(action, c, backend) {
                Ok(mode) => {
                    log::info!("{c} -> {action} ({mode:?})");
                    (Some(mode), None)
                }
                Err(e) => {
                    log::warn!("could not bind {c} to {action}: {e}");
                    (None, Some(e.to_string()))
                }
            },
            None => {
                self.service.unbind(action);
                (None, None)
            }
        };
        BindingInfo { action: action.to_string(), label: String::new(), combo, backend, mode, error }
    }
}

/// Ensures every clip preset has a hotkey entry (unbound by default) and
/// drops entries for deleted presets.
pub fn reconcile_settings(settings: &mut Settings) {
    let preset_ids: Vec<String> = settings.clip_presets.iter().map(|p| p.id.clone()).collect();
    settings.hotkeys.retain(|h| match Action::parse(&h.action) {
        Some(Action::SaveClip(id)) => preset_ids.contains(&id),
        Some(_) => true,
        None => false,
    });
    for fixed in Action::FIXED {
        if !settings.hotkeys.iter().any(|h| h.action == fixed) {
            settings.hotkeys.push(HotkeySetting { action: fixed.into(), combo: None, backend: Backend::Auto });
        }
    }
    for id in preset_ids {
        let action = Action::SaveClip(id).id();
        if !settings.hotkeys.iter().any(|h| h.action == action) {
            settings.hotkeys.push(HotkeySetting { action, combo: None, backend: Backend::Auto });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lanes_core::config::{ClipPreset, ClipQuality};

    #[test]
    fn reconcile_adds_preset_slots_and_drops_stale_ones() {
        let mut s = Settings::default();
        s.clip_presets.push(ClipPreset {
            id: "new".into(),
            name: "New".into(),
            length_secs: 10,
            lanes: None,
            quality: ClipQuality::Original,
            folder: None,
            filename_template: None,
        });
        s.clip_presets.retain(|p| p.id != "long");
        s.hotkeys.push(HotkeySetting { action: "bogus".into(), combo: None, backend: Backend::Auto });
        reconcile_settings(&mut s);
        let actions: Vec<_> = s.hotkeys.iter().map(|h| h.action.as_str()).collect();
        assert!(actions.contains(&"save_clip:new"));
        assert!(!actions.contains(&"save_clip:long"));
        assert!(!actions.contains(&"bogus"));
        assert!(actions.contains(&"toggle_replay"));
    }
}
