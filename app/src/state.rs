//! Shared application state and the settings → engine/hotkeys/autostart plumbing.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use lanes_core::audio::mixer::MeterFrame;
use lanes_core::engine::{ClipSaved, DiskGuard, Engine, EngineConfig, EngineError, EngineStatus, EventSink, OutputConfig, Stats};
use lanes_core::gpu::Gpu;
use lanes_platform::admin;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::bindings::{self, Hotkeys};
use crate::library::{Kind, Library};
use crate::settings::{self, DiskGuardAction, Settings, Store};
use crate::window;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub version: &'static str,
    pub elevated: bool,
    pub admin_task_installed: bool,
    pub os_build: u32,
    pub per_app_audio_supported: bool,
    pub installed_per_machine: bool,
}

impl AppInfo {
    pub fn current(elevated: bool) -> Self {
        Self {
            version: env!("CARGO_PKG_VERSION"),
            elevated,
            admin_task_installed: admin::task_exists(),
            os_build: lanes_platform::os::build_number(),
            per_app_audio_supported: lanes_platform::os::supports_process_loopback(),
            installed_per_machine: admin::installed_per_machine(),
        }
    }
}

pub struct AppState {
    pub settings: Mutex<Settings>,
    pub store: Store,
    pub engine: Mutex<Option<Engine>>,
    pub hotkeys: Hotkeys,
    pub library: Library,
    pub elevated: bool,
    /// Profile to restore when an auto-switched game exits.
    pub profile_before_game: Mutex<Option<String>>,
    /// Lazily created GPU for UI helpers (thumbnails, encoder list).
    tools_gpu: OnceLock<Option<Gpu>>,
}

impl AppState {
    pub fn new(settings: Settings, store: Store, engine: Engine, hotkeys: Hotkeys, elevated: bool) -> Self {
        Self {
            settings: Mutex::new(settings),
            store,
            engine: Mutex::new(Some(engine)),
            hotkeys,
            library: Library::load(&settings::config_dir()),
            elevated,
            profile_before_game: Mutex::new(None),
            tools_gpu: OnceLock::new(),
        }
    }

    pub fn settings(&self) -> Settings {
        self.settings.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn with_engine<R>(&self, f: impl FnOnce(&Engine) -> R) -> Option<R> {
        self.engine.lock().unwrap_or_else(|e| e.into_inner()).as_ref().map(f)
    }

    pub fn status(&self) -> Option<EngineStatus> {
        self.with_engine(Engine::status)
    }

    pub fn gpu(&self) -> Option<&Gpu> {
        self.tools_gpu.get_or_init(|| Gpu::new(None).map_err(|e| log::error!("no GPU for UI tools: {e}")).ok()).as_ref()
    }

    /// Folders the clips library shows (and UI file commands may touch).
    pub fn library_folders(&self) -> Vec<PathBuf> {
        let s = self.settings();
        let mut v = vec![PathBuf::from(&s.output.folder)];
        for p in &s.clip_presets {
            if let Some(f) = &p.folder
                && !v.iter().any(|x| x == Path::new(f)) {
                    v.push(PathBuf::from(f));
                }
        }
        v.push(playback_cache_dir());
        v
    }

    pub fn info(&self) -> AppInfo {
        AppInfo::current(self.elevated)
    }
}

pub fn local_data_dir() -> PathBuf {
    std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir).join("Lanes")
}

pub fn playback_cache_dir() -> PathBuf {
    local_data_dir().join("playback")
}

/// Builds the engine configuration from settings and the active profile.
pub fn engine_config(s: &Settings) -> EngineConfig {
    let profile = s.active();
    let mut replay = s.replay.clone();
    if let Some(len) = profile.replay_length_secs {
        replay.length_secs = len.clamp(5, 3600);
    }
    EngineConfig {
        profile_id: profile.id.clone(),
        profile_name: profile.name.clone(),
        encoder: s.encoder.clone(),
        video: profile.video.clone(),
        audio: profile.audio.clone(),
        replay,
        presets: s.clip_presets.clone(),
        output: OutputConfig {
            folder: PathBuf::from(&s.output.folder),
            filename_template: s.output.filename_template.clone(),
            remux_to_mp4: s.output.remux_to_mp4,
            disk_guard: s.output.disk_guard_enabled.then(|| DiskGuard {
                min_free_bytes: (f64::from(s.output.disk_guard_min_free_gb.max(0.0)) * 1e9) as u64,
                stop: s.output.disk_guard_action == DiskGuardAction::Stop,
            }),
        },
        temp_dir: local_data_dir(),
    }
}

/// Validates, persists and applies new settings everywhere. Returns what was stored.
pub fn apply_settings(app: &AppHandle, mut new: Settings) -> Result<Settings, String> {
    let state = app.state::<AppState>();
    for fix in new.validate() {
        log::warn!("settings: {fix}");
    }
    bindings::reconcile_settings(&mut new);
    let old = state.settings();
    state.store.save(&new).map_err(|e| format!("could not save settings: {e}"))?;
    *state.settings.lock().unwrap_or_else(|e| e.into_inner()) = new.clone();

    state.with_engine(|e| e.configure(engine_config(&new)));
    state.hotkeys.sync(&new);
    allow_asset_folders(app, &state.library_folders());
    if old.general != new.general {
        reconcile_autostart(&new, state.elevated);
    }
    if old.clip_presets != new.clip_presets {
        crate::tray::refresh(app);
    }
    let _ = app.emit("settings", &new);
    Ok(new)
}

pub fn allow_asset_folders(app: &AppHandle, folders: &[PathBuf]) {
    let scope = app.asset_protocol_scope();
    for f in folders {
        let _ = std::fs::create_dir_all(f);
        if let Err(e) = scope.allow_directory(f, false) {
            log::warn!("asset scope for {}: {e}", f.display());
        }
    }
}

/// Makes Windows autostart match the settings: the per-user Run key normally,
/// or the elevated task's logon trigger in admin mode.
pub fn reconcile_autostart(s: &Settings, elevated: bool) {
    let g = &s.general;
    if g.run_as_admin && admin::task_exists() {
        // The task handles logon start; the Run key would start a second, unelevated copy.
        let _ = lanes_platform::autostart::set_enabled(false);
        if elevated {
            let opts = admin::TaskOptions { logon_trigger: g.start_with_windows, delay_secs: g.autostart_delay_secs };
            if let Ok(exe) = std::env::current_exe()
                && let Err(e) = admin::install_task(&exe, &opts) {
                    log::error!("updating the elevated task failed: {e}");
                }
        } else {
            log::info!("autostart changes for admin mode apply the next time Lanes runs as administrator");
        }
        return;
    }
    if let Err(e) = lanes_platform::autostart::set_enabled(g.start_with_windows) {
        log::error!("could not update autostart: {e}");
    }
}

/// Forwards engine events to the UI. Meters and stats are only sent while the
/// UI window exists, so a closed UI costs nothing.
pub struct TauriSink {
    pub app: AppHandle,
}

impl TauriSink {
    fn ui_open(&self) -> bool {
        self.app.get_webview_window(window::MAIN).is_some()
    }
}

impl EventSink for TauriSink {
    fn status(&self, status: &EngineStatus) {
        let _ = self.app.emit("status", status);
        crate::tray::update_status(&self.app, status);
    }

    fn meters(&self, meters: &MeterFrame) {
        if self.ui_open() {
            let _ = self.app.emit("meters", meters);
        }
    }

    fn stats(&self, stats: &Stats) {
        if self.ui_open() {
            let _ = self.app.emit("stats", stats);
        }
    }

    fn clip_saved(&self, clip: &ClipSaved) {
        let state = self.app.state::<AppState>();
        state.library.add(Path::new(&clip.path), Kind::Clip);
        let s = state.settings();
        log::info!("clip saved: {} ({:.1} s)", clip.path, clip.duration_secs);
        let _ = self.app.emit("clip_saved", clip);
        let preset = s.clip_presets.iter().find(|p| p.id == clip.preset).map_or(clip.preset.as_str(), |p| p.name.as_str()).to_string();
        let game = state.status().and_then(|st| st.game);
        crate::feedback::clip_saved(
            &self.app,
            &s,
            &crate::feedback::ClipInfo {
                preset: &preset,
                secs: clip.duration_secs,
                game: game.as_ref().map(|g| g.name.as_str()),
                near: game.as_ref().and_then(|g| g.hwnd),
            },
        );
    }

    fn recording_saved(&self, paths: &[PathBuf]) {
        let state = self.app.state::<AppState>();
        for p in paths {
            state.library.add(p, Kind::Recording);
            log::info!("recording saved: {}", p.display());
        }
        let _ = self.app.emit("recording_saved", paths);
    }

    fn error(&self, error: &EngineError) {
        let _ = self.app.emit("engine_error", error);
    }
}

pub fn make_sink(app: &AppHandle) -> Arc<dyn EventSink> {
    Arc::new(TauriSink { app: app.clone() })
}
