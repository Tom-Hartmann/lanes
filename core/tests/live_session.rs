//! End-to-end tests against the real machine: GPU, monitor, audio devices and
//! hardware encoder. They capture the primary monitor and system audio for a
//! few seconds into a temp folder. Ignored by default; run with
//! `cargo test -p lanes-core --test live_session -- --ignored --nocapture`.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use lanes_core::audio::mixer::MeterFrame;
use lanes_core::config::{AudioLayout, CaptureMode, ClipPreset, ClipQuality, EncoderSettings, ReplaySettings, VideoSettings};
use lanes_core::engine::{ClipSaved, Engine, EngineConfig, EngineError, EngineStatus, EventSink, OutputConfig, Stats};

#[derive(Default)]
struct Collect {
    clips: Mutex<Vec<ClipSaved>>,
    recordings: Mutex<Vec<PathBuf>>,
    errors: Mutex<Vec<EngineError>>,
    stats: Mutex<Vec<Stats>>,
    meters: Mutex<u32>,
}

impl EventSink for Collect {
    fn status(&self, _: &EngineStatus) {}
    fn meters(&self, _: &MeterFrame) {
        *self.meters.lock().unwrap() += 1;
    }
    fn stats(&self, s: &Stats) {
        self.stats.lock().unwrap().push(s.clone());
    }
    fn clip_saved(&self, c: &ClipSaved) {
        self.clips.lock().unwrap().push(c.clone());
    }
    fn recording_saved(&self, paths: &[PathBuf]) {
        self.recordings.lock().unwrap().extend_from_slice(paths);
    }
    fn error(&self, e: &EngineError) {
        eprintln!("engine error: {e:?}");
        self.errors.lock().unwrap().push(e.clone());
    }
}

fn config(dir: &Path) -> EngineConfig {
    EngineConfig {
        profile_id: "test".into(),
        profile_name: "Test".into(),
        encoder: EncoderSettings { bitrate_kbps: 8000, max_bitrate_kbps: 12000, ..Default::default() },
        video: VideoSettings { mode: CaptureMode::Monitors, fps: 60, output_height: Some(720), ..Default::default() },
        audio: AudioLayout::default(),
        replay: ReplaySettings { length_secs: 30, ..Default::default() },
        presets: vec![ClipPreset {
            id: "p".into(),
            name: "Test".into(),
            length_secs: 2,
            lanes: None,
            quality: ClipQuality::Original,
            folder: None,
            filename_template: None,
        }],
        output: OutputConfig { folder: dir.to_path_buf(), filename_template: "{game}_{time}".into(), remux_to_mp4: true, disk_guard: None },
        temp_dir: dir.join("tmp"),
    }
}

fn wait_for<T>(what: &str, mut f: impl FnMut() -> Option<T>) -> T {
    for _ in 0..200 {
        if let Some(v) = f() {
            return v;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    panic!("timed out waiting for {what}");
}

#[test]
#[ignore = "captures the real screen and audio"]
fn record_then_clip() {
    let dir = std::env::temp_dir().join(format!("lanes-live-{}", std::process::id()));
    let sink = Arc::new(Collect::default());
    let engine = Engine::start(config(&dir), sink.clone()).unwrap();

    // Recording: 4 seconds -> MP4 after remux.
    engine.start_recording();
    std::thread::sleep(Duration::from_secs(4));
    eprintln!("encoder: {:?}", engine.status().encoder);
    engine.stop_recording();
    let recordings = wait_for("recording", || {
        let r = sink.recordings.lock().unwrap();
        (!r.is_empty()).then(|| r.clone())
    });
    let rec = &recordings[0];
    let secs = lanes_core::media::duration(rec).expect("readable recording");
    eprintln!("recording {} -> {secs:.2} s, {} bytes", rec.display(), std::fs::metadata(rec).unwrap().len());
    assert_eq!(rec.extension().unwrap(), "mp4");
    assert!((3.0..6.0).contains(&secs), "duration {secs}");

    // Replay buffer + clip of the last 2 seconds.
    engine.start_replay();
    std::thread::sleep(Duration::from_secs(4));
    engine.save_clip("p");
    let clip = wait_for("clip", || sink.clips.lock().unwrap().first().cloned());
    let clip_secs = lanes_core::media::duration(Path::new(&clip.path)).expect("readable clip");
    eprintln!("clip {} -> {clip_secs:.2} s", clip.path);
    assert!((1.5..4.5).contains(&clip_secs), "clip duration {clip_secs}");
    let thumb = lanes_core::media::thumbnail(Path::new(&clip.path), 160).unwrap();
    assert!(thumb.starts_with("data:image/bmp;base64,"));

    // Trim + remux round trip on the recording.
    let trimmed = dir.join("trimmed.mp4");
    lanes_core::media::trim(rec, &trimmed, 1.0, 2.5).unwrap();
    let t = lanes_core::media::duration(&trimmed).unwrap();
    eprintln!("trimmed -> {t:.2} s");
    assert!(t > 0.5 && t < 3.0, "trimmed {t}");

    let stats = sink.stats.lock().unwrap().clone();
    if let Some(s) = stats.last() {
        eprintln!(
            "stats: {:.1}/{} fps, dropped {}, dup {}, encode {:.2} ms, load {:.0}%, cpu {:.1}%, mem {} MB",
            s.fps, s.target_fps, s.dropped_frames, s.duplicated_frames, s.encode_ms_avg, s.encoder_load * 100.0, s.cpu_percent,
            s.memory_bytes / 1_000_000
        );
    }
    eprintln!("meter frames: {}", sink.meters.lock().unwrap());
    engine.stop_replay();
    drop(engine);
    assert!(sink.errors.lock().unwrap().iter().all(|e| e.recoverable), "fatal engine errors");
    if std::env::var_os("LANES_KEEP").is_none() {
        let _ = std::fs::remove_dir_all(dir);
    }
}

#[test]
#[ignore = "probes the real GPU"]
fn probe_encoders() {
    for a in lanes_core::gpu::adapters() {
        eprintln!("adapter: {} {:?} {} MB", a.name, a.vendor, a.dedicated_vram_mb);
    }
    let gpu = lanes_core::gpu::Gpu::new(None).unwrap();
    if std::env::var_os("LANES_AV_DEBUG").is_some() {
        unsafe { lanes_core::av::ffi::av_log_set_level(lanes_core::av::ffi::AV_LOG_DEBUG) };
    }
    eprintln!("capture gpu: {} {:?}", gpu.adapter.name, gpu.adapter.vendor);
    use lanes_core::config::{EncoderBackend, EncoderSettings, VideoCodec};
    use lanes_core::encode::video::{VideoEncoder, VideoEncoderConfig};
    for backend in [EncoderBackend::Nvenc, EncoderBackend::Amf, EncoderBackend::Qsv, EncoderBackend::X264] {
        for codec in [VideoCodec::H264, VideoCodec::Hevc, VideoCodec::Av1] {
            let cfg = VideoEncoderConfig { backend, codec, width: 1280, height: 720, fps: 30, settings: EncoderSettings::default() };
            match VideoEncoder::open(&gpu, &cfg) {
                Ok(e) => eprintln!("{backend:?} {codec:?}: OK zero_copy={}", e.is_zero_copy()),
                Err(e) => eprintln!("{backend:?} {codec:?}: {e}"),
            }
        }
    }
}

#[test]
#[ignore = "needs LANES_REMUX_SRC"]
fn remux_file() {
    let Some(src) = std::env::var_os("LANES_REMUX_SRC") else { return };
    let src = PathBuf::from(src);
    let dst = std::env::temp_dir().join("lanes-remux-test.mp4");
    let r = lanes_core::media::remux(&src, &dst);
    eprintln!("remux result: {r:?}");
    eprintln!("duration: {:?}", lanes_core::media::duration(&dst));
    let _ = std::fs::remove_file(&dst);
}

#[test]
#[ignore = "inspects the real desktop"]
fn detect_game_now() {
    let fg = lanes_core::capture::windows_list::foreground();
    eprintln!("foreground: {:?}", fg.map(|w| (w.exe, w.fullscreen)));
    eprintln!("detected: {:?}", lanes_core::games::detect(&[], true).map(|g| (g.name, g.exe, g.pid, g.fullscreen)));
}

#[test]
#[ignore = "needs LANES_SHARE_SRC"]
fn share_file() {
    let Some(src) = std::env::var_os("LANES_SHARE_SRC") else { return };
    let src = PathBuf::from(src);
    for (name, max) in [("limit", Some(10 * 1024 * 1024u64)), ("copy", None)] {
        let dst = std::env::temp_dir().join(format!("lanes-share-{name}.mp4"));
        let opts = lanes_core::share::ShareOptions { audio_tracks: None, gains_db: Vec::new(), max_bytes: max, backend: lanes_core::config::EncoderBackend::Nvenc };
        let t = std::time::Instant::now();
        let r = lanes_core::share::export(&src, &dst, &opts);
        eprintln!("{name}: {r:?} in {:.1} s", t.elapsed().as_secs_f64());
    }
}
