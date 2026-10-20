//! The engine: owns the capture/encode session and turns user commands
//! (record, pause, replay, save clip) into thread plumbing.
//!
//! A *session* (GPU device, video streams, audio session, packet router)
//! exists while recording or the replay buffer is on, and is torn down when
//! both are off, so an idle app uses no capture or encode resources. All
//! control logic runs on one control thread; callers talk to it through a
//! channel and never block on capture or encoding.

pub mod audio;
pub mod naming;
pub mod router;
pub mod video;

use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crossbeam_channel::{Receiver, Sender, bounded, unbounded};
use serde::Serialize;

use self::audio::AudioSession;
use self::router::{FilePlan, Router, RouterCommand};
use self::video::{LayerPlan, VideoEvent, VideoPlan, VideoStart, VideoStream};
use crate::Timestamp;
use crate::audio::devices::{DeviceEvent, DeviceWatcher};
use crate::audio::mixer::MeterFrame;
use crate::audio::wasapi::CaptureStatus;
use crate::av::StreamId;
use crate::capture::{Target, monitors};
use crate::config::{
    AudioLayout, CaptureMode, ClipPreset, ClipQuality, EncoderSettings, MultiMonitorLayout, ReplaySettings, VideoCodec,
    VideoSettings,
};
use crate::convert::{Rect, scaled_size};
use crate::encode::video::VideoEncoderConfig;
use crate::error::{Component, Error, Result};
use crate::games::DetectedGame;
use crate::gpu::Gpu;
use crate::mux::Container;
use crate::replay::ReplayLimits;
use crate::sysinfo::CpuMeter;
use crate::util::{Priority, spawn};

// ------------------------------------------------------------ public types ---

#[derive(Clone, Debug)]
pub struct DiskGuard {
    pub min_free_bytes: u64,
    /// Stop recording (true) or only warn (false).
    pub stop: bool,
}

#[derive(Clone, Debug)]
pub struct OutputConfig {
    pub folder: PathBuf,
    pub filename_template: String,
    pub remux_to_mp4: bool,
    pub disk_guard: Option<DiskGuard>,
}

/// Everything the engine needs, resolved from settings + the active profile.
#[derive(Clone, Debug)]
pub struct EngineConfig {
    pub profile_id: String,
    pub profile_name: String,
    pub encoder: EncoderSettings,
    pub video: VideoSettings,
    pub audio: AudioLayout,
    pub replay: ReplaySettings,
    pub presets: Vec<ClipPreset>,
    pub output: OutputConfig,
    /// For disk-backed replay segments.
    pub temp_dir: PathBuf,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum RecordingState {
    Idle,
    #[serde(rename_all = "camelCase")]
    Recording {
        /// Unix ms such that `now - sinceMs` is the recorded (unpaused) duration.
        since_ms: i64,
        paused: bool,
        files: Vec<String>,
    },
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplayState {
    pub active: bool,
    pub buffered_secs: f64,
    pub buffered_bytes: u64,
    pub capacity_secs: u32,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineStatus {
    pub recording: RecordingState,
    pub replay: ReplayState,
    pub active_profile: String,
    pub game: Option<DetectedGame>,
    pub targets: Vec<String>,
    pub encoder: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stats {
    pub fps: f64,
    pub target_fps: u32,
    pub dropped_frames: u64,
    pub duplicated_frames: u64,
    pub encode_ms_avg: f64,
    pub encoder_load: f64,
    pub encoder_queue: u64,
    pub buffer_secs: f64,
    pub buffer_bytes: u64,
    pub disk_write_bps: f64,
    pub free_disk_bytes: u64,
    pub cpu_percent: f32,
    pub memory_bytes: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipSaved {
    pub path: String,
    /// Preset id.
    pub preset: String,
    pub duration_secs: f64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineError {
    pub source: String,
    pub message: String,
    pub recoverable: bool,
}

/// Receives engine events. Called from engine threads; implementations must be cheap.
pub trait EventSink: Send + Sync + 'static {
    fn status(&self, status: &EngineStatus);
    fn meters(&self, meters: &MeterFrame);
    fn stats(&self, stats: &Stats);
    fn clip_saved(&self, clip: &ClipSaved);
    fn recording_saved(&self, paths: &[PathBuf]);
    fn error(&self, error: &EngineError);
}

enum Command {
    Configure(Box<EngineConfig>),
    StartRecording,
    StopRecording,
    TogglePause,
    StartReplay,
    StopReplay,
    SaveClip(String),
    SetGame(Option<DetectedGame>),
    Shutdown,
}

pub struct Engine {
    commands: Sender<Command>,
    status: Arc<Mutex<EngineStatus>>,
    thread: Option<JoinHandle<Option<()>>>,
}

impl Engine {
    pub fn start(config: EngineConfig, sink: Arc<dyn EventSink>) -> std::io::Result<Self> {
        crate::av::init_logging();
        let status = Arc::new(Mutex::new(EngineStatus {
            recording: RecordingState::Idle,
            replay: ReplayState { capacity_secs: config.replay.length_secs, ..Default::default() },
            active_profile: config.profile_id.clone(),
            game: None,
            targets: Vec::new(),
            encoder: None,
        }));
        let (commands, rx) = unbounded();
        let shared = status.clone();
        let thread = spawn("lanes-engine", Priority::Normal, move || Control::new(config, sink, shared).run(rx))?;
        Ok(Self { commands, status, thread: Some(thread) })
    }

    pub fn status(&self) -> EngineStatus {
        self.status.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn is_recording(&self) -> bool {
        matches!(self.status().recording, RecordingState::Recording { .. })
    }

    pub fn is_replay_active(&self) -> bool {
        self.status().replay.active
    }

    pub fn configure(&self, config: EngineConfig) {
        let _ = self.commands.send(Command::Configure(Box::new(config)));
    }
    pub fn start_recording(&self) {
        let _ = self.commands.send(Command::StartRecording);
    }
    pub fn stop_recording(&self) {
        let _ = self.commands.send(Command::StopRecording);
    }
    pub fn toggle_recording(&self) {
        if self.is_recording() { self.stop_recording() } else { self.start_recording() }
    }
    pub fn toggle_pause(&self) {
        let _ = self.commands.send(Command::TogglePause);
    }
    pub fn start_replay(&self) {
        let _ = self.commands.send(Command::StartReplay);
    }
    pub fn stop_replay(&self) {
        let _ = self.commands.send(Command::StopReplay);
    }
    pub fn toggle_replay(&self) {
        if self.is_replay_active() { self.stop_replay() } else { self.start_replay() }
    }
    pub fn save_clip(&self, preset_id: &str) {
        let _ = self.commands.send(Command::SaveClip(preset_id.into()));
    }
    pub fn set_game(&self, game: Option<DetectedGame>) {
        let _ = self.commands.send(Command::SetGame(game));
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        let _ = self.commands.send(Command::Shutdown);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

// ---------------------------------------------------------------- control ---

struct Session {
    video: Vec<VideoStream>,
    audio: Option<AudioSession>,
    router: Router,
    targets: Vec<String>,
    encoder_label: String,
    /// The backend/codec actually used (after resolving Auto and fallbacks).
    encoder_choice: (crate::config::EncoderBackend, VideoCodec),
    _watcher: Option<DeviceWatcher>,
    /// Stats deltas.
    last_frames: u64,
    last_encode_ns: u64,
    last_recorded: u64,
    last_stats_at: Instant,
}

struct RecordingInfo {
    start_unix_ms: i64,
    paused_ms: i64,
    paused_since: Option<Instant>,
    files: Vec<PathBuf>,
}

enum Internal {
    Device(DeviceEvent),
    AudioStatus(String, CaptureStatus),
}

struct Control {
    config: EngineConfig,
    sink: Arc<dyn EventSink>,
    status: Arc<Mutex<EngineStatus>>,
    session: Option<Session>,
    recording: Option<RecordingInfo>,
    replay_on: bool,
    game: Option<DetectedGame>,
    video_events: (Sender<VideoEvent>, Receiver<VideoEvent>),
    internal: (Sender<Internal>, Receiver<Internal>),
    cpu: CpuMeter,
    disk_warned: bool,
    last_tick: Instant,
    last_audio_sync: Instant,
}

fn unix_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

impl Control {
    fn new(config: EngineConfig, sink: Arc<dyn EventSink>, status: Arc<Mutex<EngineStatus>>) -> Self {
        Self {
            config,
            sink,
            status,
            session: None,
            recording: None,
            replay_on: false,
            game: None,
            video_events: unbounded(),
            internal: unbounded(),
            cpu: CpuMeter::default(),
            disk_warned: false,
            last_tick: Instant::now(),
            last_audio_sync: Instant::now(),
        }
    }

    fn run(mut self, commands: Receiver<Command>) {
        // Probe encoders once in the background of startup, so the first
        // recording starts instantly.
        if let Ok(gpu) = Gpu::new(None) {
            let _ = crate::encode::available(&gpu);
        }
        self.publish();
        loop {
            let video_rx = self.video_events.1.clone();
            let internal_rx = self.internal.1.clone();
            crossbeam_channel::select! {
                recv(commands) -> cmd => match cmd {
                    Ok(Command::Shutdown) | Err(_) => break,
                    Ok(cmd) => self.command(cmd),
                },
                recv(video_rx) -> ev => if let Ok(ev) = ev { self.video_event(ev) },
                recv(internal_rx) -> ev => if let Ok(ev) = ev { self.internal_event(ev) },
                default(Duration::from_millis(500)) => {},
            }
            if self.last_tick.elapsed() >= Duration::from_secs(1) {
                self.last_tick = Instant::now();
                self.tick();
            }
        }
        if self.recording.is_some() {
            self.stop_recording();
        }
        self.session = None;
    }

    fn error(&self, component: Component, message: impl Into<String>, recoverable: bool) {
        let message = message.into();
        log::error!("{component}: {message}");
        self.sink.error(&EngineError { source: component.to_string(), message, recoverable });
    }

    fn command(&mut self, cmd: Command) {
        match cmd {
            Command::Configure(config) => self.configure(*config),
            Command::StartRecording => self.start_recording(),
            Command::StopRecording => self.stop_recording(),
            Command::TogglePause => self.toggle_pause(),
            Command::StartReplay => self.start_replay(),
            Command::StopReplay => self.stop_replay(),
            Command::SaveClip(preset) => self.save_clip(&preset),
            Command::SetGame(game) => self.set_game(game),
            Command::Shutdown => {}
        }
        self.publish();
    }

    fn configure(&mut self, config: EngineConfig) {
        let replay_changed = config.replay != self.config.replay;
        let audio_changed = config.audio != self.config.audio;
        self.config = config;
        let replay_cfg = self.replay_config();
        if let Some(s) = &mut self.session {
            if audio_changed
                && let Some(a) = &mut s.audio {
                    a.update(&self.config.audio);
                }
            if replay_changed && self.replay_on {
                let _ = s.router.commands.send(RouterCommand::SetReplay(Some(replay_cfg)));
            }
        }
        // Video, encoder and lane-structure changes apply to the next session.
    }

    fn replay_config(&self) -> (ReplayLimits, crate::config::ReplayStorage, PathBuf) {
        (
            ReplayLimits {
                duration: Duration::from_secs(u64::from(self.config.replay.length_secs)),
                max_bytes: self.config.replay.ram_cap_mb as usize * 1024 * 1024,
            },
            self.config.replay.storage,
            self.config.temp_dir.join("replay"),
        )
    }

    // ------------------------------------------------------------ session ---

    fn ensure_session(&mut self) -> bool {
        if self.session.is_some() {
            return true;
        }
        match self.build_session() {
            Ok(s) => {
                self.session = Some(s);
                true
            }
            Err(e) => {
                self.error(Component::Engine, format!("could not start capture: {e}"), e.is_recoverable());
                false
            }
        }
    }

    fn build_session(&mut self) -> Result<Session> {
        let plans = self.plan_video();
        let first_monitor = plans.first().and_then(|p| p.layers.first()).and_then(|l| match &l.target {
            Target::Monitor { hmonitor, .. } => monitors::list().into_iter().find(|m| m.hmonitor == *hmonitor),
            Target::Window { .. } => None,
        });
        let gpu = Gpu::new(first_monitor.map(|m| m.adapter_luid))?;
        let available = crate::encode::available(&gpu);
        let (backend, codec) = crate::encode::resolve(available, self.config.encoder.backend, self.config.encoder.codec)
            .ok_or_else(|| Error::Unsupported("no usable video encoder (not even x264)".into()))?;
        if (backend, codec) != (self.config.encoder.backend, self.config.encoder.codec) {
            log::info!("encoder: using {} {}", crate::encode::backend_label(backend), crate::encode::codec_label(codec));
        }

        let t0 = Timestamp::now();
        let (packets_tx, packets_rx) = bounded(4096);
        let mut video = Vec::new();
        for mut plan in plans {
            clamp_size(&mut plan, codec);
            let stream = VideoStream::start(VideoStart {
                gpu: &gpu,
                plan,
                encoder: VideoEncoderConfig { backend, codec, width: 0, height: 0, fps: 0, settings: self.config.encoder.clone() },
                api: self.config.video.api,
                cursor: self.config.video.capture_cursor,
                t0,
                packets: packets_tx.clone(),
                events: self.video_events.0.clone(),
            })?;
            video.push(stream);
        }

        let sink = self.sink.clone();
        let internal = self.internal.0.clone();
        let audio = if self.config.audio.lanes.is_empty() {
            None
        } else {
            Some(AudioSession::start(
                t0,
                &self.config.audio,
                &self.config.encoder,
                packets_tx.clone(),
                Arc::new(move |m: &MeterFrame| sink.meters(m)),
                Arc::new(move |id: &str, st: CaptureStatus| {
                    let _ = internal.send(Internal::AudioStatus(id.to_string(), st));
                }),
            )?)
        };
        drop(packets_tx);

        let internal = self.internal.0.clone();
        let watcher = DeviceWatcher::new(move |ev| {
            let _ = internal.send(Internal::Device(ev));
        })
        .map_err(|e| log::warn!("device notifications unavailable: {e}"))
        .ok();

        let mut streams: Vec<_> = video.iter().map(|v| v.info.clone()).collect();
        if let Some(a) = &audio {
            streams.extend(a.streams.iter().cloned());
        }
        let sink = self.sink.clone();
        let router = Router::start(
            t0,
            streams,
            packets_rx,
            video.iter().map(|v| v.force_keyframe.clone()).collect(),
            Arc::new(move |e: Error| {
                sink.error(&EngineError { source: "recorder".into(), message: e.to_string(), recoverable: e.is_recoverable() })
            }),
        )?;
        let targets = video.iter().flat_map(|v| v.plan.layers.iter().map(|l| l.target.label().to_string())).collect();
        let encoder_label = video.first().map(|v| v.encoder_label.clone()).unwrap_or_default();
        log::info!("capture session started ({} video stream(s), {} lane(s))", video.len(), self.config.audio.lanes.len());
        Ok(Session {
            encoder_choice: (backend, codec),
            video,
            audio,
            router,
            targets,
            encoder_label,
            _watcher: watcher,
            last_frames: 0,
            last_encode_ns: 0,
            last_recorded: 0,
            last_stats_at: Instant::now(),
        })
    }

    /// Ends the session when nothing needs it any more.
    fn maybe_end_session(&mut self) {
        if self.recording.is_none() && !self.replay_on && self.session.is_some() {
            self.session = None;
            log::info!("capture session ended");
        }
    }

    fn plan_video(&self) -> Vec<VideoPlan> {
        let v = &self.config.video;
        let monitors = monitors::list();
        let primary = monitors.first().cloned();
        let fps = v.fps.clamp(1, 240);
        let monitor_plan = |index: u8, m: &monitors::MonitorInfo, fps: u32, max_h: Option<u32>| {
            let (w, h) = scaled_size(m.width, m.height, max_h);
            VideoPlan {
                index,
                title: m.name.clone(),
                layers: vec![LayerPlan {
                    target: Target::Monitor { hmonitor: m.hmonitor, name: m.name.clone() },
                    area: Rect::full(w, h),
                    max_fps: fps,
                }],
                width: w,
                height: h,
                fps,
            }
        };
        let window_plan = |w: &crate::capture::windows_list::WindowInfo, name: &str| {
            let (ww, wh) = ((w.rect.2 - w.rect.0).max(2) as u32, (w.rect.3 - w.rect.1).max(2) as u32);
            // Size from the window's monitor, so later resizes (or a move to
            // fullscreen) still fit.
            let mon = monitors.iter().find(|m| {
                let (cx, cy) = ((w.rect.0 + w.rect.2) / 2, (w.rect.1 + w.rect.3) / 2);
                cx >= m.x && cx < m.x + m.width as i32 && cy >= m.y && cy < m.y + m.height as i32
            });
            let (bw, bh) = mon.map(|m| (m.width, m.height)).unwrap_or((ww, wh));
            let (ow, oh) = scaled_size(bw, bh, v.output_height);
            VideoPlan {
                index: 0,
                title: name.to_string(),
                layers: vec![LayerPlan { target: Target::Window { hwnd: w.hwnd, name: name.to_string() }, area: Rect::full(ow, oh), max_fps: fps }],
                width: ow,
                height: oh,
                fps,
            }
        };
        let fallback = || primary.as_ref().map(|m| vec![monitor_plan(0, m, fps, v.output_height)]).unwrap_or_default();

        match v.mode {
            CaptureMode::FollowGame => match self.game.as_ref().and_then(|g| crate::games::game_window(g).map(|w| (g, w))) {
                Some((g, w)) => vec![window_plan(&w, &g.name)],
                None => fallback(),
            },
            CaptureMode::Window => match v.window_exe.as_deref().and_then(crate::capture::windows_list::main_window_of) {
                Some(w) => {
                    let name = w.exe.clone();
                    vec![window_plan(&w, &name)]
                }
                None => {
                    self.error(Component::Capture, format!("window of {} not found; capturing the primary monitor", v.window_exe.as_deref().unwrap_or("?")), true);
                    fallback()
                }
            },
            CaptureMode::Monitors => {
                let selected: Vec<(monitors::MonitorInfo, Option<u32>, Option<u32>)> = if v.monitors.is_empty() {
                    primary.iter().map(|m| (m.clone(), None, None)).collect()
                } else {
                    v.monitors
                        .iter()
                        .filter_map(|s| monitors.iter().find(|m| m.id == s.id).map(|m| (m.clone(), s.fps_cap, s.max_height)))
                        .collect()
                };
                if selected.is_empty() {
                    return fallback();
                }
                match v.layout {
                    MultiMonitorLayout::SeparateFiles => unique_titles(selected
                        .iter()
                        .enumerate()
                        .map(|(i, (m, cap, max_h))| {
                            let h = match (max_h, v.output_height) {
                                (Some(a), Some(b)) => Some((*a).min(b)),
                                (a, b) => a.or(b),
                            };
                            monitor_plan(i as u8, m, cap.map_or(fps, |c| c.clamp(1, fps)), h)
                        })
                        .collect()),
                    MultiMonitorLayout::Canvas => vec![canvas_plan(&selected, fps, v.output_height)],
                }
            }
        }
    }

    fn file_plans(&self, base_name: &str, ext: &str, folder: &std::path::Path, lanes: Option<&[String]>) -> Vec<FilePlan> {
        let Some(session) = &self.session else { return Vec::new() };
        let audio_ids: Vec<StreamId> = self
            .config
            .audio
            .lanes
            .iter()
            .enumerate()
            .filter(|(_, l)| lanes.is_none_or(|sel| sel.contains(&l.id)))
            .map(|(i, _)| StreamId::Audio(i as u8))
            .filter(|id| session.audio.as_ref().is_some_and(|a| a.streams.iter().any(|s| s.id == *id)))
            .collect();
        let multi = session.video.len() > 1;
        session
            .video
            .iter()
            .map(|v| {
                let name = if multi { format!("{base_name}_{}", naming::sanitize(&v.plan.title)) } else { base_name.to_string() };
                let mut streams = vec![v.info.id];
                streams.extend(audio_ids.iter().copied());
                FilePlan { path: naming::unique_path(folder, &name, ext), streams }
            })
            .collect()
    }

    fn game_name(&self) -> String {
        self.game.as_ref().map(|g| g.name.clone()).unwrap_or_default()
    }

    // ---------------------------------------------------------- recording ---

    fn start_recording(&mut self) {
        if self.recording.is_some() || !self.ensure_session() {
            return;
        }
        if let Some(guard) = &self.config.output.disk_guard
            && crate::sysinfo::free_disk_bytes(&self.config.output.folder).is_some_and(|f| f < guard.min_free_bytes) {
                self.error(Component::Recorder, "not enough free disk space to start recording", false);
                self.maybe_end_session();
                return;
            }
        let name = naming::render(&self.config.output.filename_template, &naming::NameParts::now(&self.game_name(), &self.config.profile_name, ""));
        let files = self.file_plans(&name, "mkv", &self.config.output.folder.clone(), None);
        let (reply, rx) = bounded(1);
        let session = self.session.as_ref().expect("ensured");
        let _ = session.router.commands.send(RouterCommand::StartRecording { files: files.clone(), reply });
        match rx.recv_timeout(Duration::from_secs(5)) {
            Ok(Ok(())) => {
                log::info!("recording started");
                self.recording = Some(RecordingInfo {
                    start_unix_ms: unix_ms(),
                    paused_ms: 0,
                    paused_since: None,
                    files: files.into_iter().map(|f| f.path).collect(),
                });
            }
            Ok(Err(e)) => {
                self.error(Component::Recorder, format!("could not start recording: {e}"), false);
                self.maybe_end_session();
            }
            Err(_) => self.error(Component::Recorder, "recorder did not respond", true),
        }
    }

    fn stop_recording(&mut self) {
        let Some(info) = self.recording.take() else { return };
        let mut paths = info.files.clone();
        if let Some(s) = &self.session {
            let (reply, rx) = bounded(1);
            let _ = s.router.commands.send(RouterCommand::StopRecording { at: Timestamp::now(), reply });
            if let Ok(p) = rx.recv_timeout(Duration::from_secs(10)) {
                paths = p;
            }
        }
        log::info!("recording stopped ({} file(s))", paths.len());
        self.maybe_end_session();
        self.finish_files(paths);
    }

    /// Remuxes finished MKV recordings to MP4 in the background (if enabled).
    fn finish_files(&self, paths: Vec<PathBuf>) {
        let remux = self.config.output.remux_to_mp4;
        let sink = self.sink.clone();
        let _ = spawn("lanes-remux", Priority::Low, move || {
            let mut out = Vec::new();
            for p in paths {
                if !remux {
                    out.push(p);
                    continue;
                }
                let mp4 = naming::unique_path(p.parent().unwrap_or(std::path::Path::new(".")), &p.file_stem().unwrap_or_default().to_string_lossy(), "mp4");
                match crate::media::remux(&p, &mp4) {
                    Ok(()) => {
                        let _ = std::fs::remove_file(&p);
                        out.push(mp4);
                    }
                    Err(e) => {
                        log::error!("remux of {} failed: {e}; keeping the MKV", p.display());
                        out.push(p);
                    }
                }
            }
            sink.recording_saved(&out);
        });
    }

    fn toggle_pause(&mut self) {
        let Some(info) = &mut self.recording else { return };
        let Some(s) = &self.session else { return };
        match info.paused_since.take() {
            Some(since) => {
                info.paused_ms += since.elapsed().as_millis() as i64;
                let _ = s.router.commands.send(RouterCommand::Resume);
                log::info!("recording resumed");
            }
            None => {
                info.paused_since = Some(Instant::now());
                let _ = s.router.commands.send(RouterCommand::Pause);
                log::info!("recording paused");
            }
        }
    }

    // ------------------------------------------------------------- replay ---

    fn start_replay(&mut self) {
        if self.replay_on || !self.ensure_session() {
            return;
        }
        let cfg = self.replay_config();
        if let Some(s) = &self.session {
            let _ = s.router.commands.send(RouterCommand::SetReplay(Some(cfg)));
        }
        self.replay_on = true;
        log::info!("replay buffer started ({} s)", self.config.replay.length_secs);
    }

    fn stop_replay(&mut self) {
        if !self.replay_on {
            return;
        }
        if let Some(s) = &self.session {
            let _ = s.router.commands.send(RouterCommand::SetReplay(None));
        }
        self.replay_on = false;
        log::info!("replay buffer stopped");
        self.maybe_end_session();
    }

    fn save_clip(&mut self, preset_id: &str) {
        let now = Timestamp::now();
        if !self.replay_on {
            self.error(Component::Clip, "start the replay buffer to save clips", true);
            return;
        }
        let Some(preset) = self.config.presets.iter().find(|p| p.id == preset_id).cloned() else {
            self.error(Component::Clip, format!("unknown clip preset \"{preset_id}\""), true);
            return;
        };
        let folder = preset.folder.as_ref().map(PathBuf::from).unwrap_or_else(|| self.config.output.folder.clone());
        let template = preset.filename_template.clone().unwrap_or_else(|| self.config.output.filename_template.clone());
        let name = naming::render(&template, &naming::NameParts::now(&self.game_name(), &self.config.profile_name, &preset.name));
        let container = if self.config.output.remux_to_mp4 { Container::Mp4 } else { Container::Matroska };
        let files = self.file_plans(&name, container.extension(), &folder, preset.lanes.as_deref());
        let Some(s) = &self.session else { return };
        let sink = self.sink.clone();
        let quality = preset.quality;
        let mut encoder = self.config.encoder.clone();
        // Re-encode clips with the hardware encoder the session is using, not the CPU.
        if let Some(s) = &self.session {
            (encoder.backend, encoder.codec) = s.encoder_choice;
        }
        let preset_id = preset.id.clone();
        let done: router::ClipDone = Box::new(move |result| match result {
            Ok((paths, duration)) => {
                for p in &paths {
                    if quality != ClipQuality::Original {
                        reencode_clip(p, quality, &encoder);
                    }
                    sink.clip_saved(&ClipSaved { path: p.to_string_lossy().into_owned(), preset: preset_id.clone(), duration_secs: duration });
                }
            }
            Err(e) => sink.error(&EngineError { source: "clip".into(), message: format!("saving the clip failed: {e}"), recoverable: true }),
        });
        let _ = s.router.commands.send(RouterCommand::SaveClip {
            now,
            length: Duration::from_secs(u64::from(preset.length_secs)),
            files,
            container,
            done,
        });
        log::info!("saving clip \"{}\" ({} s)", preset.name, preset.length_secs);
    }

    // --------------------------------------------------------------- game ---

    fn set_game(&mut self, game: Option<DetectedGame>) {
        if game.as_ref().map(|g| (&g.exe, g.pid)) == self.game.as_ref().map(|g| (&g.exe, g.pid)) {
            self.game = game;
            return;
        }
        self.game = game;
        self.retarget_follow();
    }

    /// In follow-game mode, points stream 0 at the current game window, or
    /// the primary monitor when there is none. The encoder keeps running.
    fn retarget_follow(&mut self) {
        if self.config.video.mode != CaptureMode::FollowGame {
            return;
        }
        let Some(s) = &mut self.session else { return };
        let Some(stream) = s.video.first() else { return };
        let area = Rect::full(stream.plan.width, stream.plan.height);
        let target = match self.game.as_ref().and_then(crate::games::game_window) {
            Some(w) => Target::Window { hwnd: w.hwnd, name: self.game.as_ref().map(|g| g.name.clone()).unwrap_or_default() },
            None => match monitors::list().into_iter().next() {
                Some(m) => Target::Monitor { hmonitor: m.hmonitor, name: m.name },
                None => return,
            },
        };
        log::info!("now capturing {}", target.label());
        s.targets = vec![target.label().to_string()];
        stream.set_layers(vec![LayerPlan { target, area, max_fps: stream.plan.fps }]);
    }

    // ------------------------------------------------------------- events ---

    fn video_event(&mut self, ev: VideoEvent) {
        match ev {
            VideoEvent::SourceClosed { stream, target } => {
                log::info!("capture source closed: {}", target.label());
                if matches!(target, Target::Window { .. }) && stream == 0 {
                    // Game window gone: capture the monitor until a game shows up again.
                    self.game = None;
                    self.retarget_follow();
                } else {
                    self.error(Component::Capture, format!("{} is no longer available", target.label()), true);
                }
            }
            VideoEvent::Failed { stream, error } => {
                self.error(Component::Encoder, format!("video stream {stream} failed: {error}"), error.is_recoverable());
                self.restart_session();
            }
        }
        self.publish();
    }

    /// Recovers from a failed stream or lost GPU: rebuilds everything and
    /// continues recording into a new file.
    fn restart_session(&mut self) {
        let was_recording = self.recording.is_some();
        let was_replay = self.replay_on;
        if was_recording {
            self.stop_recording();
        }
        self.replay_on = false;
        self.session = None;
        std::thread::sleep(Duration::from_millis(500));
        if was_replay {
            self.start_replay();
        }
        if was_recording {
            self.start_recording();
        }
    }

    fn internal_event(&mut self, ev: Internal) {
        match ev {
            Internal::Device(DeviceEvent::DefaultChanged { render }) => {
                if let Some(a) = self.session.as_ref().and_then(|s| s.audio.as_ref()) {
                    log::info!("default {} device changed; following it", if render { "output" } else { "input" });
                    a.default_device_changed(render);
                }
            }
            Internal::Device(DeviceEvent::Changed) => {}
            Internal::AudioStatus(id, CaptureStatus::Waiting { reason }) => {
                self.error(Component::Audio, format!("source \"{id}\": {reason}"), true);
            }
            Internal::AudioStatus(id, CaptureStatus::Running { device }) => log::info!("audio source \"{id}\" capturing {device}"),
        }
    }

    // --------------------------------------------------------------- tick ---

    fn tick(&mut self) {
        if self.session.is_none() {
            return;
        }
        if self.last_audio_sync.elapsed() >= Duration::from_secs(3) {
            self.last_audio_sync = Instant::now();
            if let Some(a) = self.session.as_mut().and_then(|s| s.audio.as_mut()) {
                a.sync_sources();
            }
        }
        self.check_disk();
        self.emit_stats();
        self.publish();
    }

    fn check_disk(&mut self) {
        let (Some(guard), Some(_)) = (self.config.output.disk_guard.clone(), &self.recording) else { return };
        let Some(free) = crate::sysinfo::free_disk_bytes(&self.config.output.folder) else { return };
        if free >= guard.min_free_bytes {
            self.disk_warned = false;
            return;
        }
        if guard.stop {
            self.error(Component::Recorder, format!("disk space below {} GB: recording stopped", guard.min_free_bytes / 1_000_000_000), false);
            self.stop_recording();
        } else if !self.disk_warned {
            self.disk_warned = true;
            self.error(Component::Recorder, format!("disk space below {} GB", guard.min_free_bytes / 1_000_000_000), false);
        }
    }

    fn emit_stats(&mut self) {
        let cpu = self.cpu.sample();
        let folder = self.config.output.folder.clone();
        let Some(s) = &mut self.session else { return };
        let dt = s.last_stats_at.elapsed().as_secs_f64().max(0.001);
        s.last_stats_at = Instant::now();
        let (mut frames, mut dropped, mut dup, mut enc_ns) = (0, 0, 0, 0);
        for v in &s.video {
            frames += v.stats.frames.load(Ordering::Relaxed);
            dropped += v.stats.dropped.load(Ordering::Relaxed);
            dup += v.stats.duplicated.load(Ordering::Relaxed);
            enc_ns += v.stats.encode_ns.load(Ordering::Relaxed);
        }
        let df = frames.saturating_sub(s.last_frames);
        let dns = enc_ns.saturating_sub(s.last_encode_ns);
        s.last_frames = frames;
        s.last_encode_ns = enc_ns;
        let recorded = s.router.stats.recorded_bytes.load(Ordering::Relaxed);
        let disk_bps = recorded.saturating_sub(s.last_recorded) as f64 / dt;
        s.last_recorded = recorded;
        let target_fps = s.video.first().map(|v| v.plan.fps).unwrap_or(0);
        let streams = s.video.len().max(1) as f64;
        let encode_ms = if df == 0 { 0.0 } else { dns as f64 / df as f64 / 1e6 };
        let budget_ms = if target_fps == 0 { 1.0 } else { 1000.0 / f64::from(target_fps) };
        self.sink.stats(&Stats {
            fps: df as f64 / dt / streams,
            target_fps,
            dropped_frames: dropped,
            duplicated_frames: dup,
            encode_ms_avg: encode_ms,
            // Each stream encodes on its own thread with its own frame budget.
            encoder_load: encode_ms / budget_ms,
            encoder_queue: s.router.stats.queued.load(Ordering::Relaxed),
            buffer_secs: s.router.stats.replay_ms.load(Ordering::Relaxed) as f64 / 1000.0,
            buffer_bytes: s.router.stats.replay_bytes.load(Ordering::Relaxed),
            disk_write_bps: disk_bps,
            free_disk_bytes: crate::sysinfo::free_disk_bytes(&folder).unwrap_or(0),
            cpu_percent: cpu,
            memory_bytes: crate::sysinfo::memory_bytes(),
        });
    }

    fn publish(&self) {
        let recording = match &self.recording {
            None => RecordingState::Idle,
            Some(r) => {
                let paused_now = r.paused_since.map(|s| s.elapsed().as_millis() as i64).unwrap_or(0);
                RecordingState::Recording {
                    since_ms: r.start_unix_ms + r.paused_ms + paused_now,
                    paused: r.paused_since.is_some(),
                    files: r.files.iter().map(|p| p.to_string_lossy().into_owned()).collect(),
                }
            }
        };
        let replay = ReplayState {
            active: self.replay_on,
            buffered_secs: self.session.as_ref().map(|s| s.router.stats.replay_ms.load(Ordering::Relaxed) as f64 / 1000.0).unwrap_or(0.0),
            buffered_bytes: self.session.as_ref().map(|s| s.router.stats.replay_bytes.load(Ordering::Relaxed)).unwrap_or(0),
            capacity_secs: self.config.replay.length_secs,
        };
        let status = EngineStatus {
            recording,
            replay,
            active_profile: self.config.profile_id.clone(),
            game: self.game.clone().filter(|g| !g.exe.is_empty()),
            targets: self.session.as_ref().map(|s| s.targets.clone()).unwrap_or_default(),
            encoder: self.session.as_ref().map(|s| s.encoder_label.clone()),
        };
        *self.status.lock().unwrap_or_else(|e| e.into_inner()) = status.clone();
        self.sink.status(&status);
    }
}

/// Gives plans with the same title (two monitors of the same model) distinct
/// titles, "Name", "Name (2)"..., so their files and labels do not collide.
fn unique_titles(mut plans: Vec<VideoPlan>) -> Vec<VideoPlan> {
    for i in 1..plans.len() {
        let n = plans[..i].iter().filter(|p| p.title == plans[i].title || p.title.starts_with(&format!("{} (", plans[i].title))).count();
        if n > 0 {
            let title = format!("{} ({})", plans[i].title, n + 1);
            for l in &mut plans[i].layers {
                if let Target::Monitor { name, .. } = &mut l.target {
                    *name = title.clone();
                }
            }
            plans[i].title = title;
        }
    }
    plans
}

/// Composes monitors side by side according to their desktop positions,
/// scaled to fit `max_height` and the encoder's 4096-pixel width limit.
fn canvas_plan(selected: &[(monitors::MonitorInfo, Option<u32>, Option<u32>)], fps: u32, max_height: Option<u32>) -> VideoPlan {
    let min_x = selected.iter().map(|(m, ..)| m.x).min().unwrap_or(0);
    let min_y = selected.iter().map(|(m, ..)| m.y).min().unwrap_or(0);
    let max_x = selected.iter().map(|(m, ..)| m.x + m.width as i32).max().unwrap_or(1920);
    let max_y = selected.iter().map(|(m, ..)| m.y + m.height as i32).max().unwrap_or(1080);
    let (bw, bh) = ((max_x - min_x) as f64, (max_y - min_y) as f64);
    let mut scale = 1.0f64;
    if let Some(h) = max_height {
        scale = scale.min(f64::from(h) / bh);
    }
    scale = scale.min(4096.0 / bw).min(2304.0 / bh);
    // Sizes are even and at least 2 (NV12); positions are even and may be 0.
    let even = |v: f64| ((v.round() as u32).max(2)) & !1;
    let even_pos = |v: f64| (v.round().max(0.0) as u32) & !1;
    let layers = selected
        .iter()
        .map(|(m, cap, _)| LayerPlan {
            target: Target::Monitor { hmonitor: m.hmonitor, name: m.name.clone() },
            area: Rect {
                x: even_pos(f64::from(m.x - min_x) * scale),
                y: even_pos(f64::from(m.y - min_y) * scale),
                w: even(f64::from(m.width) * scale),
                h: even(f64::from(m.height) * scale),
            },
            max_fps: cap.map_or(fps, |c| c.clamp(1, fps)),
        })
        .collect();
    VideoPlan {
        index: 0,
        title: "Canvas".into(),
        layers,
        width: even(bw * scale),
        height: even(bh * scale),
        fps,
    }
}

/// Hardware H.264 tops out at 4096 pixels wide; scale down if needed.
fn clamp_size(plan: &mut VideoPlan, codec: VideoCodec) {
    let max_w = if codec == VideoCodec::H264 { 4096 } else { 8192 };
    if plan.width > max_w {
        let s = f64::from(max_w) / f64::from(plan.width);
        let size = |v: u32| ((f64::from(v) * s) as u32 & !1).max(2);
        let pos = |v: u32| (f64::from(v) * s) as u32 & !1;
        plan.width = size(plan.width);
        plan.height = size(plan.height);
        for l in &mut plan.layers {
            l.area = Rect { x: pos(l.area.x), y: pos(l.area.y), w: size(l.area.w), h: size(l.area.h) };
        }
    }
}

/// Re-encodes a saved clip at a lower bitrate in place (clip quality presets).
fn reencode_clip(path: &std::path::Path, quality: ClipQuality, enc: &EncoderSettings) {
    let factor = match quality {
        ClipQuality::Original => return,
        ClipQuality::High => 0.6,
        ClipQuality::Medium => 0.35,
        ClipQuality::Low => 0.18,
    };
    let kbps = ((f64::from(enc.bitrate_kbps) * factor) as u32).max(1500);
    let tmp = path.with_extension(format!("q.{}", path.extension().and_then(|e| e.to_str()).unwrap_or("mp4")));
    match crate::media::transcode(path, &tmp, enc.backend, enc.codec, kbps) {
        Ok(()) => {
            let _ = std::fs::rename(&tmp, path);
        }
        Err(e) => {
            log::error!("re-encoding {} failed: {e}; keeping the original quality", path.display());
            let _ = std::fs::remove_file(&tmp);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mon(name: &str, x: i32, y: i32, w: u32, h: u32) -> monitors::MonitorInfo {
        monitors::MonitorInfo {
            id: name.into(),
            name: name.into(),
            width: w,
            height: h,
            refresh_hz: 60,
            x,
            y,
            primary: x == 0,
            hmonitor: x as isize + 1,
            gdi_name: String::new(),
            adapter_luid: (0, 0),
        }
    }

    #[test]
    fn canvas_places_monitors_by_desktop_position() {
        let sel = vec![(mon("A", 0, 0, 1920, 1080), None, None), (mon("B", 1920, 0, 1280, 720), Some(30), None)];
        let p = canvas_plan(&sel, 60, None);
        assert_eq!((p.width, p.height), (3200, 1080));
        assert_eq!(p.layers[1].area, Rect { x: 1920, y: 0, w: 1280, h: 720 });
        assert_eq!(p.layers[1].max_fps, 30);
        assert_eq!(p.layers[0].max_fps, 60);
    }

    #[test]
    fn canvas_respects_width_limit_and_max_height() {
        let sel = vec![(mon("A", 0, 0, 3840, 2160), None, None), (mon("B", 3840, 0, 3840, 2160), None, None)];
        let p = canvas_plan(&sel, 60, Some(1080));
        assert!(p.width <= 4096 && p.height <= 1080, "{}x{}", p.width, p.height);
        assert!(p.width.is_multiple_of(2) && p.height.is_multiple_of(2));
    }

    #[test]
    fn duplicate_monitor_names_get_suffixes() {
        let plan = |t: &str| VideoPlan { index: 0, title: t.into(), layers: vec![], width: 2, height: 2, fps: 60 };
        let out = unique_titles(vec![plan("AOC"), plan("Acer"), plan("AOC"), plan("AOC")]);
        let titles: Vec<_> = out.iter().map(|p| p.title.as_str()).collect();
        assert_eq!(titles, ["AOC", "Acer", "AOC (2)", "AOC (3)"]);
    }

    #[test]
    fn clamp_size_scales_wide_h264() {
        let mut plan = VideoPlan { index: 0, title: String::new(), layers: vec![], width: 5120, height: 1440, fps: 60 };
        clamp_size(&mut plan, VideoCodec::H264);
        assert_eq!(plan.width, 4096);
        let mut plan = VideoPlan { index: 0, title: String::new(), layers: vec![], width: 5120, height: 1440, fps: 60 };
        clamp_size(&mut plan, VideoCodec::Hevc);
        assert_eq!(plan.width, 5120);
    }

    #[test]
    fn recording_state_serializes_like_the_ui_expects() {
        let s = serde_json::to_value(RecordingState::Recording { since_ms: 5, paused: true, files: vec!["a".into()] }).unwrap();
        assert_eq!(s, serde_json::json!({ "state": "recording", "sinceMs": 5, "paused": true, "files": ["a"] }));
        assert_eq!(serde_json::to_value(RecordingState::Idle).unwrap(), serde_json::json!({ "state": "idle" }));
    }
}
