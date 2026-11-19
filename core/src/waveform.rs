//! Fixed-size peaks; PCM stays on disk and is read with bounded memory.
use std::{io::{BufReader, Read}, path::Path};
use crate::{Result, Error};

pub fn peaks(src: &Path) -> Result<Vec<f32>> {
    if crate::media::audio_track_titles(src)?.is_empty() { return Ok(Vec::new()); }
    let duration = crate::media::duration(src).ok_or_else(|| Error::Invalid("Unknown audio duration".into()))?;
    if !duration.is_finite() || duration <= 0.0 { return Err(Error::Invalid("Invalid audio duration".into())); }
    let work = crate::files::WorkDir::new(&std::env::temp_dir())?;
    let pcm = work.path().join("wave.pcm");
    let mut cmd = crate::ffmpeg::command()?;
    cmd.arg("-i").arg(src).args(["-map", "0:a:0", "-vn", "-ac", "1", "-ar", "8000", "-f", "s16le"]).arg(&pcm);
    crate::ffmpeg::run(cmd, None, duration)?;
    let file = std::fs::File::open(pcm)?;
    let samples = file.metadata()?.len() / 2;
    let mut reader = BufReader::new(file);
    let mut out = vec![0.0_f32; 1200];
    let mut bytes = [0u8; 2];
    for i in 0..samples {
        reader.read_exact(&mut bytes)?;
        let bin = ((i as u128 * 1200) / samples as u128) as usize;
        out[bin] = out[bin].max((i16::from_le_bytes(bytes) as f32 / 32768.0).abs());
    }
    Ok(out)
}
