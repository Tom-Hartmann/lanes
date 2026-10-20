//! Container writing (Matroska while recording, MP4 for finished files).
//!
//! MKV is used while recording because a truncated file (crash, power loss)
//! stays playable up to the last written cluster. Every audio lane is its own
//! track, titled with the lane name.

use std::path::{Path, PathBuf};
use std::ptr;

use crate::av::{self, EncodedPacket, Packet, StreamId, StreamInfo, cstr, ffi};
use crate::error::{Error, Result, av_check};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Container {
    Matroska,
    Mp4,
}

impl Container {
    pub fn extension(self) -> &'static str {
        match self {
            Container::Matroska => "mkv",
            Container::Mp4 => "mp4",
        }
    }

    fn format_name(self) -> &'static str {
        match self {
            Container::Matroska => "matroska",
            Container::Mp4 => "mp4",
        }
    }

    pub fn from_path(path: &Path) -> Self {
        match path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref() {
            Some("mp4" | "m4v" | "mov") => Container::Mp4,
            _ => Container::Matroska,
        }
    }
}

struct OutStream {
    id: StreamId,
    index: i32,
    src_tb: ffi::AVRational,
}

pub struct Muxer {
    ctx: *mut ffi::AVFormatContext,
    streams: Vec<OutStream>,
    path: PathBuf,
    header_written: bool,
    pub bytes: u64,
}

// SAFETY: a muxer is owned by one writer thread.
unsafe impl Send for Muxer {}

impl Muxer {
    /// Creates `path` with one track per stream, in order.
    pub fn create(path: &Path, container: Container, streams: &[StreamInfo]) -> Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let cpath = cstr(&path.to_string_lossy());
        let fmt = cstr(container.format_name());
        let mut ctx = ptr::null_mut();
        // SAFETY: valid strings and out-pointer; ownership of ctx is taken by
        // `me` immediately so every early return frees it.
        unsafe {
            av_check(
                ffi::avformat_alloc_output_context2(&mut ctx, ptr::null(), fmt.as_ptr(), cpath.as_ptr()),
                "avformat_alloc_output_context2",
            )?;
        }
        let mut me = Self { ctx, streams: Vec::new(), path: path.to_owned(), header_written: false, bytes: 0 };
        let mut first_audio = true;
        for info in streams {
            // SAFETY: ctx is valid; the new stream is owned by ctx.
            unsafe {
                let st = ffi::avformat_new_stream(ctx, ptr::null());
                if st.is_null() {
                    return Err(Error::Unsupported("avformat_new_stream failed".into()));
                }
                av_check(ffi::avcodec_parameters_copy((*st).codecpar, info.params.as_ptr()), "avcodec_parameters_copy")?;
                (*(*st).codecpar).codec_tag = 0;
                (*st).time_base = info.time_base;
                if !info.title.is_empty() {
                    // MKV reads "title"; MP4 stores track names as "handler_name".
                    for key in ["title", "handler_name"] {
                        let (k, v) = (cstr(key), cstr(&info.title));
                        ffi::av_dict_set(&mut (*st).metadata, k.as_ptr(), v.as_ptr(), 0);
                    }
                }
                if matches!(info.id, StreamId::Audio(_)) {
                    (*st).disposition = if first_audio { ffi::AV_DISPOSITION_DEFAULT } else { 0 };
                    first_audio = false;
                }
                me.streams.push(OutStream { id: info.id, index: (*st).index, src_tb: info.time_base });
            }
        }
        // SAFETY: ctx valid; pb is opened for writing and closed in Drop/finish.
        unsafe {
            if (*(*ctx).oformat).flags & ffi::AVFMT_NOFILE == 0 {
                av_check(ffi::avio_open(&mut (*ctx).pb, cpath.as_ptr(), ffi::AVIO_FLAG_WRITE), "avio_open")?;
            }
            let mut opts: *mut ffi::AVDictionary = ptr::null_mut();
            if container == Container::Mp4 {
                let (k, v) = (cstr("movflags"), cstr("+faststart"));
                ffi::av_dict_set(&mut opts, k.as_ptr(), v.as_ptr(), 0);
            }
            let ret = ffi::avformat_write_header(ctx, &mut opts);
            ffi::av_dict_free(&mut opts);
            av_check(ret, "avformat_write_header")?;
        }
        me.header_written = true;
        Ok(me)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn has_stream(&self, id: StreamId) -> bool {
        self.streams.iter().any(|s| s.id == id)
    }

    /// Writes `pkt` with its timestamps shifted back by `offset` (in the
    /// packet's own time base). Packets for streams not in this file are ignored.
    pub fn write(&mut self, pkt: &EncodedPacket, offset: i64) -> Result<()> {
        let Some(s) = self.streams.iter().find(|s| s.id == pkt.stream) else { return Ok(()) };
        let mut p = Packet::from_data(&pkt.data)?;
        // SAFETY: ctx and stream index are valid; the packet is consumed by the muxer.
        unsafe {
            let raw = p.raw_mut();
            raw.pts = pkt.pts - offset;
            raw.dts = pkt.dts - offset;
            raw.duration = pkt.duration;
            raw.stream_index = s.index;
            if pkt.keyframe {
                raw.flags |= ffi::AV_PKT_FLAG_KEY;
            }
            let st = *(*self.ctx).streams.add(s.index as usize);
            ffi::av_packet_rescale_ts(p.0, s.src_tb, (*st).time_base);
            av_check(ffi::av_interleaved_write_frame(self.ctx, p.0), "av_interleaved_write_frame")?;
        }
        self.bytes += pkt.data.len() as u64;
        Ok(())
    }

    /// Writes the trailer (index) and closes the file.
    pub fn finish(mut self) -> Result<PathBuf> {
        let r = self.close();
        r.map(|_| self.path.clone())
    }

    fn close(&mut self) -> Result<()> {
        if self.ctx.is_null() {
            return Ok(());
        }
        // SAFETY: ctx valid; trailer only after a successful header; pb closed once.
        let result = unsafe {
            let r = if self.header_written { ffi::av_write_trailer(self.ctx) } else { 0 };
            if !(*self.ctx).pb.is_null() && (*(*self.ctx).oformat).flags & ffi::AVFMT_NOFILE == 0 {
                ffi::avio_closep(&mut (*self.ctx).pb);
            }
            ffi::avformat_free_context(self.ctx);
            r
        };
        self.ctx = ptr::null_mut();
        av_check(result, "av_write_trailer").map(|_| ())
    }
}

impl Drop for Muxer {
    fn drop(&mut self) {
        if let Err(e) = self.close() {
            log::warn!("closing {} failed: {e}", self.path.display());
        }
    }
}

/// Writes a list of buffered packets (a clip) to a new file. Timestamps are
/// rebased so the file starts at zero at `start` (the first video keyframe),
/// and each video stream starts on its own first keyframe.
pub fn write_packets(path: &Path, container: Container, streams: &[StreamInfo], packets: &[av::SharedPacket], start: crate::Timestamp, session_start: crate::Timestamp) -> Result<f64> {
    let mut mux = Muxer::create(path, container, streams)?;
    let start_ticks = start.0 - session_start.0;
    let offsets: Vec<(StreamId, i64)> = streams
        .iter()
        .map(|s| (s.id, av::rescale(start_ticks, av::rational(1, crate::Timestamp::TICKS_PER_SEC as i32), s.time_base)))
        .collect();
    let mut started: Vec<StreamId> = Vec::new();
    let mut last = start;
    for pkt in packets {
        let Some(&(_, offset)) = offsets.iter().find(|(id, _)| *id == pkt.stream) else { continue };
        if !started.contains(&pkt.stream) {
            let is_video = matches!(pkt.stream, StreamId::Video(_));
            if (is_video && !pkt.keyframe) || pkt.pts < offset {
                continue;
            }
            started.push(pkt.stream);
        }
        mux.write(pkt, offset)?;
        last = last.max(pkt.time);
    }
    mux.finish()?;
    Ok(last.saturating_since(start).as_secs_f64())
}
