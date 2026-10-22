//! Share copies: one MP4 with the video and a SINGLE audio track mixed from
//! the chosen lanes (Discord, browsers and most players only play the first
//! track), optionally re-encoded to fit a size limit such as Discord's 10 MB.

use std::path::Path;
use std::ptr;

use crate::av::{self, CodecContext, CodecParams, Frame, Packet, ffi};
use crate::config::{EncoderBackend, VideoCodec};
use crate::error::{Error, Result, av_check};
use crate::media::{Input, decode_video, open_decoder, to_nv12};

#[derive(Clone, Debug)]
pub struct ShareOptions {
    /// Audio tracks (0-based among the file's audio streams) mixed into the
    /// single output track; `None` = all.
    pub audio_tracks: Option<Vec<usize>>,
    /// Volume per audio track in dB (same indexing); missing entries are 0 dB.
    pub gains_db: Vec<f32>,
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
    let input = Input::open(src)?;
    let v = input.video_stream().ok_or_else(|| Error::Unsupported("no video stream".into()))?;
    let audio_streams: Vec<usize> = input
        .streams()
        .iter()
        .enumerate()
        // SAFETY: valid streams of the open input.
        .filter(|(_, st)| unsafe { (*(***st).codecpar).codec_type == ffi::AVMediaType::AVMEDIA_TYPE_AUDIO })
        .map(|(i, _)| i)
        .collect();
    // (track, stream index) pairs.
    // "All" means the lanes: a mixed track (see `EncoderSettings::mix_track`)
    // already contains them and would double them.
    let titles = crate::media::audio_track_titles(src).unwrap_or_default();
    let is_mix = |k: usize| audio_streams.len() > 1 && titles.get(k).is_some_and(|t| t == crate::config::MIX_TRACK_TITLE);
    let chosen: Vec<(usize, usize)> = match &opts.audio_tracks {
        None => audio_streams.iter().copied().enumerate().filter(|&(k, _)| !is_mix(k)).collect(),
        Some(sel) => sel.iter().filter_map(|&k| audio_streams.get(k).map(|&i| (k, i))).collect(),
    };
    // SAFETY: valid context.
    let duration = (unsafe { (*input.0).duration } as f64 / f64::from(ffi::AV_TIME_BASE)).max(0.5);
    drop(input);

    // Audio: decode the chosen tracks, mix, encode one AAC track.
    let mut mix: Vec<f32> = Vec::new();
    for &(k, idx) in &chosen {
        let gain = crate::audio::dsp::gain_for(opts.gains_db.get(k).copied().unwrap_or(0.0), false);
        let samples = decode_audio(src, idx)?;
        if mix.len() < samples.len() {
            mix.resize(samples.len(), 0.0);
        }
        for (m, s) in mix.iter_mut().zip(&samples) {
            *m += s * gain;
        }
    }
    crate::audio::dsp::soft_clip(&mut mix);
    let mut audio: Option<(av::StreamInfo, Vec<av::EncodedPacket>)> = None;
    if !chosen.is_empty() {
        let mut enc = crate::encode::audio::AudioEncoder::open(crate::config::AudioCodec::Aac, AUDIO_KBPS)?;
        let mut out = Vec::new();
        for block in mix.chunks(4800) {
            enc.push(block, &mut out)?;
        }
        enc.flush(&mut out)?;
        let tb = enc.context().time_base();
        let packets = out
            .iter()
            .map(|p| av::EncodedPacket::from_av(av::StreamId::Audio(0), p, tb, crate::Timestamp(0)))
            .filter(|p| p.pts >= 0)
            .collect();
        let info = av::StreamInfo {
            id: av::StreamId::Audio(0),
            params: std::sync::Arc::new(enc.context().parameters()?),
            time_base: tb,
            title: "Mix".into(),
        };
        audio = Some((info, packets));
    }

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

/// Decodes one audio stream to interleaved stereo f32 at 48 kHz, placed by
/// timestamp from 0 (Lanes files are 48 kHz; others are not resampled).
fn decode_audio(src: &Path, idx: usize) -> Result<Vec<f32>> {
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
    let mut out: Vec<f32> = Vec::new();
    let mut pkt = Packet::alloc()?;
    let frame = Frame::alloc()?;
    let drain = |out: &mut Vec<f32>| -> Result<()> {
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
                    (out.len() / 2) as i64
                } else {
                    av::rescale(f.pts, tb, av::rational(1, 48_000))
                };
                if pts < 0 {
                    continue;
                }
                let at = pts as usize * 2;
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
                    out[at + i * 2] = sample(0);
                    out[at + i * 2 + 1] = sample(1);
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
        drain(&mut out)?;
    }
    // SAFETY: a null packet flushes the decoder.
    unsafe { ffi::avcodec_send_packet(dec.0, ptr::null()) };
    drain(&mut out)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

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
