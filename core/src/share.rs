//! Share copies: one MP4 with the video and a SINGLE audio track mixed from
//! the chosen lanes (Discord, browsers and most players only play the first
//! track), optionally re-encoded to fit a size limit such as Discord's 10 MB.
//! Also edited copies for the library: a cut plus a re-rendered mixed track,
//! with the lanes kept.

use std::path::Path;
use std::ptr;

use crate::av::{self, CodecContext, CodecParams, Frame, Packet, ffi};
use crate::config::{EncoderBackend, MIX_STREAM, MIX_TRACK_TITLE, VideoCodec};
use crate::error::{Error, Result, av_check};
use crate::media::{Input, decode_video, open_decoder, to_nv12};

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

/// Writes the share copy of `src` to `dst` (MP4).
pub fn export(src: &Path, dst: &Path, opts: &ShareOptions) -> Result<ShareResult> {
    if let Some((start, end)) = opts.range {
        let cut = temp_cut_path(dst);
        crate::media::trim(src, &cut, start, end)?;
        let result = export(&cut, dst, &ShareOptions { range: None, ..opts.clone() });
        let _ = std::fs::remove_file(&cut);
        return result;
    }
    let input = Input::open(src)?;
    let v = input.video_stream().ok_or_else(|| Error::Unsupported("no video stream".into()))?;
    // SAFETY: valid context.
    let duration = (unsafe { (*input.0).duration } as f64 / f64::from(ffi::AV_TIME_BASE)).max(0.5);
    drop(input);

    // Audio: decode the chosen lanes, mix, encode one AAC track.
    let (streams, mix_track) = audio_layout(src)?;
    let chosen = chosen_lanes(&streams, mix_track, &opts.mix);
    let audio = if chosen.is_empty() {
        None
    } else {
        Some(encode_mix(src, &chosen, &opts.mix.gains_db, av::StreamId::Audio(0), "Mix")?)
    };

    // Video: copy if the file already fits, otherwise re-encode to fit.
    let src_bytes = std::fs::metadata(src).map(|m| m.len()).unwrap_or(u64::MAX);
    let video_kbps = opts.max_bytes.filter(|&max| src_bytes > max).map(|max| target_video_kbps(max, duration));

    let tmp = dst.with_extension("partial");
    let written = (|| -> Result<bool> {
        let (video_info, video_packets, reencoded) = match video_kbps {
            None => {
                let (info, packets) = copy_video(src, v)?;
                (info, packets, false)
            }
            Some(kbps) => {
                let (info, packets) = reencode_video(src, v, opts.backend, kbps)?;
                (info, packets, true)
            }
        };
        let mut infos = vec![video_info.clone()];
        if let Some((info, _)) = &audio {
            infos.push(info.clone());
        }
        let mut mux = crate::mux::Muxer::create(&tmp, crate::mux::Container::Mp4, &infos)?;
        // Interleave by time so the MP4 muxer does not buffer everything.
        let secs = |p: &av::EncodedPacket, tb: ffi::AVRational| p.dts as f64 * f64::from(tb.num) / f64::from(tb.den);
        let mut all: Vec<(f64, &av::EncodedPacket)> = video_packets.iter().map(|p| (secs(p, video_info.time_base), p)).collect();
        if let Some((info, packets)) = &audio {
            all.extend(packets.iter().map(|p| (secs(p, info.time_base), p)));
        }
        all.sort_by(|a, b| a.0.total_cmp(&b.0));
        for (_, p) in all {
            mux.write(p, 0)?;
        }
        mux.finish()?;
        Ok(reencoded)
    })();
    let reencoded = match written {
        Ok(r) => r,
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            return Err(e);
        }
    };
    std::fs::rename(&tmp, dst)?;
    let size_bytes = std::fs::metadata(dst).map(|m| m.len()).unwrap_or(0);
    Ok(ShareResult { path: dst.to_string_lossy().into_owned(), size_bytes, reencoded })
}

/// An edited copy for the library: the part `range` (lossless cut, the start
/// snaps back to a keyframe) and, with `mix`, a new first track
/// "Mix (all lanes)" rendered from the lanes at these volumes. Video and the
/// separate lanes are copied unchanged (a previous mixed track is replaced),
/// so the copy can be remixed later. Players and Discord play the new mix.
pub fn edited_copy(src: &Path, dst: &Path, range: Option<(f64, f64)>, mix: Option<&MixSpec>) -> Result<()> {
    let Some(spec) = mix else {
        return match range {
            Some((start, end)) => crate::media::trim(src, dst, start, end),
            None => crate::media::remux(src, dst),
        };
    };
    let Some((start, end)) = range else { return remix(src, dst, spec) };
    // Cut first, then remix the cut, so only the kept part is decoded.
    let cut = temp_cut_path(dst);
    crate::media::trim(src, &cut, start, end)?;
    let result = remix(&cut, dst, spec);
    let _ = std::fs::remove_file(&cut);
    result
}

/// A temporary file for a cut, in the system temp folder (not the clips
/// folder, where the library would list it), with `dst`'s container.
fn temp_cut_path(dst: &Path) -> std::path::PathBuf {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    let ext = dst.extension().and_then(|e| e.to_str()).unwrap_or("mkv");
    std::env::temp_dir().join(format!("lanes-cut-{}-{nanos}.{ext}", std::process::id()))
}

/// Copies `src` with a newly rendered mixed track first (see `edited_copy`).
fn remix(src: &Path, dst: &Path, spec: &MixSpec) -> Result<()> {
    let (streams, mix_track) = audio_layout(src)?;
    let chosen = chosen_lanes(&streams, mix_track, spec);
    if chosen.is_empty() {
        return Err(Error::Invalid("every lane is muted in the audio mix".into()));
    }
    let (info, packets) = encode_mix(src, &chosen, &spec.gains_db, av::StreamId::Audio(MIX_STREAM), MIX_TRACK_TITLE)?;
    crate::media::copy_streams(
        src,
        dst,
        &crate::media::CopyOptions {
            skip: mix_track.map(|k| streams[k]),
            first_audio: Some((&info, &packets)),
            ..Default::default()
        },
    )
}

/// The file's audio streams (input indexes) and which of them, if any, is a
/// Lanes mixed track.
fn audio_layout(src: &Path) -> Result<(Vec<usize>, Option<usize>)> {
    let input = Input::open(src)?;
    let streams: Vec<usize> = input
        .streams()
        .iter()
        .enumerate()
        // SAFETY: valid streams of the open input.
        .filter(|(_, st)| unsafe { (*(***st).codecpar).codec_type == ffi::AVMediaType::AVMEDIA_TYPE_AUDIO })
        .map(|(i, _)| i)
        .collect();
    drop(input);
    let titles = crate::media::audio_track_titles(src).unwrap_or_default();
    let mix = if streams.len() > 1 { titles.iter().position(|t| t == MIX_TRACK_TITLE) } else { None };
    Ok((streams, mix))
}

/// (track, stream index) pairs to mix: the chosen lanes, never the mixed
/// track (it already contains them).
fn chosen_lanes(streams: &[usize], mix_track: Option<usize>, spec: &MixSpec) -> Vec<(usize, usize)> {
    match &spec.tracks {
        None => streams.iter().copied().enumerate().filter(|&(k, _)| Some(k) != mix_track).collect(),
        Some(sel) => sel.iter().filter(|&&k| Some(k) != mix_track).filter_map(|&k| streams.get(k).map(|&i| (k, i))).collect(),
    }
}

/// Decodes the `chosen` tracks, mixes them at `gains_db` and encodes one AAC
/// track, timed from the file start.
fn encode_mix(src: &Path, chosen: &[(usize, usize)], gains_db: &[f32], id: av::StreamId, title: &str) -> Result<(av::StreamInfo, Vec<av::EncodedPacket>)> {
    let mut mix: Vec<f32> = Vec::new();
    for &(k, idx) in chosen {
        let gain = crate::audio::dsp::gain_for(gains_db.get(k).copied().unwrap_or(0.0), false);
        decode_audio_into(src, idx, gain, &mut mix)?;
    }
    crate::audio::dsp::soft_clip(&mut mix);
    let mut enc = crate::encode::audio::AudioEncoder::open(crate::config::AudioCodec::Aac, AUDIO_KBPS)?;
    let mut out = Vec::new();
    for block in mix.chunks(4800) {
        enc.push(block, &mut out)?;
    }
    enc.flush(&mut out)?;
    let tb = enc.context().time_base();
    let packets = out.iter().map(|p| av::EncodedPacket::from_av(id, p, tb, crate::Timestamp(0))).filter(|p| p.pts >= 0).collect();
    let info = av::StreamInfo { id, params: std::sync::Arc::new(enc.context().parameters()?), time_base: tb, title: title.into() };
    Ok((info, packets))
}

/// Video bitrate that makes a `duration`-second file fit `max_bytes`
/// alongside the audio track (4 % reserved for container overhead).
pub fn target_video_kbps(max_bytes: u64, duration: f64) -> u32 {
    let budget = (max_bytes as f64 * 0.96) as u64;
    let audio_bytes = (f64::from(AUDIO_KBPS) * 125.0 * duration) as u64;
    let video_bits = budget.saturating_sub(audio_bytes) as f64 * 8.0;
    ((video_bits / duration.max(0.5)) / 1000.0).clamp(300.0, 50_000.0) as u32
}

/// The video stream's packets unchanged, starting at the first keyframe.
fn copy_video(src: &Path, v: usize) -> Result<(av::StreamInfo, Vec<av::EncodedPacket>)> {
    let input = Input::open(src)?;
    let st = input.streams()[v];
    // SAFETY: valid stream; parameters are deep-copied.
    let (params, tb) = unsafe { (CodecParams::from_raw_copy((*st).codecpar)?, (*st).time_base) };
    let info = av::StreamInfo { id: av::StreamId::Video(0), params: std::sync::Arc::new(params), time_base: tb, title: String::new() };
    let mut out: Vec<av::EncodedPacket> = Vec::new();
    let mut pkt = Packet::alloc()?;
    let mut last_dts: Option<i64> = None;
    while input.read(&mut pkt)?.is_some() {
        let raw = pkt.raw();
        if raw.stream_index as usize != v {
            continue;
        }
        let keyframe = raw.flags & ffi::AV_PKT_FLAG_KEY != 0;
        if out.is_empty() && !keyframe {
            continue;
        }
        let pts = if raw.pts == ffi::AV_NOPTS_VALUE { raw.dts } else { raw.pts };
        let mut dts = if raw.dts == ffi::AV_NOPTS_VALUE { pts } else { raw.dts };
        if let Some(prev) = last_dts.filter(|&p| dts <= p) {
            dts = prev + 1;
        }
        last_dts = Some(dts);
        out.push(av::EncodedPacket {
            stream: av::StreamId::Video(0),
            data: pkt.data().into(),
            pts: pts.max(dts),
            dts,
            duration: raw.duration,
            keyframe,
            time: crate::Timestamp(0),
        });
    }
    Ok((info, out))
}

/// Re-encodes the video stream to H.264 at `kbps` (hardware encoder, x264 fallback).
fn reencode_video(src: &Path, v: usize, backend: EncoderBackend, kbps: u32) -> Result<(av::StreamInfo, Vec<av::EncodedPacket>)> {
    let input = Input::open(src)?;
    let dec = open_decoder(&input, v)?;
    // SAFETY: valid decoder context and stream.
    let (w, h, fr) = unsafe { ((*dec.0).width, (*dec.0).height, (*input.streams()[v]).avg_frame_rate) };
    let fps = if fr.num > 0 && fr.den > 0 { (fr.num / fr.den).max(1) } else { 60 };
    let open = |name: &str| -> Result<CodecContext> {
        let enc = CodecContext::encoder(name)?;
        // SAFETY: not yet opened; plain field setup.
        unsafe {
            let c = &mut *enc.0;
            c.width = w;
            c.height = h;
            c.time_base = av::rational(1, fps);
            c.framerate = av::rational(fps, 1);
            c.pix_fmt = ffi::AVPixelFormat::AV_PIX_FMT_NV12;
            c.gop_size = fps * 2;
            c.max_b_frames = 0;
            c.bit_rate = i64::from(kbps) * 1000;
            c.rc_max_rate = c.bit_rate;
            c.rc_buffer_size = (c.bit_rate * 2) as i32;
            c.color_primaries = ffi::AVColorPrimaries::AVCOL_PRI_BT709;
            c.color_trc = ffi::AVColorTransferCharacteristic::AVCOL_TRC_BT709;
            c.colorspace = ffi::AVColorSpace::AVCOL_SPC_BT709;
            c.color_range = ffi::AVColorRange::AVCOL_RANGE_MPEG;
            c.flags |= ffi::AV_CODEC_FLAG_GLOBAL_HEADER as i32;
        }
        // A size target needs a hard rate cap.
        enc.set_opt("rc", "cbr");
        enc.open()?;
        Ok(enc)
    };
    let name = crate::encode::encoder_name(backend, VideoCodec::H264).unwrap_or("libx264");
    let enc = match open(name) {
        Ok(e) => e,
        Err(e) if name != "libx264" => {
            log::warn!("{name} unavailable for the share copy ({e}); using x264");
            open("libx264")?
        }
        Err(e) => return Err(e),
    };
    log::info!("share copy: re-encoding {} with {name} at {kbps} kbps", src.display());
    let tb = enc.time_base();
    let info = av::StreamInfo { id: av::StreamId::Video(0), params: std::sync::Arc::new(enc.parameters()?), time_base: tb, title: String::new() };
    let mut packets = Vec::new();
    let mut out = Vec::new();
    let mut n = 0i64;
    decode_video(&input, v, &dec, |frame| {
        let nv12 = to_nv12(frame)?;
        // SAFETY: owned frame.
        unsafe { (*nv12.0).pts = n };
        n += 1;
        enc.encode(Some(&nv12), &mut out)?;
        packets.extend(out.drain(..).map(|p| av::EncodedPacket::from_av(av::StreamId::Video(0), &p, tb, crate::Timestamp(0))));
        Ok(true)
    })?;
    enc.encode(None, &mut out)?;
    packets.extend(out.drain(..).map(|p| av::EncodedPacket::from_av(av::StreamId::Video(0), &p, tb, crate::Timestamp(0))));
    Ok((info, packets))
}

/// Decodes one audio stream to interleaved stereo f32 at 48 kHz and adds it,
/// times `gain`, into `mix`, placed by timestamp from 0 (Lanes files are
/// 48 kHz; others are not resampled).
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
        // Very long clips hit the quality floor instead of going to zero.
        assert_eq!(target_video_kbps(1024 * 1024, 600.0), 300);
    }
}
