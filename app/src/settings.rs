//! Persisted settings: versioned JSON in `%APPDATA%\Lanes\settings.json`.
//!
//! Loading reads the file as untyped JSON, runs every migration from the
//! file's version up to [`CURRENT_VERSION`], then deserializes. Missing fields
//! take their defaults, so additive changes need no migration; renames and
//! restructurings get a step in [`MIGRATIONS`]. A file from a newer version is
//! left untouched (copied to `settings.newer.json`) and defaults are used.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use lanes_core::config::{
    AudioLayout, ClipPreset, ClipQuality, EncoderSettings, Id, ReplaySettings, VideoSettings,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::hotkeys::{Backend, Combo};

pub const CURRENT_VERSION: u32 = 1;

/// One migration step: rewrites a settings document from version `n` to `n + 1`.
type Migration = fn(&mut Value);

/// `MIGRATIONS[i]` upgrades version `i + 1` to `i + 2`.
const MIGRATIONS: &[Migration] = &[];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub version: u32,
    pub general: General,
    pub output: Output,
    pub encoder: EncoderSettings,
    pub replay: ReplaySettings,
    pub clip_presets: Vec<ClipPreset>,
    pub hotkeys: Vec<HotkeySetting>,
    pub profiles: Vec<Profile>,
    pub active_profile: Id,
    pub games: Vec<GameEntry>,
    pub game_detection: GameDetection,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct General {
    pub start_with_windows: bool,
    pub start_minimized: bool,
    /// Start the replay buffer as soon as the app starts.
    pub autostart_replay: bool,
    /// Profile to activate on start; `None` keeps the last active one.
    pub autostart_profile: Option<Id>,
    /// Seconds to wait after logon before starting capture, so boot is not slowed.
    pub autostart_delay_secs: u32,
    /// Run elevated via the scheduled task (see `lanes_platform::admin`).
    pub run_as_admin: bool,
    pub clip_sound: bool,
    /// Output device for the clip sound; `None` = Windows default output.
    pub clip_sound_device: Option<String>,
    /// 0-100.
    pub clip_sound_volume: u8,
    pub clip_toast: bool,
    /// A small on-screen notice (no focus change) when a clip is saved.
    pub clip_overlay: bool,
    pub clip_overlay_corner: OverlayCorner,
    /// Placeholders: {preset} {duration} {game}.
    pub clip_overlay_text: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum OverlayCorner {
    TopLeft,
    #[default]
    TopRight,
    BottomLeft,
    BottomRight,
}

impl Default for General {
    fn default() -> Self {
        Self {
            start_with_windows: false,
            start_minimized: true,
            autostart_replay: false,
            autostart_profile: None,
            autostart_delay_secs: 10,
            run_as_admin: false,
            clip_sound: true,
            clip_sound_device: None,
            clip_sound_volume: 70,
            clip_toast: false,
            clip_overlay: true,
            clip_overlay_corner: OverlayCorner::TopRight,
            clip_overlay_text: "Clip saved · {preset} · {duration} s".into(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum DiskGuardAction {
    #[default]
    Warn,
    Stop,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Output {
    pub folder: String,
    /// Placeholders: {game} {profile} {date} {time} {preset}.
    pub filename_template: String,
    /// Remux finished recordings and clips from MKV to MP4.
    pub remux_to_mp4: bool,
    pub disk_guard_enabled: bool,
    pub disk_guard_min_free_gb: f32,
    pub disk_guard_action: DiskGuardAction,
}

impl Default for Output {
    fn default() -> Self {
        Self {
            folder: default_output_folder(),
            filename_template: "{game}_{date}_{time}".into(),
            remux_to_mp4: true,
            disk_guard_enabled: true,
            disk_guard_min_free_gb: 5.0,
            disk_guard_action: DiskGuardAction::Stop,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HotkeySetting {
    /// Action id: `toggle_recording`, `toggle_pause`, `toggle_replay` or `save_clip:<preset id>`.
    pub action: String,
    pub combo: Option<Combo>,
    #[serde(default)]
    pub backend: Backend,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    pub id: Id,
    pub name: String,
    #[serde(default)]
    pub video: VideoSettings,
    #[serde(default)]
    pub audio: AudioLayout,
    /// Overrides the global replay length.
    #[serde(default)]
    pub replay_length_secs: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GameEntry {
    /// Executable file name, matched case-insensitively (e.g. "cs2.exe").
    pub exe: String,
    pub name: String,
    /// Profile to activate while this game runs.
    #[serde(default)]
    pub profile: Option<Id>,
    #[serde(default = "yes")]
    pub enabled: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct GameDetection {
    pub enabled: bool,
    /// Also treat unknown fullscreen applications as games.
    pub fullscreen_heuristic: bool,
    pub auto_switch_profile: bool,
}

impl Default for GameDetection {
    fn default() -> Self {
        Self { enabled: true, fullscreen_heuristic: true, auto_switch_profile: true }
    }
}

fn yes() -> bool {
    true
}

impl Default for Settings {
    fn default() -> Self {
        let preset = |id: &str, name: &str, secs: u32| ClipPreset {
            id: id.into(),
            name: name.into(),
            length_secs: secs,
            lanes: None,
            quality: ClipQuality::Original,
            folder: None,
            filename_template: None,
        };
        let hotkey = |action: &str, combo: &str| HotkeySetting {
            action: action.into(),
            combo: combo.parse().ok(),
            backend: Backend::Auto,
        };
        Self {
            version: CURRENT_VERSION,
            general: General::default(),
            output: Output::default(),
            encoder: EncoderSettings::default(),
            replay: ReplaySettings::default(),
            clip_presets: vec![preset("short", "Last 30 seconds", 30), preset("long", "Last 2 minutes", 120)],
            // Avoids ShadowPlay's Alt+F9/Alt+F10 defaults.
            hotkeys: vec![
                hotkey("toggle_recording", "Ctrl+Shift+F9"),
                hotkey("toggle_pause", "Ctrl+Shift+F8"),
                hotkey("toggle_replay", "Ctrl+Shift+F11"),
                hotkey("save_clip:short", "Ctrl+Shift+F10"),
                hotkey("save_clip:long", "Ctrl+Shift+F12"),
            ],
            profiles: vec![Profile {
                id: "default".into(),
                name: "Default".into(),
                video: VideoSettings::default(),
                audio: AudioLayout::default(),
                replay_length_secs: None,
            }],
            active_profile: "default".into(),
            games: Vec::new(),
            game_detection: GameDetection::default(),
        }
    }
}

impl Settings {
    pub fn profile(&self, id: &str) -> Option<&Profile> {
        self.profiles.iter().find(|p| p.id == id)
    }

    /// The active profile, falling back to the first one.
    pub fn active(&self) -> &Profile {
        self.profile(&self.active_profile).or(self.profiles.first()).expect("validated: at least one profile")
    }

    /// Repairs invariants the rest of the app relies on. Returns a list of
    /// fixes applied, for logging.
    pub fn validate(&mut self) -> Vec<String> {
        let mut fixes = Vec::new();
        if self.profiles.is_empty() {
            self.profiles = Settings::default().profiles;
            fixes.push("no profiles; restored the default profile".into());
        }
        if self.profile(&self.active_profile).is_none() {
            self.active_profile = self.profiles[0].id.clone();
            fixes.push(format!("active profile missing; using \"{}\"", self.profiles[0].name));
        }
        for p in &mut self.profiles {
            p.video.fps = p.video.fps.clamp(1, 240);
            let lane_ids: Vec<_> = p.audio.lanes.iter().map(|l| l.id.clone()).collect();
            for s in &mut p.audio.sources {
                let before = s.lanes.len();
                s.lanes.retain(|l| lane_ids.contains(l));
                if s.lanes.len() != before {
                    fixes.push(format!("source \"{}\" referenced deleted lanes", s.name));
                }
            }
        }
        self.replay.length_secs = self.replay.length_secs.clamp(5, 3600);
        self.replay.ram_cap_mb = self.replay.ram_cap_mb.clamp(64, 32 * 1024);
        self.encoder.keyframe_interval_secs = self.encoder.keyframe_interval_secs.clamp(0.25, 10.0);
        self.encoder.quality = self.encoder.quality.min(51);
        for c in &mut self.clip_presets {
            c.length_secs = c.length_secs.clamp(1, 3600);
        }
        fixes
    }
}

/// `%USERPROFILE%\Videos\Lanes`, or the current directory if that is unknown.
pub fn default_output_folder() -> String {
    std::env::var_os("USERPROFILE")
        .map(|home| PathBuf::from(home).join("Videos").join("Lanes"))
        .unwrap_or_else(|| PathBuf::from("Lanes"))
        .to_string_lossy()
        .into_owned()
}

/// `%APPDATA%\Lanes`.
pub fn config_dir() -> PathBuf {
    std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(".")).join("Lanes")
}

#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    #[error("settings file is from a newer Lanes (version {0}); using defaults")]
    Newer(u32),
    #[error("settings file is not valid: {0}")]
    Invalid(#[from] serde_json::Error),
}

/// Parses a settings document, migrating it to the current version.
pub fn parse(text: &str) -> Result<Settings, LoadError> {
    let mut doc: Value = serde_json::from_str(text)?;
    let version = doc.get("version").and_then(Value::as_u64).unwrap_or(1) as u32;
    if version > CURRENT_VERSION {
        return Err(LoadError::Newer(version));
    }
    migrate(&mut doc, version, MIGRATIONS);
    let mut settings: Settings = serde_json::from_value(doc)?;
    settings.version = CURRENT_VERSION;
    settings.validate();
    Ok(settings)
}

/// Runs `steps[from - 1..]` on `doc` and stamps the resulting version.
fn migrate(doc: &mut Value, from: u32, steps: &[Migration]) {
    let start = from.saturating_sub(1) as usize;
    for (i, step) in steps.iter().enumerate().skip(start) {
        step(doc);
        if let Some(obj) = doc.as_object_mut() {
            obj.insert("version".into(), Value::from(i as u32 + 2));
        }
    }
}

/// Settings file storage.
pub struct Store {
    path: PathBuf,
}

impl Store {
    pub fn new(dir: &Path) -> Self {
        Self { path: dir.join("settings.json") }
    }

    /// Loads settings, falling back to defaults (and keeping a copy of an
    /// unreadable file) so a bad file never prevents startup.
    pub fn load(&self) -> Settings {
        let text = match fs::read_to_string(&self.path) {
            Ok(t) => t,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Settings::default(),
            Err(e) => {
                log::error!("could not read {}: {e}", self.path.display());
                return Settings::default();
            }
        };
        match parse(&text) {
            Ok(s) => s,
            Err(e) => {
                let backup = self.path.with_extension(match e {
                    LoadError::Newer(_) => "newer.json",
                    LoadError::Invalid(_) => "invalid.json",
                });
                let _ = fs::copy(&self.path, &backup);
                log::error!("{e} (copy kept at {})", backup.display());
                Settings::default()
            }
        }
    }

    /// Writes atomically: temp file, then rename over the old file.
    pub fn save(&self, settings: &Settings) -> io::Result<()> {
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir)?;
        }
        let tmp = self.path.with_extension("json.tmp");
        fs::write(&tmp, serde_json::to_vec_pretty(settings).map_err(io::Error::other)?)?;
        fs::rename(&tmp, &self.path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_roundtrip() {
        let s = Settings::default();
        let text = serde_json::to_string(&s).unwrap();
        assert_eq!(parse(&text).unwrap(), s);
    }

    #[test]
    fn missing_fields_take_defaults() {
        let s = parse(r#"{ "version": 1, "output": { "folder": "D:\\Clips" } }"#).unwrap();
        assert_eq!(s.output.folder, "D:\\Clips");
        assert_eq!(s.output.filename_template, Output::default().filename_template);
        assert_eq!(s.encoder, EncoderSettings::default());
        assert!(!s.profiles.is_empty());
    }

    #[test]
    fn unknown_fields_are_ignored() {
        assert!(parse(r#"{ "version": 1, "somethingRemoved": 3 }"#).is_ok());
    }

    #[test]
    fn missing_version_is_treated_as_v1() {
        assert_eq!(parse("{}").unwrap().version, CURRENT_VERSION);
    }

    #[test]
    fn newer_version_is_rejected() {
        let text = format!(r#"{{ "version": {} }}"#, CURRENT_VERSION + 1);
        assert!(matches!(parse(&text), Err(LoadError::Newer(_))));
    }

    #[test]
    fn invalid_json_is_rejected() {
        assert!(matches!(parse("{ nope"), Err(LoadError::Invalid(_))));
    }

    #[test]
    fn migrations_run_in_order_from_the_file_version() {
        fn v1_to_v2(doc: &mut Value) {
            // e.g. a rename: "outDir" -> "output.folder"
            let dir = doc.as_object_mut().unwrap().remove("outDir").unwrap();
            doc["output"] = serde_json::json!({ "folder": dir });
        }
        fn v2_to_v3(doc: &mut Value) {
            let folder = doc["output"]["folder"].as_str().unwrap().to_uppercase();
            doc["output"]["folder"] = Value::from(folder);
        }
        let steps: &[Migration] = &[v1_to_v2, v2_to_v3];

        let mut doc = serde_json::json!({ "version": 1, "outDir": "c:\\x" });
        migrate(&mut doc, 1, steps);
        assert_eq!(doc["output"]["folder"], "C:\\X");
        assert_eq!(doc["version"], 3);

        // A v2 file only runs the second step.
        let mut doc = serde_json::json!({ "version": 2, "output": { "folder": "d:\\y" } });
        migrate(&mut doc, 2, steps);
        assert_eq!(doc["output"]["folder"], "D:\\Y");
        assert_eq!(doc["version"], 3);
    }

    #[test]
    fn validate_repairs_dangling_references() {
        let mut s = Settings { active_profile: "gone".into(), ..Default::default() };
        s.profiles[0].audio.sources[0].lanes.push("deleted-lane".into());
        let fixes = s.validate();
        assert_eq!(s.active_profile, "default");
        assert!(!s.profiles[0].audio.sources[0].lanes.contains(&"deleted-lane".to_string()));
        assert_eq!(fixes.len(), 2);
    }

    #[test]
    fn store_saves_and_loads() {
        let dir = std::env::temp_dir().join(format!("lanes-settings-test-{}", std::process::id()));
        let store = Store::new(&dir);
        let mut s = Settings::default();
        s.encoder.bitrate_kbps = 12_345;
        store.save(&s).unwrap();
        assert_eq!(store.load(), s);
        let _ = fs::remove_dir_all(dir);
    }
}
