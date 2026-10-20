//! Tauri commands invoked from the UI (see `ui/src/lib/ipc.ts` for the contract).
//!
//! Anything that may take more than a few milliseconds (thumbnails, file
//! operations, UAC) is `async` and runs on a blocking worker, never on the
//! main thread.

use std::path::{Path, PathBuf};

use lanes_core::Timestamp;
use lanes_core::audio::devices::{self, AudioApp, AudioDevices};
use lanes_core::capture::monitors::{self, MonitorInfo};
use lanes_core::capture::windows_list::{self, WindowInfo};
use lanes_core::capture::{Target, thumbnail};
use lanes_core::encode::EncoderInfo;
use lanes_core::engine::{EngineStatus, naming};
use lanes_platform::admin;
use tauri::{AppHandle, Manager, State};

use crate::actions::{self, Action, Trigger};
use crate::bindings::BindingInfo;
use crate::hotkeys::Backend;
use crate::library::{self, ClipInfo};
use crate::logbus::{self, LogEntry};
use crate::settings::Settings;
use crate::state::{self, AppInfo, AppState};

type CmdResult<T> = Result<T, String>;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> CmdResult<T> + Send + 'static) -> CmdResult<T> {
    tauri::async_runtime::spawn_blocking(f).await.map_err(err)?
}

// ------------------------------------------------------------------ app ---

#[tauri::command]
pub fn get_app_info(state: State<'_, AppState>) -> AppInfo {
    state.info()
}

/// Uncaught UI errors are forwarded here so they end up in the same log.
#[tauri::command]
pub fn ui_log(level: String, message: String) {
    let message: String = message.chars().take(2000).collect();
    match level.as_str() {
        "error" => log::error!(target: "lanes_ui", "{message}"),
        "warn" => log::warn!(target: "lanes_ui", "{message}"),
        _ => log::info!(target: "lanes_ui", "{message}"),
    }
}

#[tauri::command]
pub fn get_logs() -> Vec<LogEntry> {
    logbus::history()
}

#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> Settings {
    state.settings()
}

#[tauri::command]
pub fn update_settings(app: AppHandle, settings: Settings) -> CmdResult<Settings> {
    state::apply_settings(&app, settings)
}

#[tauri::command]
pub fn activate_profile(app: AppHandle, state: State<'_, AppState>, id: String) -> CmdResult<()> {
    let mut s = state.settings();
    if s.profile(&id).is_none() {
        return Err(format!("unknown profile \"{id}\""));
    }
    s.active_profile = id;
    // A manual choice overrides game auto-switching until the next game.
    *state.profile_before_game.lock().unwrap_or_else(|e| e.into_inner()) = None;
    state::apply_settings(&app, s).map(|_| ())
}

// -------------------------------------------------------------- hotkeys ---

#[tauri::command]
pub fn list_hotkeys(state: State<'_, AppState>) -> Vec<BindingInfo> {
    state.hotkeys.list()
}

#[tauri::command]
pub fn set_hotkey(app: AppHandle, state: State<'_, AppState>, action: String, combo: Option<String>, backend: Backend) -> CmdResult<BindingInfo> {
    let combo = combo.map(|c| c.parse()).transpose().map_err(err)?;
    let mut s = state.settings();
    let setting = state.hotkeys.set(&s, &action, combo, backend)?;
    match s.hotkeys.iter_mut().find(|h| h.action == action) {
        Some(h) => *h = setting,
        None => s.hotkeys.push(setting),
    }
    state::apply_settings(&app, s)?;
    state.hotkeys.list().into_iter().find(|b| b.action == action).ok_or_else(|| "binding vanished".into())
}

#[tauri::command]
pub fn trigger_action(app: AppHandle, action: String) -> CmdResult<()> {
    let a = Action::parse(&action).ok_or_else(|| format!("unknown action \"{action}\""))?;
    actions::dispatch(&app, &a, Trigger::Ui, Timestamp::now(), None);
    Ok(())
}

// --------------------------------------------------------------- engine ---

#[tauri::command]
pub fn get_status(state: State<'_, AppState>) -> CmdResult<EngineStatus> {
    state.status().ok_or_else(|| "engine stopped".into())
}

#[tauri::command]
pub fn start_recording(state: State<'_, AppState>) {
    state.with_engine(|e| e.start_recording());
}

#[tauri::command]
pub fn stop_recording(state: State<'_, AppState>) {
    state.with_engine(|e| e.stop_recording());
}

#[tauri::command]
pub fn toggle_pause(state: State<'_, AppState>) {
    state.with_engine(|e| e.toggle_pause());
}

#[tauri::command]
pub fn start_replay(state: State<'_, AppState>) {
    state.with_engine(|e| e.start_replay());
}

#[tauri::command]
pub fn stop_replay(state: State<'_, AppState>) {
    state.with_engine(|e| e.stop_replay());
}

#[tauri::command]
pub fn save_clip(app: AppHandle, preset_id: String) {
    actions::dispatch(&app, &Action::SaveClip(preset_id), Trigger::Ui, Timestamp::now(), None);
}

// -------------------------------------------------------------- sources ---

#[tauri::command]
pub async fn list_monitors() -> CmdResult<Vec<MonitorInfo>> {
    blocking(|| Ok(monitors::list())).await
}

#[tauri::command]
pub async fn get_monitor_thumbnail(app: AppHandle, id: String) -> CmdResult<String> {
    blocking(move || {
        let state = app.state::<AppState>();
        let gpu = state.gpu().ok_or("no GPU available")?;
        let m = monitors::list().into_iter().find(|m| m.id == id).ok_or("monitor not found")?;
        thumbnail::grab(gpu, &Target::Monitor { hmonitor: m.hmonitor, name: m.name }, 320).map_err(err)
    })
    .await
}

#[tauri::command]
pub async fn list_windows() -> CmdResult<Vec<WindowInfo>> {
    blocking(|| Ok(windows_list::list())).await
}

#[tauri::command]
pub async fn list_audio_devices() -> CmdResult<AudioDevices> {
    blocking(|| Ok(devices::list())).await
}

#[tauri::command]
pub async fn list_audio_apps() -> CmdResult<Vec<AudioApp>> {
    blocking(|| Ok(devices::apps())).await
}

#[tauri::command]
pub async fn list_encoders(app: AppHandle) -> CmdResult<Vec<EncoderInfo>> {
    blocking(move || {
        let state = app.state::<AppState>();
        let gpu = state.gpu().ok_or("no GPU available")?;
        Ok(lanes_core::encode::available(gpu).to_vec())
    })
    .await
}

// ---------------------------------------------------------------- clips ---

fn checked_path(state: &AppState, path: &str) -> CmdResult<PathBuf> {
    let p = PathBuf::from(path);
    if library::is_within(&p, &state.library_folders()) {
        Ok(p)
    } else {
        Err("that file is not in a Lanes output folder".into())
    }
}

#[tauri::command]
pub async fn list_clips(app: AppHandle) -> CmdResult<Vec<ClipInfo>> {
    blocking(move || {
        let state = app.state::<AppState>();
        let mut folders = state.library_folders();
        folders.retain(|f| f != &state::playback_cache_dir());
        Ok(state.library.list(&folders))
    })
    .await
}

#[tauri::command]
pub async fn get_clip_thumbnail(app: AppHandle, path: String) -> CmdResult<String> {
    blocking(move || {
        let p = checked_path(&app.state::<AppState>(), &path)?;
        lanes_core::media::thumbnail(&p, 320).map_err(err)
    })
    .await
}

/// MP4 plays directly in WebView2; MKV is remuxed once into a cache folder.
#[tauri::command]
pub async fn prepare_clip_playback(app: AppHandle, path: String) -> CmdResult<String> {
    blocking(move || {
        let p = checked_path(&app.state::<AppState>(), &path)?;
        if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("mp4")) {
            return Ok(path);
        }
        let cache = state::playback_cache_dir();
        std::fs::create_dir_all(&cache).map_err(err)?;
        let meta = std::fs::metadata(&p).map_err(err)?;
        let stamp = meta.modified().ok().and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs());
        let name = format!("{}_{stamp}_{}.mp4", naming::sanitize(&p.file_stem().unwrap_or_default().to_string_lossy()), meta.len());
        let out = cache.join(name);
        if !out.exists() {
            lanes_core::media::remux(&p, &out).map_err(err)?;
        }
        Ok(out.to_string_lossy().into_owned())
    })
    .await
}

#[tauri::command]
pub async fn delete_clip(app: AppHandle, path: String) -> CmdResult<()> {
    blocking(move || {
        let state = app.state::<AppState>();
        let p = checked_path(&state, &path)?;
        recycle(&p)?;
        state.library.remove(&p);
        Ok(())
    })
    .await
}

#[tauri::command]
pub fn reveal_clip(state: State<'_, AppState>, path: String) -> CmdResult<()> {
    let p = checked_path(&state, &path)?;
    open_explorer(&format!("/select,\"{}\"", p.display()))
}

#[tauri::command]
pub fn open_output_folder(state: State<'_, AppState>) -> CmdResult<()> {
    let folder = PathBuf::from(state.settings().output.folder);
    std::fs::create_dir_all(&folder).map_err(err)?;
    open_explorer(&format!("\"{}\"", folder.display()))
}

#[tauri::command]
pub async fn remux_clip(app: AppHandle, path: String) -> CmdResult<String> {
    blocking(move || {
        let state = app.state::<AppState>();
        let p = checked_path(&state, &path)?;
        let dir = p.parent().unwrap_or(Path::new("."));
        let out = naming::unique_path(dir, &p.file_stem().unwrap_or_default().to_string_lossy(), "mp4");
        lanes_core::media::remux(&p, &out).map_err(err)?;
        state.library.add(&out, kind_of(&state, &p));
        Ok(out.to_string_lossy().into_owned())
    })
    .await
}

#[tauri::command]
pub async fn trim_clip(app: AppHandle, path: String, start_secs: f64, end_secs: f64) -> CmdResult<String> {
    blocking(move || {
        let state = app.state::<AppState>();
        let p = checked_path(&state, &path)?;
        let dir = p.parent().unwrap_or(Path::new("."));
        let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("mp4").to_string();
        let out = naming::unique_path(dir, &format!("{}_trim", p.file_stem().unwrap_or_default().to_string_lossy()), &ext);
        lanes_core::media::trim(&p, &out, start_secs, end_secs).map_err(err)?;
        state.library.add(&out, library::Kind::Clip);
        Ok(out.to_string_lossy().into_owned())
    })
    .await
}

fn kind_of(state: &AppState, path: &Path) -> library::Kind {
    state
        .library
        .list(&[path.parent().unwrap_or(Path::new(".")).to_path_buf()])
        .into_iter()
        .find(|c| Path::new(&c.path) == path)
        .map_or(library::Kind::Recording, |c| c.kind)
}

fn open_explorer(args: &str) -> CmdResult<()> {
    use std::os::windows::process::CommandExt;
    std::process::Command::new("explorer.exe").raw_arg(args).spawn().map(|_| ()).map_err(err)
}

/// Moves a file to the recycle bin (undoable), without UI.
fn recycle(path: &Path) -> CmdResult<()> {
    use windows::Win32::UI::Shell::{FO_DELETE, FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_NOERRORUI, FOF_SILENT, SHFILEOPSTRUCTW, SHFileOperationW};
    // pFrom is a double-NUL-terminated list.
    let mut from: Vec<u16> = path.as_os_str().encode_wide_vec();
    from.extend([0, 0]);
    let mut op = SHFILEOPSTRUCTW {
        wFunc: FO_DELETE,
        pFrom: windows::core::PCWSTR(from.as_ptr()),
        fFlags: (FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_SILENT | FOF_NOERRORUI).0 as u16,
        ..Default::default()
    };
    // SAFETY: `from` is double-NUL terminated and outlives the call.
    let rc = unsafe { SHFileOperationW(&mut op) };
    if rc == 0 && !op.fAnyOperationsAborted.as_bool() { Ok(()) } else { Err(format!("could not delete the file (code {rc})")) }
}

trait EncodeWideVec {
    fn encode_wide_vec(&self) -> Vec<u16>;
}

impl EncodeWideVec for std::ffi::OsStr {
    fn encode_wide_vec(&self) -> Vec<u16> {
        use std::os::windows::ffi::OsStrExt;
        self.encode_wide().collect()
    }
}

// ---------------------------------------------------------------- admin ---

/// Enables or disables admin mode. Enabling shows one UAC prompt (the
/// elevated helper registers the scheduled task); disabling removes it.
#[tauri::command]
pub async fn set_run_as_admin(app: AppHandle, enabled: bool) -> CmdResult<AppInfo> {
    blocking(move || {
        let state = app.state::<AppState>();
        let mut s = state.settings();
        if enabled {
            let opts = admin::TaskOptions { logon_trigger: s.general.start_with_windows, delay_secs: s.general.autostart_delay_secs };
            let result = if state.elevated {
                std::env::current_exe().map_err(|e| e.to_string()).and_then(|exe| admin::install_task(&exe, &opts).map_err(err))
            } else {
                admin::install_via_uac(&opts).map_err(err)
            };
            result.map_err(|e| format!("could not set up admin mode: {e}"))?;
        } else {
            let result = if state.elevated { admin::remove_task() } else { admin::remove_via_uac() };
            result.map_err(|e| format!("could not remove the admin task: {e}"))?;
        }
        s.general.run_as_admin = enabled;
        state::apply_settings(&app, s)?;
        // The Run key and task must not both start Lanes.
        state::reconcile_autostart(&state.settings(), state.elevated);
        crate::tray::refresh(&app);
        Ok(state.info())
    })
    .await
}

#[tauri::command]
pub fn relaunch_as_admin(app: AppHandle) {
    relaunch_elevated(&app);
}

/// Starts the elevated task (no UAC prompt) and exits this instance so the
/// elevated one can take the single-instance lock.
pub fn relaunch_elevated(app: &AppHandle) {
    let _ = std::fs::create_dir_all(state::local_data_dir());
    let _ = std::fs::write(crate::show_marker(), b"");
    match admin::run_task() {
        Ok(()) => {
            log::info!("restarting as administrator");
            app.exit(0);
        }
        Err(e) => {
            let _ = std::fs::remove_file(crate::show_marker());
            log::error!("could not restart as administrator: {e}");
        }
    }
}
