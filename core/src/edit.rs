//! Precise, non-destructive clip edits. Each kept segment is streamed to a
//! private work directory, then joined without loading media into memory.
use std::path::Path;
use serde::{Deserialize, Serialize};
use crate::{Error, Result, ffmpeg::Control, share::MixSpec};

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(default)]
pub struct Crop {
    pub left: f64,
    pub right: f64,
    pub top: f64,
    pub bottom: f64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct EditSpec {
    #[serde(default)]
    pub audio: AudioEffects,
    pub segments: Vec<(f64, f64)>,
    #[serde(default)]
    pub crop: Crop,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AudioEffects {
    pub fade_in: f64,
    pub crossfade: f64,
    pub fade_out: f64,
    pub normalize: bool,
    pub denoise: bool,
}

impl EditSpec {
    pub fn validate(&self, duration: f64) -> Result<()> {
        if !duration.is_finite() || duration <= 0.0 || self.segments.is_empty() || self.segments.len() > 32 {
            return Err(Error::Invalid("Keep between 1 and 32 segments from a valid clip".into()));
        }
        if [self.audio.fade_in, self.audio.fade_out].iter().any(|x| !x.is_finite() || *x < 0.0 || *x > self.duration()) {
            return Err(Error::Invalid("Fades must fit within the edited duration".into()));
        }
        if !self.audio.crossfade.is_finite() || !(0.0..=2.0).contains(&self.audio.crossfade) { return Err(Error::Invalid("Cut crossfade must be between 0 and 2 seconds".into())); }
        for &(start, end) in &self.segments {
            if !start.is_finite() || !end.is_finite() || start < 0.0 || end - start < 0.05 || end - start <= self.audio.crossfade * 2.0 || end > duration + 0.05 {
                return Err(Error::Invalid("Segments must be inside the clip, and at least 0.05 seconds long".into()));
            }

        }
        let c = self.crop;
        if [c.left, c.right, c.top, c.bottom].iter().any(|x| !x.is_finite() || *x < 0.0)
            || c.left + c.right >= 0.95 || c.top + c.bottom >= 0.95 {
            return Err(Error::Invalid("The crop must keep at least 5% of the frame on each axis".into()));
        }
        Ok(())
    }

    pub fn duration(&self) -> f64 { self.segments.iter().map(|(a, b)| b - a).sum::<f64>() - self.audio.crossfade * self.segments.len().saturating_sub(1) as f64 }

    pub fn markers(&self, markers: &[f64]) -> Vec<f64> {
        let mut offset = 0.0;
        let mut out = Vec::new();
        for &(start, end) in &self.segments {
            out.extend(markers.iter().filter(|&&m| m >= start && m < end).map(|m| offset + m - start));
            offset += end - start - self.audio.crossfade;
        }
        out
    }
}

pub fn render(src: &Path, dst: &Path, edit: &EditSpec, mix: Option<&MixSpec>, control: &Control<'_>) -> Result<()> {
    let duration = crate::media::duration(src).ok_or_else(|| Error::Invalid("Cannot determine clip duration".into()))?;
    edit.validate(duration)?;
    let input = crate::media::Input::open(src)?;
    let video = input.video_stream().ok_or_else(|| Error::Invalid("This file has no video".into()))?;
    // SAFETY: input owns the stream and codec parameters throughout this read.
    let (width, height) = unsafe { let p = (*input.streams()[video]).codecpar; ((*p).width, (*p).height) };
    drop(input);
    let c = edit.crop;
    let x = ((width as f64 * c.left) as i32 / 2) * 2;
    let y = ((height as f64 * c.top) as i32 / 2) * 2;
    let w = ((width as f64 * (1.0 - c.left - c.right)) as i32 / 2) * 2;
    let h = ((height as f64 * (1.0 - c.top - c.bottom)) as i32 / 2) * 2;
    if w < 16 || h < 16 { return Err(Error::Invalid("Crop must leave at least 16 × 16 pixels".into())); }
    let work = crate::files::WorkDir::new(dst.parent().unwrap_or(Path::new(".")))?;
    let total = edit.duration();
    let source_total: f64 = edit.segments.iter().map(|(a,b)| b-a).sum();
    let mut elapsed = 0.0;
    let mut list = String::new();
    for (i, &(start, end)) in edit.segments.iter().enumerate() {
        let name = format!("segment-{i}.mp4");
        let mut cmd = crate::ffmpeg::command()?;
        cmd.args(["-ss", &format!("{start:.6}")]).arg("-i").arg(src)
            .args(["-t", &format!("{:.6}", end - start), "-map", "0:v:0", "-vf", &format!("crop={w}:{h}:{x}:{y},setsar=1"),
                "-c:v", "libx264", "-preset", "fast", "-crf", "20", "-pix_fmt", "yuv420p", "-threads", "2"]);
        crate::share::audio_args(&mut cmd, src, mix, crate::share::OriginalAudio::Encode)?;
        cmd.arg(work.path().join(&name));
        let report = |fraction: f64| (control.progress)(0.95 * (elapsed + fraction * (end - start)) / source_total);
        crate::ffmpeg::run(cmd, Some(&Control { cancel: control.cancel, progress: &report }), end - start)?;
        list.push_str(&format!("file '{name}'\n"));
        elapsed += end - start;
    }
    let manifest = work.path().join("segments.txt");
    std::fs::write(&manifest, list)?;
    let out = work.path().join("finished.mp4");
    let mut cmd = crate::ffmpeg::command()?;
    let mut filters = Vec::new();
    if edit.audio.denoise { filters.push("afftdn=nf=-25".to_owned()); }
    if edit.audio.normalize { filters.push("loudnorm=I=-16:TP=-1.5:LRA=11".to_owned()); }
    if edit.audio.fade_in > 0.0 { filters.push(format!("afade=t=in:d={}", edit.audio.fade_in)); }
    if edit.audio.fade_out > 0.0 { filters.push(format!("afade=t=out:st={}:d={}", total - edit.audio.fade_out, edit.audio.fade_out)); }
    if edit.audio.crossfade > 0.0 && edit.segments.len() > 1 {
        let titles = crate::media::audio_track_titles(&work.path().join("segment-0.mp4"))?;
        let mut graph = Vec::new();
        for i in 0..edit.segments.len() {
            cmd.arg("-i").arg(work.path().join(format!("segment-{i}.mp4")));
            graph.push(format!("[{i}:v]settb=AVTB,setpts=PTS-STARTPTS[v{i}]"));
            for k in 0..titles.len() { graph.push(format!("[{i}:a:{k}]asetpts=PTS-STARTPTS[a{i}_{k}]")); }
        }
        let mut video = "v0".to_owned();
        let mut audio: Vec<String> = (0..titles.len()).map(|k| format!("a0_{k}")).collect();
        let mut offset = edit.segments[0].1 - edit.segments[0].0;
        for (i, &(start, end)) in edit.segments.iter().enumerate().skip(1) {
            graph.push(format!("[{video}][v{i}]xfade=transition=fade:duration={}:offset={}[vx{i}]", edit.audio.crossfade, offset - edit.audio.crossfade));
            video = format!("vx{i}");
            for (k, label) in audio.iter_mut().enumerate() {
                graph.push(format!("[{label}][a{i}_{k}]acrossfade=d={}:c1=tri:c2=tri[ax{i}_{k}]", edit.audio.crossfade));
                *label = format!("ax{i}_{k}");
            }
            offset += end - start - edit.audio.crossfade;
        }
        for (k, label) in audio.iter_mut().enumerate() {
            if !filters.is_empty() { graph.push(format!("[{label}]{}[out{k}]", filters.join(","))); *label = format!("out{k}"); }
        }
        cmd.args(["-filter_complex", &graph.join(";"), "-map", &format!("[{video}]"), "-c:v", "libx264", "-preset", "fast", "-crf", "20", "-pix_fmt", "yuv420p", "-threads", "2"]);
        for (k, (label, title)) in audio.iter().zip(&titles).enumerate() {
            cmd.args(["-map", &format!("[{label}]"), &format!("-metadata:s:a:{k}"), &format!("handler_name={title}")]);
        }
        cmd.args(["-c:a", "aac", "-b:a", "160k", "-ar", "48000", "-t", &total.to_string(), "-movflags", "+faststart"]);
    } else {
        cmd.args(["-f", "concat", "-safe", "1", "-i"]).arg(manifest)
            .args(["-map", "0", "-c", "copy", "-movflags", "+faststart"]);
        if !filters.is_empty() && !crate::media::audio_track_titles(src)?.is_empty() {
            cmd.args(["-c:a", "aac", "-b:a", "160k", "-ar", "48000", "-af", &filters.join(",")]);
        }
    }
    cmd.arg(&out);
    let report = |fraction: f64| (control.progress)(0.95 + fraction * 0.04);
    crate::ffmpeg::run(cmd, Some(&Control { cancel: control.cancel, progress: &report }), total)?;
    if control.cancel.load(std::sync::atomic::Ordering::Relaxed) { return Err(Error::Invalid("Export cancelled".into())); }
    std::fs::rename(out, dst)?;
    (control.progress)(1.0);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validates_segments_and_remaps_bookmarks() {
        let mut edit = EditSpec { audio: AudioEffects::default(), segments: vec![(1.0, 3.0), (5.0, 7.0)], crop: Crop::default() };
        edit.validate(10.0).unwrap();
        assert_eq!(edit.markers(&[0.0, 2.0, 3.0, 4.0, 6.0]), vec![1.0, 3.0]);
        edit.segments[1].0 = 2.0;
        edit.validate(10.0).unwrap(); // reordered/overlapping source ranges are allowed
        edit.segments = vec![(0.0, f64::NAN)];
        assert!(edit.validate(10.0).is_err());
        edit.segments = vec![(0.0, 1.0)];
        edit.crop.left = 1.0;
        assert!(edit.validate(10.0).is_err());
    }
}
