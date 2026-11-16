//! Synthetic media only: no desktop capture, GPU, or user files.
use std::{path::Path, sync::atomic::{AtomicBool, Ordering}};
use lanes_core::{edit::{Crop, EditSpec}, ffmpeg::{self, Control}, files::WorkDir, share::{MixSpec, ShareOptions}};

fn fixture(path: &Path) {
    let mut cmd = ffmpeg::command().unwrap();
    cmd.args(["-f", "lavfi", "-i", "color=red:s=320x180:r=30:d=2",
        "-f", "lavfi", "-i", "color=green:s=320x180:r=30:d=2",
        "-f", "lavfi", "-i", "color=blue:s=320x180:r=30:d=2",
        "-f", "lavfi", "-i", "sine=frequency=440:duration=6:sample_rate=48000",
        "-f", "lavfi", "-i", "sine=frequency=880:duration=6:sample_rate=48000",
        "-filter_complex", "[0:v][1:v][2:v]concat=n=3:v=1:a=0[v]",
        "-map", "[v]", "-map", "3:a", "-map", "4:a", "-c:v", "libx264", "-preset", "ultrafast",
        "-c:a", "aac", "-metadata:s:a:0", "title=Game", "-metadata:s:a:1", "title=Voice"])
        .arg(path);
    ffmpeg::run(cmd, None, 6.0).unwrap();
}

fn pixels(path: &Path, at: f64) -> Vec<u8> {
    let mut cmd = ffmpeg::command().unwrap();
    cmd.args(["-ss", &at.to_string(), "-i"]).arg(path)
        .args(["-frames:v", "1", "-an", "-pix_fmt", "rgb24", "-f", "rawvideo", "pipe:1"]);
    let out = cmd.output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    out.stdout
}

#[test]
fn crops_joins_kept_sections_and_preserves_audio() {
    let work = WorkDir::new(&std::env::temp_dir()).unwrap();
    let src = work.path().join("source.mkv");
    fixture(&src);
    let dst = work.path().join("edited.mp4");
    let edit = EditSpec { segments: vec![(0.0, 1.5), (4.5, 6.0)], crop: Crop { left: 0.1, right: 0.1, top: 0.1, bottom: 0.1 } };
    let cancel = AtomicBool::new(false);
    let progress = std::sync::Mutex::new(Vec::new());
    lanes_core::edit::render(&src, &dst, &edit, Some(&MixSpec::default()), &Control {
        cancel: &cancel, progress: &|p| progress.lock().unwrap().push(p),
    }).unwrap();
    let duration = lanes_core::media::duration(&dst).unwrap();
    assert!((duration - 3.0).abs() < 0.15, "duration {duration}");
    let first = pixels(&dst, 0.5);
    let last = pixels(&dst, 2.0);
    assert_eq!(first.len(), 256 * 144 * 3, "crop dimensions");
    assert!(first[0] > 200 && first[1] < 30 && first[2] < 30, "first kept section must be red");
    assert!(last[2] > 200 && last[0] < 30 && last[1] < 30, "second must be blue, never the removed green section");
    assert_eq!(lanes_core::media::audio_track_titles(&dst).unwrap(), ["Mix (all lanes)", "Game", "Voice"]);
    assert_eq!(progress.lock().unwrap().last(), Some(&1.0));
    assert!(progress.lock().unwrap().windows(2).all(|w| w[0] <= w[1]));

    let share = work.path().join("share.mp4");
    let opts = ShareOptions { mix: MixSpec::default(), range: None, max_bytes: Some(512 * 1024), backend: lanes_core::config::EncoderBackend::X264 };
    let result = lanes_core::share::export(&dst, &share, &opts).unwrap();
    assert!(result.size_bytes <= opts.max_bytes.unwrap());
    assert_eq!(lanes_core::media::audio_track_titles(&share).unwrap().len(), 1);
    let impossible = work.path().join("too-small.mp4");
    assert!(lanes_core::share::export(&dst, &impossible, &ShareOptions { max_bytes: Some(1024), ..opts }).is_err());
    assert!(!impossible.exists());
}

#[test]
fn cancellation_cleans_partial_exports_and_keeps_source() {
    let work = WorkDir::new(&std::env::temp_dir()).unwrap();
    let src = work.path().join("source.mkv");
    fixture(&src);
    let original = std::fs::read(&src).unwrap();
    let dst = work.path().join("cancelled.mp4");
    let cancel = AtomicBool::new(false);
    let result = lanes_core::edit::render(&src, &dst,
        &EditSpec { segments: vec![(0.0, 6.0)], crop: Crop::default() }, None,
        &Control { cancel: &cancel, progress: &|_| cancel.store(true, Ordering::Relaxed) });
    assert!(result.unwrap_err().to_string().contains("cancelled"));
    assert!(!dst.exists());
    assert_eq!(std::fs::read(&src).unwrap(), original);
    assert_eq!(std::fs::read_dir(work.path()).unwrap().count(), 1, "no leftover work directories");
}

#[test]
fn share_reencodes_to_an_enforced_size_limit() {
    let work = WorkDir::new(&std::env::temp_dir()).unwrap();
    let src = work.path().join("large.mp4");
    let mut cmd = ffmpeg::command().unwrap();
    cmd.args(["-f", "lavfi", "-i", "testsrc2=size=640x360:rate=30:duration=6",
        "-f", "lavfi", "-i", "sine=frequency=440:sample_rate=48000:duration=6",
        "-c:v", "libx264", "-preset", "ultrafast", "-crf", "0", "-c:a", "aac"])
        .arg(&src);
    ffmpeg::run(cmd, None, 6.0).unwrap();
    let limit = 512 * 1024;
    assert!(std::fs::metadata(&src).unwrap().len() > limit);
    let out = work.path().join("small.mp4");
    let result = lanes_core::share::export(&src, &out, &ShareOptions {
        mix: MixSpec::default(), range: None, max_bytes: Some(limit), backend: lanes_core::config::EncoderBackend::X264,
    }).unwrap();
    assert!(result.reencoded);
    assert!(result.size_bytes <= limit);
    assert!((lanes_core::media::duration(&out).unwrap() - 6.0).abs() < 0.15, "must not fit by truncating the clip");
}
