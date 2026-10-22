//! Operations on finished files for the clips library: probing, thumbnails,
//! MKV â†’ MP4 remux, lossless keyframe-aligned trim and re-encoding for clip
//! quality presets. All run on background threads, never on capture paths.

use std::path::Path;
use std::ptr;

use crate::av::{self, CodecContext, CodecParams, Frame, Packet, cstr, ffi};
use crate::capture::thumbnail::{base64, bmp};
use crate::config::{EncoderBackend, VideoCodec};
use crate::error::{Error, Result, av_check};

/// An opened input file.
pub(crate) struct Input(pub(crate) *mut ffi::AVFormatContext);

impl Input {
    pub(crate) fn open(path: &Path) -> Result<Self> {
        let cpath = cstr(&path.to_string_lossy());
        let mut ctx = ptr::null_mut();
        // SAFETY: valid string and out-pointer; ownership moves into `Input`
        // only on success (FFmpeg frees ctx itself on failure).
        unsafe {
            av_check(ffi::avformat_open_input(&mut ctx, cpath.as_ptr(), ptr::null(), ptr::null_mut()), "avformat_open_input")?;
            let me = Self(ctx);
            av_check(ffi::avformat_find_stream_info(ctx, ptr::null_mut()), "avformat_find_stream_info")?;
            Ok(me)
        }
    }

    pub(crate) fn streams(&self) -> &[*mut ffi::AVStream] {
        // SAFETY: nb_streams entries are valid for the context's lifetime.
        unsafe { std::slice::from_raw_parts((*self.0).streams, (*self.0).nb_streams as usize) }
    }

    pub(crate) fn video_stream(&self) -> Option<usize> {
        // SAFETY: valid streams.
        self.streams().iter().position(|&s| unsafe { (*(*s).codecpar).codec_type == ffi::AVMediaType::AVMEDIA_TYPE_VIDEO })
    }

    /// Reads the next packet, or `None` at end of file.
    pub(crate) fn read(&self, pkt: &mut Packet) -> Result<Option<()>> {
        // SAFETY: valid context and packet; the previous payload is unref'd first.
        unsafe {
            ffi::av_packet_unref(pkt.0);
            let ret = ffi::av_read_frame(self.0, pkt.0);
            if ret == ffi::AVERROR_EOF {
                return Ok(None);
            }
            av_check(ret, "av_read_frame")?;
        }
        Ok(Some(()))
    }
}

impl Drop for Input {
    fn drop(&mut self) {
        // SAFETY: opened by avformat_open_input; closed once.
        unsafe { ffi::avformat_close_input(&mut self.0) };
    }
}

/// Titles of the audio tracks (lane names in Lanes files), in order.
pub fn audio_track_titles(path: &Path) -> Result<Vec<String>> {
    let input = Input::open(path)?;
    let mut out = Vec::new();
    for &st in input.streams() {
        // SAFETY: valid streams of the open input.
        unsafe {
            if (*(*st).codecpar).codec_type == ffi::AVMediaType::AVMEDIA_TYPE_AUDIO {
                let n = out.len() + 1;
                out.push(
                    dict_get((*st).metadata, "title")
                        .or_else(|| dict_get((*st).metadata, "handler_name"))
                        .filter(|t| !t.is_empty() && t != "SoundHandler")
                        .unwrap_or_else(|| format!("Track {n}")),
                );
            }
        }
    }
    Ok(out)
}

/// Duration in seconds.
pub fn duration(path: &Path) -> Option<f64> {
    let input = Input::open(path).ok()?;
    // SAFETY: valid context.
    let d = unsafe { (*input.0).duration };
    (d > 0).then(|| d as f64 / f64::from(ffi::AV_TIME_BASE))
}

/// Copies all streams into a new container (e.g. MKV â†’ MP4 with faststart).
pub fn remux(src: &Path, dst: &Path) -> Result<()> {
    copy_streams(src, dst, &CopyOptions::default())
}

/// Copies audio track `track` (0-based among the audio streams) alone into
/// `dst`, timed like the video of a remux so the two play in sync.
pub fn extract_audio(src: &Path, track: usize, dst: &Path) -> Result<()> {
    let idx = {
        let input = Input::open(src)?;
        input
            .streams()
            .iter()
            .enumerate()
            // SAFETY: valid streams of the open input.
            .filter(|(_, st)| unsafe { (*(***st).codecpar).codec_type == ffi::AVMediaType::AVMEDIA_TYPE_AUDIO })
            .nth(track)
            .map(|(i, _)| i)
            .ok_or_else(|| Error::Invalid(format!("no audio track {track}")))?
    };
    copy_streams(src, dst, &CopyOptions { only: Some(idx), ..Default::default() })
}

/// Lossless cut: starts at the keyframe at or before `start` and ends after
/// `end` (seconds). Returns the actual start used.
pub fn trim(src: &Path, dst: &Path, start: f64, end: f64) -> Result<()> {
    if end <= start {
        return Err(Error::Invalid("the end must be after the start".into()));
    }
    copy_streams(src, dst, &CopyOptions { range: Some((start, end)), ..Default::default() })
}

/// What `copy_streams` copies (default: every stream, whole file).
#[derive(Default)]
pub(crate) struct CopyOptions<'a> {
    /// Cut to this part (seconds); the start snaps back to a keyframe.
    pub range: Option<(f64, f64)>,
    /// Write just this input stream (timing still follows the video).
    pub only: Option<usize>,
    /// Leave out this input stream.
    pub skip: Option<usize>,
    /// A ready-made track, timed like the input, written as the first audio track.
    pub first_audio: Option<(&'a av::StreamInfo, &'a [av::EncodedPacket])>,
}

/// Copies streams into a new container without re-encoding; times start at
/// zero from the first video keyframe.
pub(crate) fn copy_streams(src: &Path, dst: &Path, opts: &CopyOptions<'_>) -> Result<()> {
    let input = Input::open(src)?;
    let video = input.video_stream();
    let mut infos = Vec::new();
    for (i, &st) in input.streams().iter().enumerate() {
        // SAFETY: valid stream; parameters are deep-copied.
        unsafe {
            let title = dict_get((*st).metadata, "title").or_else(|| dict_get((*st).metadata, "handler_name")).unwrap_or_default();
            infos.push(av::StreamInfo {
                id: if Some(i) == video { av::StreamId::Video(0) } else { av::StreamId::Audio(i as u8) },
                params: std::sync::Arc::new(CodecParams::from_raw_copy((*st).codecpar)?),
                time_base: (*st).time_base,
                title,
            });
        }
    }
    let tmp = dst.with_extension("partial");
    let container = crate::mux::Container::from_path(dst);
    let result = copy_packets(&input, video, &infos, opts, &tmp, container);
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result?;
    std::fs::rename(&tmp, dst)?;
    Ok(())
}

type ExtraTrack<'a> = (&'a av::StreamInfo, std::iter::Peekable<std::slice::Iter<'a, av::EncodedPacket>>);

/// Writes the extra track's packets up to `until` seconds (input timeline),
/// shifted like the copied streams and limited to the kept part.
fn write_extra(mux: &mut crate::mux::Muxer, extra: &mut Option<ExtraTrack<'_>>, until: f64, origin: f64, end: Option<f64>) -> Result<()> {
    let Some((info, packets)) = extra.as_mut() else { return Ok(()) };
    let tb = info.time_base;
    let offset = av::rescale((origin * 1_000_000.0) as i64, av::rational(1, 1_000_000), tb);
    while let Some(p) = packets.peek() {
        let secs = p.pts as f64 * f64::from(tb.num) / f64::from(tb.den);
        if secs > until {
            break;
        }
        if secs >= origin - 0.001 && end.is_none_or(|e| secs <= e) {
            mux.write(p, offset)?;
        }
        packets.next();
    }
    Ok(())
}

fn copy_packets(
    input: &Input,
    video: Option<usize>,
    infos: &[av::StreamInfo],
    opts: &CopyOptions<'_>,
    tmp: &Path,
    container: crate::mux::Container,
) -> Result<()> {
    let range = opts.range;
    // Streams to write, in file order; the extra track goes before the first audio track.
    let mut written: Vec<av::StreamInfo> = Vec::new();
    let mut placed = false;
    for (i, info) in infos.iter().enumerate() {
        if opts.only.is_some_and(|o| o != i) || opts.skip == Some(i) {
            continue;
        }
        if !placed
            && matches!(info.id, av::StreamId::Audio(_))
            && let Some((x, _)) = opts.first_audio
        {
            written.push(x.clone());
            placed = true;
        }
        written.push(info.clone());
    }
    if !placed && let Some((x, _)) = opts.first_audio {
        written.push(x.clone());
    }
    let mut extra: Option<ExtraTrack<'_>> = opts.first_audio.map(|(info, packets)| (info, packets.iter().peekable()));
    // The muxer skips packets of streams it was not given.
    let mut mux = crate::mux::Muxer::create(tmp, container, &written)?;
    // Last dts written per input stream; muxers like MP4 reject repeats.
    let mut last_dts: Vec<Option<i64>> = vec![None; infos.len()];

    if let (Some((start, _)), Some(v)) = (range, video) {
        let st = input.streams()[v];
        // SAFETY: valid stream; seek to the keyframe at or before `start`.
        unsafe {
            let ts = av::rescale((start * 1_000_000.0) as i64, av::rational(1, 1_000_000), (*st).time_base);
            ffi::av_seek_frame(input.0, v as i32, ts, ffi::AVSEEK_FLAG_BACKWARD);
        }
    }

    // Per-stream offset so the output starts at zero (set from the first video keyframe).
    let mut origin_secs: Option<f64> = None;
    let mut pkt = Packet::alloc()?;
    while input.read(&mut pkt)?.is_some() {
        let raw = pkt.raw();
        let idx = raw.stream_index as usize;
        let st = input.streams()[idx];
        // SAFETY: valid stream.
        let tb = unsafe { (*st).time_base };
        let pts = if raw.pts == ffi::AV_NOPTS_VALUE { raw.dts } else { raw.pts };
        let secs = pts as f64 * f64::from(tb.num) / f64::from(tb.den);
        if origin_secs.is_none() {
            if Some(idx) == video && raw.flags & ffi::AV_PKT_FLAG_KEY == 0 {
                continue;
            }
            if Some(idx) != video && video.is_some() {
                continue;
            }
            origin_secs = Some(secs);
        }
        let origin = origin_secs.unwrap_or(0.0);
        if secs < origin - 0.001 {
            continue;
        }
        if let Some((_, end)) = range
            && secs > end {
                if Some(idx) == video {
                    break;
                }
                continue;
            }
        let offset = av::rescale((origin * 1_000_000.0) as i64, av::rational(1, 1_000_000), tb);
        let mut dts = if raw.dts == ffi::AV_NOPTS_VALUE { pts } else { raw.dts };
        // Repair files with repeated timestamps (e.g. recorded by older builds
        // around a pause) instead of failing the whole remux.
        if let Some(prev) = last_dts[idx]
            && dts <= prev {
                dts = prev + 1;
            }
        last_dts[idx] = Some(dts);
        write_extra(&mut mux, &mut extra, secs, origin, range.map(|r| r.1))?;
        let enc = av::EncodedPacket {
            stream: infos[idx].id,
            data: pkt.data().into(),
            pts: pts.max(dts),
            dts,
            duration: raw.duration,
            keyframe: raw.flags & ffi::AV_PKT_FLAG_KEY != 0,
            time: crate::Timestamp(0),
        };
        mux.write(&enc, offset)?;
    }
    write_extra(&mut mux, &mut extra, f64::INFINITY, origin_secs.unwrap_or(0.0), range.map(|r| r.1))?;
    mux.finish()?;
    Ok(())
}

unsafe fn dict_get(dict: *mut ffi::AVDictionary, key: &str) -> Option<String> {
    let k = cstr(key);
    // SAFETY: the caller passes a valid (possibly null) dictionary.
    unsafe {
        let e = ffi::av_dict_get(dict, k.as_ptr(), ptr::null(), 0);
        (!e.is_null()).then(|| std::ffi::CStr::from_ptr((*e).value).to_string_lossy().into_owned())
    }
}

/// Decoder for the file's video stream (software).
pub(crate) fn open_decoder(input: &Input, v: usize) -> Result<CodecContext> {
    let st = input.streams()[v];
    // SAFETY: valid parameters; decoder context owned by CodecContext.
    unsafe {
        let codec = ffi::avcodec_find_decoder((*(*st).codecpar).codec_id);
        if codec.is_null() {
            return Err(Error::Unsupported("no decoder for this video".into()));
        }
        let ctx = ffi::avcodec_alloc_context3(codec);
        if ctx.is_null() {
            return Err(Error::Unsupported("avcodec_alloc_context3 failed".into()));
        }
        let ctx = CodecContext(ctx);
        av_check(ffi::avcodec_parameters_to_context(ctx.0, (*st).codecpar), "avcodec_parameters_to_context")?;
        (*ctx.0).thread_count = 2;
        ctx.open()?;
        Ok(ctx)
    }
}

/// Decodes frames of the video stream, calling `f` for each until it returns false.
pub(crate) fn decode_video(input: &Input, v: usize, dec: &CodecContext, mut f: impl FnMut(&Frame) -> Result<bool>) -> Result<()> {
    let mut pkt = Packet::alloc()?;
    let frame = Frame::alloc()?;
    let mut draining = false;
    loop {
        if !draining {
            match input.read(&mut pkt)? {
                Some(()) if pkt.raw().stream_index as usize != v => continue,
                // SAFETY: valid decoder and packet.
                Some(()) => unsafe {
                    let _ = ffi::avcodec_send_packet(dec.0, pkt.0);
                },
                None => {
                    draining = true;
                    // SAFETY: null packet starts draining.
                    unsafe { ffi::avcodec_send_packet(dec.0, ptr::null()) };
                }
            }
        }
        loop {
            // SAFETY: valid decoder and frame.
            let ret = unsafe { ffi::avcodec_receive_frame(dec.0, frame.0) };
            if ret == ffi::AVERROR(ffi::EAGAIN) {
                break;
            }
            if ret == ffi::AVERROR_EOF {
                return Ok(());
            }
            av_check(ret, "avcodec_receive_frame")?;
            if !f(&frame)? {
                return Ok(());
            }
        }
    }
}

/// A BMP data URL of the first frame, `width` pixels wide.
pub fn thumbnail(path: &Path, width: u32) -> Result<String> {
    let input = Input::open(path)?;
    let v = input.video_stream().ok_or_else(|| Error::Unsupported("no video stream".into()))?;
    let dec = open_decoder(&input, v)?;
    let mut result = None;
    decode_video(&input, v, &dec, |frame| {
        result = Some(frame_to_bgra(frame, width)?);
        Ok(false)
    })?;
    let (w, h, px) = result.ok_or_else(|| Error::Unsupported("no frame decoded".into()))?;
    Ok(format!("data:image/bmp;base64,{}", base64(&bmp(w, h, &px))))
}

/// Nearest-neighbour downscale + BT.709 limited-range YUV â†’ BGRA.
fn frame_to_bgra(frame: &Frame, width: u32) -> Result<(u32, u32, Vec<u8>)> {
    // SAFETY: plane pointers/linesizes describe the decoded frame; every read
    // index is bounded by the frame's width/height and subsampling.
    unsafe {
        let f = &*frame.0;
        let (sw, sh) = (f.width as u32, f.height as u32);
        if sw == 0 || sh == 0 {
            return Err(Error::Unsupported("empty frame".into()));
        }
        let w = width.min(sw).max(2);
        let h = ((u64::from(sh) * u64::from(w) / u64::from(sw)) as u32).max(2);
        let fmt = f.format;
        let nv12 = fmt == ffi::AVPixelFormat::AV_PIX_FMT_NV12 as i32;
        let planar420 = fmt == ffi::AVPixelFormat::AV_PIX_FMT_YUV420P as i32 || fmt == ffi::AVPixelFormat::AV_PIX_FMT_YUVJ420P as i32;
        if !nv12 && !planar420 {
            return Err(Error::Unsupported(format!("thumbnail: unsupported pixel format {fmt}")));
        }
        let mut out = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            let sy = (u64::from(y) * u64::from(sh) / u64::from(h)) as usize;
            for x in 0..w {
                let sx = (u64::from(x) * u64::from(sw) / u64::from(w)) as usize;
                let yv = f32::from(*f.data[0].add(sy * f.linesize[0] as usize + sx));
                let (u, v) = if nv12 {
                    let p = f.data[1].add((sy / 2) * f.linesize[1] as usize + (sx / 2) * 2);
                    (f32::from(*p), f32::from(*p.add(1)))
                } else {
                    (
                        f32::from(*f.data[1].add((sy / 2) * f.linesize[1] as usize + sx / 2)),
                        f32::from(*f.data[2].add((sy / 2) * f.linesize[2] as usize + sx / 2)),
                    )
                };
                let c = (yv - 16.0) * 1.164;
                let (d, e) = (u - 128.0, v - 128.0);
                let r = (c + 1.793 * e).clamp(0.0, 255.0) as u8;
                let g = (c - 0.213 * d - 0.533 * e).clamp(0.0, 255.0) as u8;
                let b = (c + 2.112 * d).clamp(0.0, 255.0) as u8;
                out.extend_from_slice(&[b, g, r, 255]);
            }
        }
        Ok((w, h, out))
    }
}

/// Re-encodes the video stream at `bitrate_kbps` (audio is copied). Used for
/// clip quality presets; runs on a low-priority background thread.
pub fn transcode(src: &Path, dst: &Path, backend: EncoderBackend, codec: VideoCodec, bitrate_kbps: u32) -> Result<()> {
    let input = Input::open(src)?;
    let v = input.video_stream().ok_or_else(|| Error::Unsupported("no video stream".into()))?;
    let dec = open_decoder(&input, v)?;
    // SAFETY: valid decoder context.
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
            c.gop_size = fps;
            c.max_b_frames = 0;
            c.bit_rate = i64::from(bitrate_kbps) * 1000;
            c.rc_max_rate = c.bit_rate * 3 / 2;
            c.rc_buffer_size = (c.bit_rate * 3) as i32;
            c.color_primaries = ffi::AVColorPrimaries::AVCOL_PRI_BT709;
            c.color_trc = ffi::AVColorTransferCharacteristic::AVCOL_TRC_BT709;
            c.colorspace = ffi::AVColorSpace::AVCOL_SPC_BT709;
            c.color_range = ffi::AVColorRange::AVCOL_RANGE_MPEG;
            c.flags |= ffi::AV_CODEC_FLAG_GLOBAL_HEADER as i32;
        }
        enc.open()?;
        Ok(enc)
    };
    // The session's hardware encoder (system-memory NV12 input), else x264.
    let name = crate::encode::encoder_name(backend, codec).unwrap_or("libx264");
    let (enc, used) = match open(name) {
        Ok(e) => (e, name),
        Err(e) if name != "libx264" => {
            log::warn!("{name} unavailable for re-encoding ({e}); using x264");
            (open("libx264")?, "libx264")
        }
        Err(e) => return Err(e),
    };
    log::info!("re-encoding {} with {used} at {bitrate_kbps} kbps", src.display());

    // Output: transcoded video + copied audio streams.
    let track_title = |st: *mut ffi::AVStream| {
        // SAFETY: valid stream of the open input.
        unsafe { dict_get((*st).metadata, "title").or_else(|| dict_get((*st).metadata, "handler_name")).unwrap_or_default() }
    };
    let mut infos = vec![av::StreamInfo {
        id: av::StreamId::Video(0),
        params: std::sync::Arc::new(enc.parameters()?),
        time_base: enc.time_base(),
        title: track_title(input.streams()[v]),
    }];
    for (i, &st) in input.streams().iter().enumerate() {
        if i == v {
            continue;
        }
        // SAFETY: valid stream.
        unsafe {
            infos.push(av::StreamInfo {
                id: av::StreamId::Audio(i as u8),
                params: std::sync::Arc::new(CodecParams::from_raw_copy((*st).codecpar)?),
                time_base: (*st).time_base,
                title: track_title(st),
            });
        }
    }
    let tmp = dst.with_extension("partial");
    let mut mux = crate::mux::Muxer::create(&tmp, crate::mux::Container::from_path(dst), &infos)?;

    // Pass 1: video.
    let mut n = 0i64;
    let mut out = Vec::new();
    let tb = enc.time_base();
    let write = |mux: &mut crate::mux::Muxer, out: &mut Vec<Packet>| -> Result<()> {
        for p in out.drain(..) {
            mux.write(&av::EncodedPacket::from_av(av::StreamId::Video(0), &p, tb, crate::Timestamp(0)), 0)?;
        }
        Ok(())
    };
    decode_video(&input, v, &dec, |frame| {
        let nv12 = to_nv12(frame)?;
        // SAFETY: owned frame.
        unsafe { (*nv12.0).pts = n };
        n += 1;
        enc.encode(Some(&nv12), &mut out)?;
        write(&mut mux, &mut out)?;
        Ok(true)
    })?;
    enc.encode(None, &mut out)?;
    write(&mut mux, &mut out)?;

    // Pass 2: audio, copied.
    drop(input);
    let input = Input::open(src)?;
    let mut pkt = Packet::alloc()?;
    while input.read(&mut pkt)?.is_some() {
        let raw = pkt.raw();
        let idx = raw.stream_index as usize;
        if idx == v {
            continue;
        }
        let e = av::EncodedPacket {
            stream: av::StreamId::Audio(idx as u8),
            data: pkt.data().into(),
            pts: raw.pts,
            dts: raw.dts,
            duration: raw.duration,
            keyframe: true,
            time: crate::Timestamp(0),
        };
        mux.write(&e, 0)?;
    }
    mux.finish()?;
    std::fs::rename(&tmp, dst)?;
    Ok(())
}

/// Converts a decoded 4:2:0 frame to NV12 (identity for NV12).
pub(crate) fn to_nv12(src: &Frame) -> Result<Frame> {
    let out = Frame::alloc()?;
    // SAFETY: the output buffer is allocated for NV12 at the source size;
    // copies are bounded by width/height and each plane's linesize.
    unsafe {
        let s = &*src.0;
        let o = &mut *out.0;
        o.format = ffi::AVPixelFormat::AV_PIX_FMT_NV12 as i32;
        o.width = s.width;
        o.height = s.height;
        av_check(ffi::av_frame_get_buffer(out.0, 0), "av_frame_get_buffer")?;
        let (w, h) = (s.width as usize, s.height as usize);
        for y in 0..h {
            ptr::copy_nonoverlapping(s.data[0].add(y * s.linesize[0] as usize), o.data[0].add(y * o.linesize[0] as usize), w);
        }
        let nv12 = s.format == ffi::AVPixelFormat::AV_PIX_FMT_NV12 as i32;
        for y in 0..h / 2 {
            let dst = o.data[1].add(y * o.linesize[1] as usize);
            if nv12 {
                ptr::copy_nonoverlapping(s.data[1].add(y * s.linesize[1] as usize), dst, w);
            } else {
                let (u, v) = (s.data[1].add(y * s.linesize[1] as usize), s.data[2].add(y * s.linesize[2] as usize));
                for x in 0..w / 2 {
                    *dst.add(2 * x) = *u.add(x);
                    *dst.add(2 * x + 1) = *v.add(x);
                }
            }
        }
    }
    Ok(out)
}
