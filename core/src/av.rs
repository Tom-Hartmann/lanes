//! Minimal RAII wrappers over the FFmpeg C API (via `ffmpeg_next::ffi`).
//!
//! The safe `ffmpeg-next` API does not cover hardware frame contexts, and the
//! encode path needs exact control over frames and packets, so this module
//! wraps the handful of raw types we use. Every `unsafe` FFmpeg call in the
//! crate goes through these types or carries its own `SAFETY` comment.

use std::ffi::{CString, c_void};
use std::ptr;
use std::sync::Arc;

pub use ffmpeg_next::ffi;
use ffi::{AVBufferRef, AVCodecContext, AVCodecParameters, AVFrame, AVPacket, AVRational};

use crate::error::{Error, Result, av_check};

/// Silences FFmpeg's own stderr logging except errors; call once at startup.
pub fn init_logging() {
    // SAFETY: plain setter.
    unsafe { ffi::av_log_set_level(ffi::AV_LOG_ERROR) };
}

pub const fn rational(num: i32, den: i32) -> AVRational {
    AVRational { num, den }
}

/// Rescales `v` from time base `from` to `to` (rounding to nearest).
pub fn rescale(v: i64, from: AVRational, to: AVRational) -> i64 {
    // SAFETY: pure arithmetic.
    unsafe { ffi::av_rescale_q(v, from, to) }
}

pub fn cstr(s: &str) -> CString {
    CString::new(s.replace('\0', "")).expect("NULs removed")
}

/// An owned `AVCodecContext`.
pub struct CodecContext(pub *mut AVCodecContext);

// SAFETY: a codec context is used by one thread at a time; we never share the pointer.
unsafe impl Send for CodecContext {}

impl CodecContext {
    /// Allocates a context for the encoder called `name` (e.g. "h264_nvenc").
    pub fn encoder(name: &str) -> Result<Self> {
        let cname = cstr(name);
        // SAFETY: valid C string; a null codec is handled.
        unsafe {
            let codec = ffi::avcodec_find_encoder_by_name(cname.as_ptr());
            if codec.is_null() {
                return Err(Error::Unsupported(format!("FFmpeg has no encoder \"{name}\"")));
            }
            let ctx = ffi::avcodec_alloc_context3(codec);
            if ctx.is_null() {
                return Err(Error::Unsupported("avcodec_alloc_context3 failed".into()));
            }
            Ok(Self(ctx))
        }
    }

    pub fn as_ptr(&self) -> *mut AVCodecContext {
        self.0
    }

    /// Sets a private (codec-specific) option. Unknown options are logged, not fatal.
    pub fn set_opt(&self, key: &str, value: &str) {
        let (k, v) = (cstr(key), cstr(value));
        // SAFETY: priv_data belongs to this context; strings are valid.
        let ret = unsafe { ffi::av_opt_set((*self.0).priv_data, k.as_ptr(), v.as_ptr(), ffi::AV_OPT_SEARCH_CHILDREN) };
        if ret < 0 {
            log::debug!("encoder option {key}={value} not accepted");
        }
    }

    pub fn open(&self) -> Result<()> {
        // SAFETY: context was allocated with a codec; no options dictionary.
        av_check(unsafe { ffi::avcodec_open2(self.0, ptr::null(), ptr::null_mut()) }, "avcodec_open2").map(|_| ())
    }

    /// Sends a frame (or `None` to flush) and collects every packet that is ready.
    pub fn encode(&self, frame: Option<&Frame>, out: &mut Vec<Packet>) -> Result<()> {
        // SAFETY: frame pointer is valid or null (flush).
        let ret = unsafe { ffi::avcodec_send_frame(self.0, frame.map_or(ptr::null(), |f| f.0)) };
        if ret < 0 && ret != ffi::AVERROR_EOF {
            av_check(ret, "avcodec_send_frame")?;
        }
        loop {
            let pkt = Packet::alloc()?;
            // SAFETY: valid context and packet.
            let ret = unsafe { ffi::avcodec_receive_packet(self.0, pkt.0) };
            if ret == ffi::AVERROR(ffi::EAGAIN) || ret == ffi::AVERROR_EOF {
                return Ok(());
            }
            av_check(ret, "avcodec_receive_packet")?;
            out.push(pkt);
        }
    }

    /// Snapshot of the stream parameters (codec id, size, extradata...) for muxers.
    pub fn parameters(&self) -> Result<CodecParams> {
        CodecParams::from_context(self.0)
    }

    pub fn time_base(&self) -> AVRational {
        // SAFETY: valid context.
        unsafe { (*self.0).time_base }
    }

    pub fn frame_size(&self) -> i32 {
        // SAFETY: valid context.
        unsafe { (*self.0).frame_size }
    }
}

impl Drop for CodecContext {
    fn drop(&mut self) {
        // SAFETY: allocated by avcodec_alloc_context3; freed once.
        unsafe { ffi::avcodec_free_context(&mut self.0) };
    }
}

/// An owned `AVFrame`.
pub struct Frame(pub *mut AVFrame);

// SAFETY: frames are moved between threads but never shared.
unsafe impl Send for Frame {}

impl Frame {
    pub fn alloc() -> Result<Self> {
        // SAFETY: plain allocation.
        let f = unsafe { ffi::av_frame_alloc() };
        if f.is_null() { Err(Error::Unsupported("av_frame_alloc failed".into())) } else { Ok(Self(f)) }
    }

    pub fn as_ptr(&self) -> *mut AVFrame {
        self.0
    }
}

impl Drop for Frame {
    fn drop(&mut self) {
        // SAFETY: allocated by av_frame_alloc; freed once.
        unsafe { ffi::av_frame_free(&mut self.0) };
    }
}

/// An owned `AVPacket`.
pub struct Packet(pub *mut AVPacket);

// SAFETY: packets are moved between threads but never shared.
unsafe impl Send for Packet {}

impl Packet {
    pub fn alloc() -> Result<Self> {
        // SAFETY: plain allocation.
        let p = unsafe { ffi::av_packet_alloc() };
        if p.is_null() { Err(Error::Unsupported("av_packet_alloc failed".into())) } else { Ok(Self(p)) }
    }

    pub fn data(&self) -> &[u8] {
        // SAFETY: data/size describe the packet's buffer (or are null/0).
        unsafe {
            let p = &*self.0;
            if p.data.is_null() || p.size <= 0 { &[] } else { std::slice::from_raw_parts(p.data, p.size as usize) }
        }
    }

    pub fn raw(&self) -> &AVPacket {
        // SAFETY: valid packet.
        unsafe { &*self.0 }
    }

    pub fn raw_mut(&mut self) -> &mut AVPacket {
        // SAFETY: valid packet, uniquely owned.
        unsafe { &mut *self.0 }
    }

    /// Builds a packet holding a copy of `data`.
    pub fn from_data(data: &[u8]) -> Result<Self> {
        let pkt = Self::alloc()?;
        // SAFETY: av_new_packet allocates `len` bytes (plus padding) we then fill.
        unsafe {
            av_check(ffi::av_new_packet(pkt.0, data.len() as i32), "av_new_packet")?;
            ptr::copy_nonoverlapping(data.as_ptr(), (*pkt.0).data, data.len());
        }
        Ok(pkt)
    }
}

impl Drop for Packet {
    fn drop(&mut self) {
        // SAFETY: allocated by av_packet_alloc; freed once.
        unsafe { ffi::av_packet_free(&mut self.0) };
    }
}

/// An owned `AVBufferRef` (hardware device/frames contexts).
pub struct BufferRef(pub *mut AVBufferRef);

// SAFETY: AVBufferRef reference counting is atomic; hw contexts are thread-safe
// to reference (FFmpeg locks the D3D11 device context itself).
unsafe impl Send for BufferRef {}
unsafe impl Sync for BufferRef {}

impl BufferRef {
    /// A new reference to the same buffer.
    pub fn new_ref(&self) -> *mut AVBufferRef {
        // SAFETY: valid buffer.
        unsafe { ffi::av_buffer_ref(self.0) }
    }
}

impl Clone for BufferRef {
    fn clone(&self) -> Self {
        Self(self.new_ref())
    }
}

impl Drop for BufferRef {
    fn drop(&mut self) {
        // SAFETY: owned reference, released once.
        unsafe { ffi::av_buffer_unref(&mut self.0) };
    }
}

/// Immutable codec parameters of an encoded stream, shared with muxers.
pub struct CodecParams(*mut AVCodecParameters);

// SAFETY: never mutated after creation; muxers only copy from it.
unsafe impl Send for CodecParams {}
unsafe impl Sync for CodecParams {}

impl CodecParams {
    fn from_context(ctx: *const AVCodecContext) -> Result<Self> {
        // SAFETY: allocation, then fill from a valid opened context.
        unsafe {
            let p = ffi::avcodec_parameters_alloc();
            if p.is_null() {
                return Err(Error::Unsupported("avcodec_parameters_alloc failed".into()));
            }
            let me = Self(p);
            av_check(ffi::avcodec_parameters_from_context(p, ctx), "avcodec_parameters_from_context")?;
            Ok(me)
        }
    }

    /// Deep-copies parameters owned by FFmpeg (e.g. a demuxer stream's codecpar).
    ///
    /// # Safety
    /// `src` must point to valid codec parameters for the duration of the call.
    pub unsafe fn from_raw_copy(src: *const AVCodecParameters) -> Result<Self> {
        // SAFETY: allocation, then a deep copy from a source the caller guarantees valid.
        unsafe {
            let p = ffi::avcodec_parameters_alloc();
            if p.is_null() {
                return Err(Error::Unsupported("avcodec_parameters_alloc failed".into()));
            }
            let me = Self(p);
            av_check(ffi::avcodec_parameters_copy(p, src), "avcodec_parameters_copy")?;
            Ok(me)
        }
    }

    pub fn as_ptr(&self) -> *const AVCodecParameters {
        self.0
    }

    pub fn is_video(&self) -> bool {
        // SAFETY: valid parameters.
        unsafe { (*self.0).codec_type == ffi::AVMediaType::AVMEDIA_TYPE_VIDEO }
    }
}

impl Drop for CodecParams {
    fn drop(&mut self) {
        // SAFETY: allocated by avcodec_parameters_alloc; freed once.
        unsafe { ffi::avcodec_parameters_free(&mut self.0) };
    }
}

// ------------------------------------------------------------ D3D11 hwctx ---

/// `AVD3D11VADeviceContext` from libavutil/hwcontext_d3d11va.h (not in the
/// generated bindings because that header pulls in d3d11.h).
#[repr(C)]
struct AVD3D11VADeviceContext {
    device: *mut c_void,
    device_context: *mut c_void,
    video_device: *mut c_void,
    video_context: *mut c_void,
    lock: Option<unsafe extern "C" fn(*mut c_void)>,
    unlock: Option<unsafe extern "C" fn(*mut c_void)>,
    lock_ctx: *mut c_void,
}

/// `AVD3D11VAFramesContext` from libavutil/hwcontext_d3d11va.h.
#[repr(C)]
struct AVD3D11VAFramesContext {
    texture: *mut c_void,
    bind_flags: u32,
    misc_flags: u32,
    texture_infos: *mut c_void,
}

/// An FFmpeg D3D11VA device wrapping our own `ID3D11Device`, so encoders read
/// the textures we render into without any copy.
pub fn d3d11_device(gpu: &crate::gpu::Gpu) -> Result<BufferRef> {
    use windows::core::Interface;
    // SAFETY: allocates the hw device context and fills the D3D11-specific
    // struct as documented: FFmpeg takes ownership of one reference to each
    // interface, so we hand it AddRef'ed raw pointers (`into_raw` on clones).
    unsafe {
        let buf = ffi::av_hwdevice_ctx_alloc(ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_D3D11VA);
        if buf.is_null() {
            return Err(Error::Unsupported("av_hwdevice_ctx_alloc(D3D11VA) failed".into()));
        }
        let buf = BufferRef(buf);
        let hwctx = (*buf.0).data as *mut ffi::AVHWDeviceContext;
        let d3d = (*hwctx).hwctx as *mut AVD3D11VADeviceContext;
        (*d3d).device = gpu.device.clone().into_raw();
        (*d3d).device_context = gpu.context.clone().into_raw();
        av_check(ffi::av_hwdevice_ctx_init(buf.0), "av_hwdevice_ctx_init")?;
        Ok(buf)
    }
}

/// A pool of NV12 D3D11 textures the video processor can render into.
pub fn d3d11_frames(device: &BufferRef, width: u32, height: u32, pool_size: i32) -> Result<BufferRef> {
    // D3D11_BIND_RENDER_TARGET
    const BIND_RENDER_TARGET: u32 = 0x20;
    // SAFETY: allocation from a valid device; fields set before init as required.
    unsafe {
        let buf = ffi::av_hwframe_ctx_alloc(device.0);
        if buf.is_null() {
            return Err(Error::Unsupported("av_hwframe_ctx_alloc failed".into()));
        }
        let buf = BufferRef(buf);
        let frames = (*buf.0).data as *mut ffi::AVHWFramesContext;
        (*frames).format = ffi::AVPixelFormat::AV_PIX_FMT_D3D11;
        (*frames).sw_format = ffi::AVPixelFormat::AV_PIX_FMT_NV12;
        (*frames).width = width as i32;
        (*frames).height = height as i32;
        (*frames).initial_pool_size = pool_size;
        let d3d = (*frames).hwctx as *mut AVD3D11VAFramesContext;
        (*d3d).bind_flags = BIND_RENDER_TARGET;
        av_check(ffi::av_hwframe_ctx_init(buf.0), "av_hwframe_ctx_init")?;
        Ok(buf)
    }
}

/// Takes a free surface from a D3D11 frames pool. Returns the frame and its
/// texture + array slice to render into.
pub fn d3d11_get_surface(frames: &BufferRef) -> Result<(Frame, windows::Win32::Graphics::Direct3D11::ID3D11Texture2D, u32)> {
    use windows::Win32::Graphics::Direct3D11::ID3D11Texture2D;
    use windows::core::Interface;
    let frame = Frame::alloc()?;
    // SAFETY: valid pool; for AV_PIX_FMT_D3D11 data[0] is an ID3D11Texture2D*
    // owned by the pool and data[1] the array index (documented in pixfmt.h).
    unsafe {
        av_check(ffi::av_hwframe_get_buffer(frames.0, frame.0, 0), "av_hwframe_get_buffer")?;
        let raw = (*frame.0).data[0] as *mut c_void;
        let index = (*frame.0).data[1] as usize as u32;
        let texture = ID3D11Texture2D::from_raw_borrowed(&raw)
            .ok_or_else(|| Error::Unsupported("hw frame without texture".into()))?
            .clone();
        Ok((frame, texture, index))
    }
}

/// A shared, immutable encoded packet, cheap to fan out to the recorder and the replay buffer.
pub type SharedPacket = Arc<EncodedPacket>;

/// Which output stream a packet belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum StreamId {
    Video(u8),
    /// Audio lane by index in the session's lane list.
    Audio(u8),
}

/// An encoded packet copied out of FFmpeg.
pub struct EncodedPacket {
    pub stream: StreamId,
    pub data: Box<[u8]>,
    /// In the stream's time base.
    pub pts: i64,
    pub dts: i64,
    pub duration: i64,
    pub keyframe: bool,
    /// Presentation time on the shared clock, used to cut clips.
    pub time: crate::Timestamp,
}

impl EncodedPacket {
    pub fn from_av(stream: StreamId, pkt: &Packet, time_base: AVRational, session_start: crate::Timestamp) -> Self {
        let raw = pkt.raw();
        let ticks = rescale(raw.pts, time_base, rational(1, crate::Timestamp::TICKS_PER_SEC as i32));
        Self {
            stream,
            data: pkt.data().into(),
            pts: raw.pts,
            dts: raw.dts,
            duration: raw.duration,
            keyframe: raw.flags & ffi::AV_PKT_FLAG_KEY != 0,
            time: crate::Timestamp(session_start.0 + ticks),
        }
    }
}

/// Description of one output stream, enough to create muxer streams.
#[derive(Clone)]
pub struct StreamInfo {
    pub id: StreamId,
    pub params: Arc<CodecParams>,
    pub time_base: AVRational,
    /// Track title (lane name / monitor name).
    pub title: String,
}
