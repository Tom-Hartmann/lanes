use std::{path::Path, sync::atomic::AtomicBool};
use lanes_core::{ffmpeg::{self, Control}, files::WorkDir, studio::{self, Preset}};

fn fixture(path: &Path, audio: bool) {
    let mut cmd = ffmpeg::command().unwrap();
    cmd.args(["-f", "lavfi", "-i", if audio { "color=red:s=320x240:r=30:d=2" } else { "color=blue:s=640x360:r=25:d=2" }]);
    if audio { cmd.args(["-f", "lavfi", "-i", "sine=frequency=440:duration=2", "-c:a", "aac"]); }
    cmd.args(["-c:v", "libx264", "-preset", "ultrafast"]).arg(path);
    ffmpeg::run(cmd, None, 2.0).unwrap();
}

#[test]
fn compilation_normalizes_mixed_sources_and_crossfades() {
    let work = WorkDir::new(&std::env::temp_dir()).unwrap();
    let a = work.path().join("a.mkv"); let b = work.path().join("b.mkv");
    fixture(&a, true); fixture(&b, false);
    let out = work.path().join("joined.mp4");
    let preset = Preset { width: 320, height: 180, crossfade: 0.25, ..Preset::default() };
    let progress = std::sync::Mutex::new(Vec::new());
    studio::compile(&[a.clone(), b.clone()], &out, &preset, &Control { cancel: &AtomicBool::new(false), progress: &|p| progress.lock().unwrap().push(p) }).unwrap();
    let duration = lanes_core::media::duration(&out).unwrap();
    assert!((duration - 3.75).abs() < 0.15, "duration {duration}");
    assert_eq!(lanes_core::media::audio_track_titles(&out).unwrap().len(), 1);
    assert!(progress.lock().unwrap().windows(2).all(|w| w[0] <= w[1]));
    assert!((lanes_core::media::frame_duration(&out).unwrap() - 1.0/30.0).abs() < 0.001);
    let mkv = work.path().join("joined.mkv");
    studio::compile(&[a.clone(), b.clone()], &mkv, &Preset { format: "mkv".into(), width: 320, height: 180, ..Preset::default() }, &Control { cancel: &AtomicBool::new(false), progress: &|_| {} }).unwrap();
    assert!((lanes_core::media::duration(&mkv).unwrap()-4.0).abs()<0.2);
    let limited = work.path().join("limited.mp4");
    studio::compile(std::slice::from_ref(&a), &limited, &Preset { max_mb: 1, width: 320, height: 180, ..Preset::default() }, &Control { cancel: &AtomicBool::new(false), progress: &|_| {} }).unwrap();
    assert!(std::fs::metadata(&limited).unwrap().len() <= 1_000_000);
    let cancelled = work.path().join("cancelled.mp4");
    assert!(studio::compile(&[a,b], &cancelled, &preset, &Control { cancel: &AtomicBool::new(true), progress: &|_| {} }).is_err());
    assert!(!cancelled.exists());
}

#[test]
fn recovery_salvages_truncated_mkv_without_touching_original() {
    let work = WorkDir::new(&std::env::temp_dir()).unwrap();
    let source = work.path().join("source.mkv"); fixture(&source, true);
    let mut bytes = std::fs::read(&source).unwrap(); bytes.truncate(bytes.len() * 3 / 4);
    std::fs::write(&source, &bytes).unwrap();
    let out = work.path().join("recovered.mkv");
    studio::recover(&source, &out, &Control { cancel: &AtomicBool::new(false), progress: &|_| {} }).unwrap();
    assert!(lanes_core::media::duration(&out).unwrap() > 0.1);
    assert_eq!(std::fs::read(source).unwrap(), bytes);
}

#[test]
fn rejects_invalid_presets() {
    assert!(Preset { width: 1919, ..Preset::default() }.validate().is_err());
    assert!(Preset { crossfade: f64::NAN, ..Preset::default() }.validate().is_err());
}

#[test]
#[ignore = "requires optional speech engine and LANES_SPEECH_FIXTURE"]
fn local_captions_transcribe_real_speech() {
    let speech = std::path::PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap()).join("Lanes/speech");
    let source = std::path::PathBuf::from(std::env::var_os("LANES_SPEECH_FIXTURE").unwrap());
    let work = WorkDir::new(&std::env::temp_dir()).unwrap();
    let out = work.path().join("captions.srt");
    lanes_core::captions::transcribe(&source, &out, &speech.join("whisper-cli.exe"), &speech.join("ggml-tiny.bin"),
        &Control { cancel: &AtomicBool::new(false), progress: &|_| {} }).unwrap();
    let text = std::fs::read_to_string(out).unwrap();
    assert!(text.contains("-->")); assert!(text.to_lowercase().contains("country"), "{text}");
}
