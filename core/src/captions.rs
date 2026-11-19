use std::{path::Path, process::Command, os::windows::process::CommandExt};
use crate::{Result, Error, ffmpeg::{self, Control}};

/// Local whisper.cpp engine; outputs an editable SRT beside the clip.
pub fn transcribe(src: &Path, dst: &Path, engine: &Path, model: &Path, control: &Control<'_>) -> Result<()> {
    if !engine.is_file() || !model.is_file() { return Err(Error::Invalid("Install the optional local speech engine with scripts/setup-speech.ps1 first".into())); }
    let work = crate::files::WorkDir::new(dst.parent().unwrap_or(Path::new(".")))?;
    let audio = work.path().join("audio.wav");
    let mut cmd = ffmpeg::command()?;
    cmd.arg("-i").arg(src).args(["-map", "0:a:0", "-vn", "-ar", "16000", "-ac", "1", "-c:a", "pcm_s16le"]).arg(&audio);
    let extract = |f| (control.progress)(f * 0.1);
    ffmpeg::run(cmd, Some(&Control { cancel: control.cancel, progress: &extract }), crate::media::duration(src).unwrap_or(1.0))?;
    let prefix = work.path().join("captions");
    let mut cmd = Command::new(engine);
    cmd.creation_flags(0x0800_0000).arg("-m").arg(model).arg("-f").arg(audio)
        .args(["-l", "auto", "-osrt", "-of"]).arg(&prefix).args(["-t", "4", "-ng"]);
    let speech = |f| (control.progress)(0.1 + f * 0.89);
    ffmpeg::run_worker(cmd, Some(&Control { cancel: control.cancel, progress: &speech }), 1.0)?;
    let srt = prefix.with_extension("srt");
    if !srt.is_file() { return Err(Error::Invalid("Speech engine did not produce captions".into())); }
    if control.cancel.load(std::sync::atomic::Ordering::Relaxed) { return Err(Error::Invalid("Export cancelled".into())); }
    std::fs::rename(srt, dst)?;
    (control.progress)(1.0);
    Ok(())
}
