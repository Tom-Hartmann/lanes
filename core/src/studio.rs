//! Consistent media compilation: bounded streaming workers, cancellable jobs.
use std::path::{Path, PathBuf};
use serde::{Deserialize, Serialize};
use crate::{Error, Result, ffmpeg::{self, Control}};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Preset { pub format: String, pub max_mb: u32, pub width: u32, pub height: u32, pub fps: u32, pub crf: u32, pub crossfade: f64 }
impl Default for Preset {
    fn default() -> Self { Self { format: "mp4".into(), max_mb: 0, width: 1280, height: 720, fps: 30, crf: 20, crossfade: 0.0 } }
}
impl Preset {
    pub fn validate(&self) -> Result<()> {
        if !["mp4", "mkv"].contains(&self.format.as_str()) || self.max_mb > 10000 || (self.format != "mp4" && self.max_mb > 0) || !(16..=3840).contains(&self.width) || !(16..=2160).contains(&self.height)
            || !self.width.is_multiple_of(2) || !self.height.is_multiple_of(2)
            || ![24, 25, 30, 50, 60].contains(&self.fps) || !(16..=32).contains(&self.crf)
            || !self.crossfade.is_finite() || !(0.0..=2.0).contains(&self.crossfade) {
            return Err(Error::Invalid("Invalid export preset".into()));
        }
        Ok(())
    }
}

pub fn compile(paths: &[PathBuf], dst: &Path, preset: &Preset, control: &Control<'_>) -> Result<()> {
    preset.validate()?;
    if paths.is_empty() || paths.len() > 16 { return Err(Error::Invalid("Choose 1 to 16 clips".into())); }
    let work = crate::files::WorkDir::new(dst.parent().unwrap_or(Path::new(".")))?;
    let mut durations = Vec::new();
    for p in paths {
        let d = crate::media::duration(p).ok_or_else(|| Error::Invalid("Cannot read source duration".into()))?;
        if !d.is_finite() || d <= preset.crossfade * 2.0 || d <= 0.05 { return Err(Error::Invalid("Clip is too short for the selected transition".into())); }
        durations.push(d);
    }
    let encode = |cmd: &mut std::process::Command| {
        cmd.args(["-c:v", "libx264", "-preset", "fast", "-crf", &preset.crf.to_string(), "-pix_fmt", "yuv420p", "-c:a", "aac", "-b:a", "160k", "-ar", "48000", "-ac", "2", "-threads", "2"]);
    };
    for (i, p) in paths.iter().enumerate() {
        let mut cmd = ffmpeg::command()?;
        cmd.arg("-i").arg(p);
        if crate::media::audio_track_titles(p)?.is_empty() { cmd.args(["-f", "lavfi", "-i", "anullsrc=r=48000:cl=stereo", "-map", "1:a:0"]); }
        else { cmd.args(["-map", "0:a:0"]); }
        cmd.args(["-map", "0:v:0", "-t", &durations[i].to_string(), "-vf",
            &format!("scale={}:{}:force_original_aspect_ratio=decrease,pad={}:{}:(ow-iw)/2:(oh-ih)/2,setsar=1,fps={},settb=1/{}", preset.width, preset.height, preset.width, preset.height, preset.fps, preset.fps), "-af", "apad"]);
        encode(&mut cmd);
        cmd.arg(work.path().join(format!("part-{i}.mp4")));
        let progress = |f| (control.progress)(0.65 * (i as f64 + f) / paths.len() as f64);
        ffmpeg::run(cmd, Some(&Control { cancel: control.cancel, progress: &progress }), durations[i])?;
    }
    let mut previous = work.path().join("part-0.mp4");
    let mut elapsed = durations[0];
    if paths.len() > 1 {
        let joined = work.path().join("joined.mp4");
        let mut cmd = ffmpeg::command()?;
        if preset.crossfade == 0.0 {
            let manifest = work.path().join("parts.txt");
            std::fs::write(&manifest, (0..paths.len()).map(|i| format!("file 'part-{i}.mp4'\n")).collect::<String>())?;
            cmd.args(["-f", "concat", "-safe", "1", "-i"]).arg(manifest).args(["-map", "0", "-c", "copy"]);
            elapsed = durations.iter().sum();
        } else {
            let mut graph = Vec::new();
            for i in 0..paths.len() {
                cmd.arg("-i").arg(work.path().join(format!("part-{i}.mp4")));
                graph.push(format!("[{i}:v]settb=AVTB,setpts=PTS-STARTPTS[v{i}];[{i}:a]asetpts=PTS-STARTPTS[a{i}]"));
            }
            let mut video = "v0".to_owned();
            let mut audio = "a0".to_owned();
            for (i, duration) in durations.iter().enumerate().skip(1) {
                graph.push(format!("[{video}][v{i}]xfade=transition=fade:duration={}:offset={}[vx{i}];[{audio}][a{i}]acrossfade=d={}:c1=tri:c2=tri[ax{i}]", preset.crossfade, elapsed - preset.crossfade, preset.crossfade));
                elapsed += duration - preset.crossfade;
                video = format!("vx{i}"); audio = format!("ax{i}");
            }
            cmd.args(["-filter_complex", &graph.join(";"), "-map", &format!("[{video}]"), "-map", &format!("[{audio}]")]);
            encode(&mut cmd);
        }
        cmd.args(["-t", &elapsed.to_string(), "-movflags", "+faststart"]).arg(&joined);
        let progress = |f| (control.progress)(0.65 + 0.24 * f);
        ffmpeg::run(cmd, Some(&Control { cancel: control.cancel, progress: &progress }), elapsed)?;
        previous = joined;
    }
    if control.cancel.load(std::sync::atomic::Ordering::Relaxed) { return Err(Error::Invalid("Export cancelled".into())); }
    if preset.max_mb > 0 {
        let limited = work.path().join("limited.mp4");
        let progress = |_| (control.progress)(0.95);
        crate::share::export_control(&previous, &limited, &crate::share::ShareOptions {
            mix: crate::share::MixSpec::default(), range: None, max_bytes: Some(u64::from(preset.max_mb) * 1_000_000), backend: crate::config::EncoderBackend::X264,
        }, Some(&Control { cancel: control.cancel, progress: &progress }))?;
        previous = limited;
    }
    if preset.format == "mkv" {
        let container = work.path().join("finished.mkv");
        let mut cmd = ffmpeg::command()?;
        cmd.arg("-i").arg(&previous).args(["-map", "0", "-c", "copy"]).arg(&container);
        let progress = |_| (control.progress)(0.98);
        ffmpeg::run(cmd, Some(&Control { cancel: control.cancel, progress: &progress }), elapsed)?;
        previous = container;
    }
    if control.cancel.load(std::sync::atomic::Ordering::Relaxed) { return Err(Error::Invalid("Export cancelled".into())); }
    std::fs::rename(previous, dst)?;
    (control.progress)(1.0);
    Ok(())
}

/// Salvage readable packets into a fresh container; never modifies the source.
pub fn recover(src: &Path, dst: &Path, control: &Control<'_>) -> Result<()> {
    let work = crate::files::WorkDir::new(dst.parent().unwrap_or(Path::new(".")))?;
    let out = work.path().join("recovered.mkv");
    let mut cmd = ffmpeg::command()?;
    cmd.args(["-fflags", "+genpts+discardcorrupt", "-err_detect", "ignore_err", "-i"]).arg(src)
        .args(["-map", "0:v:0", "-map", "0:a?", "-c", "copy"]).arg(&out);
    ffmpeg::run(cmd, Some(control), crate::media::duration(src).unwrap_or(1.0))?;
    if crate::media::duration(&out).is_none_or(|d| d <= 0.0) { return Err(Error::Invalid("No playable media could be recovered".into())); }
    std::fs::rename(out, dst)?;
    Ok(())
}
