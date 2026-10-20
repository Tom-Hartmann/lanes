//! The clip-saved chime: a short two-note sound played through WASAPI on a
//! chosen output device. Not the Windows sound scheme (which is often silent)
//! and not a file: the samples are generated, so nothing can be missing.

use std::time::Duration;

use windows::Win32::Media::Audio::{
    AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM, AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,
    IAudioRenderClient,
};

use crate::error::{Error, Result, WinContext};
use crate::util::{ComGuard, Priority, spawn};

const RATE: f32 = 48_000.0;

/// Plays the chime asynchronously. `device` is an output device id (`None` =
/// default output); `volume` is 0â€“100. Failures are logged, never raised.
pub fn play(device: Option<String>, volume: u8) {
    let spawned = spawn("lanes-chime", Priority::Normal, move || {
        if let Err(e) = render(device.as_deref(), &samples(f32::from(volume.min(100)) / 100.0)) {
            log::warn!("clip sound could not play: {e}");
        }
    });
    if let Err(e) = spawned {
        log::warn!("clip sound thread: {e}");
    }
}

/// Interleaved stereo f32: two short bell-like notes (E6 then A6).
fn samples(volume: f32) -> Vec<f32> {
    let gain = 0.35 * volume * volume; // perceptual curve
    let note = |freq: f32, start: f32, len: f32, out: &mut Vec<f32>| {
        let first = (start * RATE) as usize;
        let n = (len * RATE) as usize;
        if out.len() < (first + n) * 2 {
            out.resize((first + n) * 2, 0.0);
        }
        for i in 0..n {
            let t = i as f32 / RATE;
            let attack = (t / 0.004).min(1.0);
            let release = ((len - t) / 0.02).clamp(0.0, 1.0);
            let env = attack * release * (-t * 9.0).exp();
            let s = gain * env * ((std::f32::consts::TAU * freq * t).sin() + 0.25 * (std::f32::consts::TAU * 2.0 * freq * t).sin());
            out[(first + i) * 2] += s;
            out[(first + i) * 2 + 1] += s;
        }
    };
    let mut out = Vec::new();
    note(1318.5, 0.0, 0.16, &mut out);
    note(1760.0, 0.09, 0.30, &mut out);
    out
}

fn render(device: Option<&str>, samples: &[f32]) -> Result<()> {
    let _com = ComGuard::mta();
    let (client, name) = super::wasapi::activate_endpoint(device, true)?;
    let format = super::wasapi::float_format();
    let frames_total = samples.len() / 2;
    // SAFETY: the format outlives Initialize; every GetBuffer is matched by a
    // ReleaseBuffer of the same size, and writes stay within the frames granted.
    unsafe {
        client
            .Initialize(
                AUDCLNT_SHAREMODE_SHARED,
                AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,
                2_000_000,
                0,
                &format.Format,
                None,
            )
            .ctx("chime Initialize")?;
        let buffer = client.GetBufferSize().ctx("GetBufferSize")? as usize;
        let render: IAudioRenderClient = client.GetService().ctx("IAudioRenderClient")?;
        client.Start().ctx("chime Start")?;
        let mut written = 0usize;
        while written < frames_total {
            let padding = client.GetCurrentPadding().ctx("GetCurrentPadding")? as usize;
            let n = buffer.saturating_sub(padding).min(frames_total - written);
            if n > 0 {
                let data = render.GetBuffer(n as u32).ctx("GetBuffer")? as *mut f32;
                if data.is_null() {
                    return Err(Error::Unsupported("render buffer was null".into()));
                }
                std::ptr::copy_nonoverlapping(samples.as_ptr().add(written * 2), data, n * 2);
                render.ReleaseBuffer(n as u32, 0).ctx("ReleaseBuffer")?;
                written += n;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        // Let the device play out what is queued, then stop.
        for _ in 0..100 {
            if client.GetCurrentPadding().unwrap_or(0) == 0 {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let _ = client.Stop();
    }
    log::info!("clip sound played on {name}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chime_is_short_quiet_and_scales_with_volume() {
        let loud = samples(1.0);
        let mute = samples(0.0);
        assert!(loud.len() / 2 < (0.5 * RATE) as usize, "under half a second");
        let peak = loud.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!(peak > 0.1 && peak < 0.9, "peak {peak}");
        assert!(mute.iter().all(|s| *s == 0.0));
        assert!(loud.iter().all(|s| s.is_finite()));
    }
}
