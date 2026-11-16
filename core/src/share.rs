//! Share copies: one MP4 with the video and a SINGLE audio track mixed from
//! the chosen lanes (Discord, browsers and most players only play the first
//! track), optionally re-encoded to fit a size limit such as Discord's 10 MB.
//! Also edited copies for the library: a cut plus a re-rendered mixed track,
//! with the lanes kept.

use std::path::Path;
#[cfg(test)]
use std::ptr;

#[cfg(test)]
use crate::av::{self, CodecContext, Frame, Packet, ffi};
use crate::config::{EncoderBackend, MIX_TRACK_TITLE, VideoCodec};
use crate::error::{Error, Result};
#[cfg(test)]
use crate::error::av_check;
#[cfg(test)]
use crate::media::Input;

/// The clip viewer's audio mix: which lanes to hear and how loud.
#[derive(Clone, Debug, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MixSpec {
    /// Audio tracks (0-based among the file's audio streams) to mix; `None` =
    /// every lane. A Lanes mixed track is never part of it (it holds the lanes).
    pub tracks: Option<Vec<usize>>,
    /// Volume per audio track in dB (same indexing); missing entries are 0 dB.
    pub gains_db: Vec<f32>,
}

#[derive(Clone, Debug)]
pub struct ShareOptions {
    /// The lanes mixed into the single output track.
    pub mix: MixSpec,
    /// Only this part (seconds); the start snaps back to a keyframe.
    pub range: Option<(f64, f64)>,
    /// Size limit for the whole file; the video is re-encoded only if needed.
    pub max_bytes: Option<u64>,
    /// Hardware encoder for re-encoding (falls back to x264). Output is H.264,
    /// which plays everywhere, Discord embeds included.
    pub backend: EncoderBackend,
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShareResult {
    pub path: String,
    pub size_bytes: u64,
    pub reencoded: bool,
}

const AUDIO_KBPS: u32 = 160;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum OriginalAudio { Discard, Copy, Encode }

/// Adds a bounded streaming audio mix. Original lanes can be retained behind it.
pub(crate) fn audio_args(cmd: &mut std::process::Command, src: &Path, spec: Option<&MixSpec>, originals: OriginalAudio) -> Result<()> {
    let keep_lanes = originals != OriginalAudio::Discard;
    let titles = crate::media::audio_track_titles(src)?;
    let Some(spec) = spec else {
        cmd.args(["-map", "0:a?", "-c:a", "aac", "-b:a", "160k", "-ar", "48000"]);
        return Ok(());
    };
    if titles.is_empty() { return Ok(()); }
    let mixed = (titles.len() > 1).then(|| titles.iter().position(|t| t == MIX_TRACK_TITLE)).flatten();
    let selected = spec.tracks.clone().unwrap_or_else(|| (0..titles.len()).collect());
    if selected.iter().any(|&k| k >= titles.len()) { return Err(Error::Invalid("Unknown audio lane".into())); }
    let mut chosen: Vec<_> = selected.into_iter().filter(|k| Some(*k) != mixed).collect();
    chosen.sort_unstable(); chosen.dedup();
    if chosen.is_empty() { return Err(Error::Invalid("every lane is muted in the audio mix".into())); }
    let mut graph = Vec::new();
    for &k in &chosen {
        let db = spec.gains_db.get(k).copied().unwrap_or(0.0);
        if !db.is_finite() || !(-96.0..=24.0).contains(&db) { return Err(Error::Invalid("Audio gain must be between -96 and +24 dB".into())); }
        graph.push(format!("[0:a:{k}]aresample=48000,aformat=channel_layouts=stereo,volume={db}dB[a{k}]"));
    }
    let inputs: String = chosen.iter().map(|k| format!("[a{k}]")).collect();
    graph.push(format!("{inputs}amix=inputs={}:normalize=0:duration=longest,alimiter=limit=1:level=false:latency=true[mix]", chosen.len()));
    cmd.arg("-filter_complex").arg(graph.join(";")).args(["-map", "[mix]", "-c:a", "aac", "-b:a", "160k", "-ar", "48000"]);
    cmd.args(["-metadata:s:a:0", &format!("title={}", if keep_lanes { MIX_TRACK_TITLE } else { "Mix" }), "-metadata:s:a:0", &format!("handler_name={}", if keep_lanes { MIX_TRACK_TITLE } else { "Mix" }), "-disposition:a:0", "default"]);
    if keep_lanes {
        for (out, (k, title)) in (1..).zip(titles.iter().enumerate().filter(|(k, _)| Some(*k) != mixed)) {
            cmd.args(["-map", &format!("0:a:{k}"), &format!("-metadata:s:a:{out}"), &format!("title={title}"), &format!("-metadata:s:a:{out}"), &format!("handler_name={title}"), &format!("-disposition:a:{out}"), "0"]);
            if originals == OriginalAudio::Copy { cmd.args([&format!("-c:a:{out}"), "copy"]); }
        }
    }
    Ok(())
}

/// Export uses streaming FFmpeg decoding/muxing; memory does not scale with duration.
pub fn export(src: &Path, dst: &Path, opts: &ShareOptions) -> Result<ShareResult> {
    if let Some((start, end)) = opts.range {
        let work = crate::files::WorkDir::new(dst.parent().unwrap_or(Path::new(".")))?;
        let cut = work.path().join("cut.mkv");
        crate::media::trim(src, &cut, start, end)?;
        return export(&cut, dst, &ShareOptions { range: None, ..opts.clone() });
    }
    let duration = crate::media::duration(src).filter(|d| d.is_finite() && *d > 0.0).ok_or_else(|| Error::Invalid("Cannot determine clip duration".into()))?;
    let work = crate::files::WorkDir::new(dst.parent().unwrap_or(Path::new(".")))?;
    let tmp = work.path().join("share.mp4");
    let size = std::fs::metadata(src)?.len();
    let mut kbps = opts.max_bytes.filter(|&max| size > max).map(|max| target_video_kbps(max, duration));
    for _ in 0..3 {
        if kbps.is_some_and(|k| k < 300) { return Err(Error::Invalid("This clip is too long for the selected size. Shorten it or choose a larger limit.".into())); }
        let render = |backend| -> Result<()> {
            let mut cmd = crate::ffmpeg::command()?;
            cmd.arg("-i").arg(src).args(["-map", "0:v:0"]);
            audio_args(&mut cmd, src, Some(&opts.mix), OriginalAudio::Discard)?;
            match kbps {
                None => { cmd.args(["-c:v", "copy"]); }
                Some(rate) => {
                    let name = crate::encode::encoder_name(backend, VideoCodec::H264).unwrap_or("libx264");
                    cmd.args(["-c:v", name, "-b:v", &format!("{rate}k"), "-maxrate", &format!("{rate}k"), "-bufsize", &format!("{}k", rate * 2), "-pix_fmt", "yuv420p", "-threads", "2"]);
                    if name == "libx264" { cmd.args(["-preset", "fast"]); }
                }
            }
            cmd.args(["-movflags", "+faststart"]).arg(&tmp);
            crate::ffmpeg::run(cmd, None, duration)
        };
        if let Err(e) = render(opts.backend) {
            if kbps.is_none() || opts.backend == EncoderBackend::X264 { return Err(e); }
            log::warn!("hardware export unavailable: {e}; retrying with x264");
            render(EncoderBackend::X264)?;
        }
        let size_bytes = std::fs::metadata(&tmp)?.len();
        if opts.max_bytes.is_none_or(|max| size_bytes <= max) {
            std::fs::rename(&tmp, dst)?;
            return Ok(ShareResult { path: dst.to_string_lossy().into_owned(), size_bytes, reencoded: kbps.is_some() });
        }
        let max = opts.max_bytes.expect("checked limit");
        kbps = Some(match kbps {
            None => target_video_kbps(max, duration),
            Some(rate) => (f64::from(rate) * max as f64 / size_bytes as f64 * 0.85) as u32,
        });
    }
    Err(Error::Invalid("The export could not fit the selected size. Choose a larger limit or shorten the selection.".into()))
}

/// Existing lossless trim path; mix rendering is streamed through the worker.
pub fn edited_copy(src: &Path, dst: &Path, range: Option<(f64, f64)>, mix: Option<&MixSpec>) -> Result<()> {
    let Some(spec) = mix else {
        return match range { Some((a, b)) => crate::media::trim(src, dst, a, b), None => crate::media::remux(src, dst) };
    };
    let work = crate::files::WorkDir::new(dst.parent().unwrap_or(Path::new(".")))?;
    let cut = work.path().join("cut.mkv");
    let input = if let Some((a, b)) = range { crate::media::trim(src, &cut, a, b)?; &cut } else { src };
    let tmp = work.path().join(format!("mix.{}", dst.extension().and_then(|s| s.to_str()).unwrap_or("mkv")));
    let mut cmd = crate::ffmpeg::command()?;
    cmd.arg("-i").arg(input).args(["-map", "0:v?", "-c:v", "copy"]);
    audio_args(&mut cmd, input, Some(spec), OriginalAudio::Copy)?;
    cmd.arg(&tmp);
    crate::ffmpeg::run(cmd, None, crate::media::duration(input).unwrap_or(1.0))?;
    std::fs::rename(tmp, dst)?;
    Ok(())
}

/// Budget includes audio and container overhead. Callers reject <300 kbps.
pub fn target_video_kbps(max_bytes: u64, duration: f64) -> u32 {
    let budget = max_bytes as f64 * 0.94;
    ((budget * 8.0 / duration.max(0.5) / 1000.0 - f64::from(AUDIO_KBPS)).max(0.0) as u32).min(50_000)
}

#[cfg(test)]
fn decode_audio_into(src: &Path, idx: usize, gain: f32, mix: &mut Vec<f32>) -> Result<()> {
    let input = Input::open(src)?;
    let st = input.streams()[idx];
    // SAFETY: valid parameters; the decoder context is owned by CodecContext.
    let (dec, tb) = unsafe {
        let codec = ffi::avcodec_find_decoder((*(*st).codecpar).codec_id);
        if codec.is_null() {
            return Err(Error::Unsupported("no decoder for this audio track".into()));
        }
        let ctx = CodecContext(ffi::avcodec_alloc_context3(codec));
        av_check(ffi::avcodec_parameters_to_context(ctx.0, (*st).codecpar), "avcodec_parameters_to_context")?;
        ctx.open()?;
        (ctx, (*st).time_base)
    };
    let mut pkt = Packet::alloc()?;
    let frame = Frame::alloc()?;
    // Where the next frame goes when it has no timestamp (in samples).
    let mut cursor = 0usize;
    let mut drain = |out: &mut Vec<f32>| -> Result<()> {
        loop {
            // SAFETY: valid decoder and frame.
            let ret = unsafe { ffi::avcodec_receive_frame(dec.0, frame.0) };
            if ret == ffi::AVERROR(ffi::EAGAIN) || ret == ffi::AVERROR_EOF {
                return Ok(());
            }
            av_check(ret, "avcodec_receive_frame audio")?;
            // SAFETY: the decoded frame's planes hold nb_samples samples per
            // channel in its (float) sample format; reads stay within them.
            unsafe {
                let f = &*frame.0;
                let n = f.nb_samples.max(0) as usize;
                let ch = f.ch_layout.nb_channels.max(1) as usize;
                let pts = if f.pts == ffi::AV_NOPTS_VALUE {
                    (cursor / 2) as i64
                } else {
                    av::rescale(f.pts, tb, av::rational(1, 48_000))
                };
                if pts < 0 {
                    continue;
                }
                let at = pts as usize * 2;
                cursor = at + n * 2;
                if out.len() < at + n * 2 {
                    out.resize(at + n * 2, 0.0);
                }
                let planar = f.format == ffi::AVSampleFormat::AV_SAMPLE_FMT_FLTP as i32;
                let packed = f.format == ffi::AVSampleFormat::AV_SAMPLE_FMT_FLT as i32;
                for i in 0..n {
                    let sample = |c: usize| -> f32 {
                        let c = c.min(ch - 1);
                        if planar {
                            *(f.data[c] as *const f32).add(i)
                        } else if packed {
                            *(f.data[0] as *const f32).add(i * ch + c)
                        } else {
                            0.0
                        }
                    };
                    out[at + i * 2] += sample(0) * gain;
                    out[at + i * 2 + 1] += sample(1) * gain;
                }
            }
        }
    };
    while input.read(&mut pkt)?.is_some() {
        if pkt.raw().stream_index as usize != idx {
            continue;
        }
        // SAFETY: valid decoder and packet.
        unsafe { ffi::avcodec_send_packet(dec.0, pkt.0) };
        drain(mix)?;
    }
    // SAFETY: a null packet flushes the decoder.
    unsafe { ffi::avcodec_send_packet(dec.0, ptr::null()) };
    drain(mix)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mux::{Container, Muxer};
    use std::f32::consts::SQRT_2;

    /// A 440 Hz sine, interleaved stereo at 48 kHz.
    fn sine(amplitude: f32, secs: f32) -> Vec<f32> {
        let n = (48_000.0 * secs) as usize;
        (0..n)
            .flat_map(|i| {
                let s = amplitude * (i as f32 * 440.0 * std::f32::consts::TAU / 48_000.0).sin();
                [s, s]
            })
            .collect()
    }

    /// An audio-only file with one AAC track per (title, amplitude).
    fn write_tracks(path: &Path, tracks: &[(&str, f32)]) {
        let mut infos = Vec::new();
        let mut packets = Vec::new();
        for (k, &(title, amplitude)) in tracks.iter().enumerate() {
            let mut enc = crate::encode::audio::AudioEncoder::open(crate::config::AudioCodec::Aac, 160).unwrap();
            let id = av::StreamId::Audio(k as u8);
            let tb = enc.context().time_base();
            infos.push(av::StreamInfo { id, params: std::sync::Arc::new(enc.context().parameters().unwrap()), time_base: tb, title: title.into() });
            let mut out = Vec::new();
            for block in sine(amplitude, 2.0).chunks(4800) {
                enc.push(block, &mut out).unwrap();
            }
            enc.flush(&mut out).unwrap();
            packets.extend(out.iter().map(|p| av::EncodedPacket::from_av(id, p, tb, crate::Timestamp(0))).filter(|p| p.pts >= 0));
        }
        // All tracks share the 1/48000 time base.
        packets.sort_by_key(|p| p.dts);
        let mut mux = Muxer::create(path, Container::Matroska, &infos).unwrap();
        for p in &packets {
            mux.write(p, 0).unwrap();
        }
        mux.finish().unwrap();
    }

    /// RMS of one stream, without the edges (encoder start-up and padding).
    fn rms(path: &Path, stream: usize) -> f32 {
        let mut s = Vec::new();
        decode_audio_into(path, stream, 1.0, &mut s).unwrap();
        let mid = &s[s.len() / 4..s.len() * 3 / 4];
        (mid.iter().map(|x| x * x).sum::<f32>() / mid.len() as f32).sqrt()
    }

    fn assert_near(got: f32, want: f32, what: &str) {
        assert!((got - want).abs() < want * 0.05, "{what}: rms {got}, expected {want}");
    }

    fn packet_payloads(path: &Path, stream: usize) -> Vec<Vec<u8>> {
        let input = Input::open(path).unwrap();
        let mut packet = Packet::alloc().unwrap();
        let mut out = Vec::new();
        while input.read(&mut packet).unwrap().is_some() {
            if packet.raw().stream_index as usize == stream { out.push(packet.data().to_vec()); }
        }
        out
    }

    #[test]
    fn edited_copy_renders_the_mix_from_the_lanes() {
        let dir = std::env::temp_dir().join(format!("lanes-edit-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("clip.mkv");
        // A loud recorded mix and two lanes in phase.
        write_tracks(&src, &[(MIX_TRACK_TITLE, 0.8), ("Game", 0.3), ("Music", 0.2)]);

        // Music at -6 dB: the new mix is 0.3 + 0.1, the lanes stay as they were.
        let dst = dir.join("edit.mkv");
        edited_copy(&src, &dst, None, Some(&MixSpec { tracks: None, gains_db: vec![0.0, 0.0, -6.0206] })).unwrap();
        assert_eq!(crate::media::audio_track_titles(&dst).unwrap(), [MIX_TRACK_TITLE, "Game", "Music"]);
        assert_near(rms(&dst, 0), 0.4 / SQRT_2, "mix");
        assert_near(rms(&dst, 1), 0.3 / SQRT_2, "game lane");
        assert_near(rms(&dst, 2), 0.2 / SQRT_2, "music lane");
        assert_eq!(packet_payloads(&src, 1), packet_payloads(&dst, 1), "mix-only edits must preserve original lane packets");
        assert_eq!(packet_payloads(&src, 2), packet_payloads(&dst, 2));

        // Music muted (left out): the mix is only the game.
        let muted = dir.join("muted.mkv");
        edited_copy(&src, &muted, None, Some(&MixSpec { tracks: Some(vec![1]), gains_db: Vec::new() })).unwrap();
        assert_near(rms(&muted, 0), 0.3 / SQRT_2, "mix without music");

        // Every lane muted is an error, not a silent file.
        assert!(edited_copy(&src, &dir.join("silent.mkv"), None, Some(&MixSpec { tracks: Some(Vec::new()), gains_db: Vec::new() })).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn bitrate_fits_discord_limit() {
        // 30 s into 10 MB: about 2.4 Mbit/s video after audio and overhead.
        let kbps = target_video_kbps(10 * 1024 * 1024, 30.0);
        assert!((2000..2600).contains(&kbps), "{kbps}");
        let total_bytes = (f64::from(kbps + AUDIO_KBPS) * 125.0 * 30.0) as u64;
        assert!(total_bytes < 10 * 1024 * 1024);
        // Impossible budgets are rejected, rather than silently exceeding the limit.
        assert_eq!(target_video_kbps(1024 * 1024, 600.0), 0);
    }
}
