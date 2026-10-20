//! Serializable configuration shared by the engine, the settings file and the UI.
//!
//! These types are part of the persisted settings format, so changes need a
//! settings migration in the app crate (see `app/src/settings.rs`).

use serde::{Deserialize, Serialize};

/// Stable identifier for lanes, sources, profiles and clip presets.
pub type Id = String;

// ---------------------------------------------------------------- encoder ---

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum EncoderBackend {
    /// Pick the best available hardware encoder, falling back to x264.
    #[default]
    Auto,
    Nvenc,
    Amf,
    Qsv,
    X264,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum VideoCodec {
    #[default]
    H264,
    Hevc,
    Av1,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum RateControl {
    Cbr,
    #[default]
    Vbr,
    /// Constant quality (CQP / CRF / CQ depending on the encoder).
    Cqp,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum EncoderPreset {
    /// Lowest GPU/CPU load.
    Performance,
    #[default]
    Balanced,
    Quality,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum AudioCodec {
    #[default]
    Aac,
    Opus,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct EncoderSettings {
    pub backend: EncoderBackend,
    pub codec: VideoCodec,
    pub rate_control: RateControl,
    /// Target bitrate for CBR/VBR.
    pub bitrate_kbps: u32,
    /// Peak bitrate for VBR.
    pub max_bitrate_kbps: u32,
    /// Quality for CQP, 0 (best) to 51.
    pub quality: u8,
    pub keyframe_interval_secs: f32,
    pub preset: EncoderPreset,
    pub audio_codec: AudioCodec,
    pub audio_bitrate_kbps: u32,
}

impl Default for EncoderSettings {
    fn default() -> Self {
        Self {
            backend: EncoderBackend::Auto,
            codec: VideoCodec::H264,
            rate_control: RateControl::Vbr,
            bitrate_kbps: 20_000,
            max_bitrate_kbps: 30_000,
            quality: 23,
            keyframe_interval_secs: 1.0,
            preset: EncoderPreset::Balanced,
            audio_codec: AudioCodec::Aac,
            audio_bitrate_kbps: 160,
        }
    }
}

// ------------------------------------------------------------------ video ---

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum CaptureMode {
    /// Capture the detected game's window, or its monitor if window capture fails.
    #[default]
    FollowGame,
    /// Capture the selected monitors.
    Monitors,
    /// Capture a specific window, chosen by executable name.
    Window,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum MultiMonitorLayout {
    /// One video file per monitor.
    #[default]
    SeparateFiles,
    /// All selected monitors composed side by side into one video.
    Canvas,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum CaptureApi {
    /// Windows.Graphics.Capture, falling back to DXGI Desktop Duplication.
    #[default]
    Auto,
    Wgc,
    Duplication,
}

/// Per-monitor capture settings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MonitorSelection {
    /// Stable monitor id (the display device path).
    pub id: String,
    /// Frame-rate cap for this monitor; `None` uses the profile fps.
    #[serde(default)]
    pub fps_cap: Option<u32>,
    /// Output height cap for this monitor (width follows the aspect ratio).
    #[serde(default)]
    pub max_height: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct VideoSettings {
    pub mode: CaptureMode,
    pub api: CaptureApi,
    /// Monitors captured in `Monitors` mode. Empty means the primary monitor.
    pub monitors: Vec<MonitorSelection>,
    pub layout: MultiMonitorLayout,
    /// Executable captured in `Window` mode (e.g. "game.exe").
    pub window_exe: Option<String>,
    pub fps: u32,
    /// Output height (width follows the aspect ratio); `None` keeps native resolution.
    pub output_height: Option<u32>,
    pub capture_cursor: bool,
}

impl Default for VideoSettings {
    fn default() -> Self {
        Self {
            mode: CaptureMode::FollowGame,
            api: CaptureApi::Auto,
            monitors: Vec::new(),
            layout: MultiMonitorLayout::SeparateFiles,
            window_exe: None,
            fps: 60,
            output_height: None,
            capture_cursor: true,
        }
    }
}

// ------------------------------------------------------------------ audio ---

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum SourceKind {
    /// Everything played on an output device (loopback). `device: None` follows the default device.
    SystemOutput {
        #[serde(default)]
        device: Option<String>,
    },
    /// Audio from one application and its child processes (process loopback, Windows 10 2004+).
    App { exe: String },
    /// A microphone or other input device. `device: None` follows the default device.
    Input {
        #[serde(default)]
        device: Option<String>,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NoiseGate {
    /// Level below which the gate closes, in dBFS.
    pub threshold_db: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioSource {
    pub id: Id,
    pub name: String,
    pub kind: SourceKind,
    #[serde(default)]
    pub volume_db: f32,
    #[serde(default)]
    pub muted: bool,
    /// Downmix to mono (useful for mics that only use one channel).
    #[serde(default)]
    pub mono: bool,
    #[serde(default)]
    pub noise_gate: Option<NoiseGate>,
    #[serde(default)]
    pub noise_suppression: bool,
    /// Lanes this source feeds. A source may feed several lanes.
    #[serde(default)]
    pub lanes: Vec<Id>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Lane {
    pub id: Id,
    pub name: String,
    #[serde(default)]
    pub volume_db: f32,
    #[serde(default)]
    pub muted: bool,
    #[serde(default)]
    pub mono: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AudioLayout {
    pub lanes: Vec<Lane>,
    pub sources: Vec<AudioSource>,
}

impl Default for AudioLayout {
    /// "Game" (system audio) and "Mic" (default input): the Milestone 3 setup.
    fn default() -> Self {
        let lane = |id: &str, name: &str| Lane { id: id.into(), name: name.into(), volume_db: 0.0, muted: false, mono: false };
        Self {
            lanes: vec![lane("game", "Game"), lane("mic", "Mic")],
            sources: vec![
                AudioSource {
                    id: "system".into(),
                    name: "System audio".into(),
                    kind: SourceKind::SystemOutput { device: None },
                    volume_db: 0.0,
                    muted: false,
                    mono: false,
                    noise_gate: None,
                    noise_suppression: false,
                    lanes: vec!["game".into()],
                },
                AudioSource {
                    id: "mic".into(),
                    name: "Microphone".into(),
                    kind: SourceKind::Input { device: None },
                    volume_db: 0.0,
                    muted: false,
                    mono: true,
                    noise_gate: Some(NoiseGate { threshold_db: -50.0 }),
                    noise_suppression: false,
                    lanes: vec!["mic".into()],
                },
            ],
        }
    }
}

// ----------------------------------------------------------------- replay ---

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ReplayStorage {
    #[default]
    Ram,
    Disk,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ReplaySettings {
    pub length_secs: u32,
    /// Hard cap on buffered bytes; the oldest GOPs are dropped first.
    pub ram_cap_mb: u32,
    pub storage: ReplayStorage,
}

impl Default for ReplaySettings {
    fn default() -> Self {
        Self { length_secs: 120, ram_cap_mb: 1024, storage: ReplayStorage::Ram }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ClipQuality {
    /// Copy the buffered packets as they are (instant).
    #[default]
    Original,
    /// Re-encode in the background at a lower bitrate.
    High,
    Medium,
    Low,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipPreset {
    pub id: Id,
    pub name: String,
    pub length_secs: u32,
    /// Lanes to include; `None` includes all.
    #[serde(default)]
    pub lanes: Option<Vec<Id>>,
    #[serde(default)]
    pub quality: ClipQuality,
    /// Overrides the output folder.
    #[serde(default)]
    pub folder: Option<String>,
    /// Overrides the filename template.
    #[serde(default)]
    pub filename_template: Option<String>,
}
