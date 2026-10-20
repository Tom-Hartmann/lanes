//! Video encoding. Hardware encoders read NV12 D3D11 surfaces straight from an
//! FFmpeg frames pool that the video processor renders into (zero-copy).
//! x264, and hardware encoders that reject D3D11 input, use a readback path:
//! the NV12 surface is copied to a staging texture and uploaded as a system
//! memory frame. That path is only a fallback.

use windows::Win32::Graphics::Direct3D11::{
    D3D11_BIND_RENDER_TARGET, D3D11_CPU_ACCESS_READ, D3D11_MAP_READ, D3D11_MAPPED_SUBRESOURCE, D3D11_TEXTURE2D_DESC,
    D3D11_USAGE_DEFAULT, D3D11_USAGE_STAGING, ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_NV12, DXGI_SAMPLE_DESC};

use crate::av::{self, BufferRef, CodecContext, Frame, Packet, ffi, rational};
use crate::config::{EncoderBackend, EncoderPreset, EncoderSettings, RateControl, VideoCodec};
use crate::error::{Error, Result, WinContext, av_check};
use crate::gpu::Gpu;

#[derive(Clone, Debug)]
pub struct VideoEncoderConfig {
    pub backend: EncoderBackend,
    pub codec: VideoCodec,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub settings: EncoderSettings,
}

/// Where the next frame must be rendered.
pub struct Surface {
    pub texture: ID3D11Texture2D,
    pub index: u32,
    hw_frame: Option<Frame>,
}

enum Input {
    /// Zero-copy D3D11 frames pool.
    Hardware { frames: BufferRef },
    /// GPU NV12 render target + staging readback.
    Readback { target: ID3D11Texture2D, staging: ID3D11Texture2D },
}

pub struct VideoEncoder {
    ctx: CodecContext,
    input: Input,
    gpu: Gpu,
    pub config: VideoEncoderConfig,
    _device: Option<BufferRef>,
}

// SAFETY: owned by one encoder thread; FFmpeg and D3D11 objects are not shared.
unsafe impl Send for VideoEncoder {}

impl VideoEncoder {
    /// Opens the encoder, trying the zero-copy path first.
    pub fn open(gpu: &Gpu, config: &VideoEncoderConfig) -> Result<Self> {
        if config.backend != EncoderBackend::X264 {
            match Self::open_hardware(gpu, config) {
                Ok(e) => return Ok(e),
                Err(e) if config.backend == EncoderBackend::Qsv => {
                    log::warn!("QSV rejected D3D11 input ({e}); using system-memory upload");
                }
                Err(e) => return Err(e),
            }
        }
        Self::open_readback(gpu, config)
    }

    /// True if the encoder opens with a small test configuration.
    pub fn probe(gpu: &Gpu, backend: EncoderBackend, codec: VideoCodec) -> bool {
        let config = VideoEncoderConfig {
            backend,
            codec,
            width: 1280,
            height: 720,
            fps: 30,
            settings: EncoderSettings { bitrate_kbps: 4000, max_bitrate_kbps: 6000, ..Default::default() },
        };
        Self::open(gpu, &config).is_ok()
    }

    fn open_hardware(gpu: &Gpu, config: &VideoEncoderConfig) -> Result<Self> {
        let name = super::encoder_name(config.backend, config.codec)
            .ok_or_else(|| Error::Unsupported(format!("{:?} cannot encode {:?}", config.backend, config.codec)))?;
        let device = av::d3d11_device(gpu)?;
        // Pool size 0: individual textures allocated on demand. A fixed-size pool
        // makes FFmpeg create one NV12 texture array, which drivers reject in
        // combination with the render-target flag the video processor needs.
        let frames = av::d3d11_frames(&device, config.width, config.height, 0)?;
        let ctx = CodecContext::encoder(name)?;
        configure(&ctx, config, ffi::AVPixelFormat::AV_PIX_FMT_D3D11);
        // SAFETY: the context takes its own reference to the frames pool.
        unsafe { (*ctx.as_ptr()).hw_frames_ctx = frames.new_ref() };
        ctx.open()?;
        log::info!("video encoder {name} opened ({}x{} @ {} fps, zero-copy)", config.width, config.height, config.fps);
        Ok(Self { ctx, input: Input::Hardware { frames }, gpu: gpu.clone(), config: config.clone(), _device: Some(device) })
    }

    fn open_readback(gpu: &Gpu, config: &VideoEncoderConfig) -> Result<Self> {
        let name = super::encoder_name(config.backend, config.codec)
            .ok_or_else(|| Error::Unsupported(format!("{:?} cannot encode {:?}", config.backend, config.codec)))?;
        let ctx = CodecContext::encoder(name)?;
        configure(&ctx, config, ffi::AVPixelFormat::AV_PIX_FMT_NV12);
        ctx.open()?;
        let target = nv12_texture(gpu, config.width, config.height, false)?;
        let staging = nv12_texture(gpu, config.width, config.height, true)?;
        log::info!("video encoder {name} opened ({}x{} @ {} fps, readback)", config.width, config.height, config.fps);
        Ok(Self { ctx, input: Input::Readback { target, staging }, gpu: gpu.clone(), config: config.clone(), _device: None })
    }

    pub fn is_zero_copy(&self) -> bool {
        matches!(self.input, Input::Hardware { .. })
    }

    /// A surface for the next frame. Render into `texture`/`index`, then `submit`.
    pub fn next_surface(&mut self) -> Result<Surface> {
        match &self.input {
            Input::Hardware { frames } => {
                let (frame, texture, index) = av::d3d11_get_surface(frames)?;
                Ok(Surface { texture, index, hw_frame: Some(frame) })
            }
            Input::Readback { target, .. } => Ok(Surface { texture: target.clone(), index: 0, hw_frame: None }),
        }
    }

    /// Encodes the rendered surface as frame number `pts` (in 1/fps units).
    pub fn submit(&mut self, surface: Surface, pts: i64, keyframe: bool, out: &mut Vec<Packet>) -> Result<()> {
        let frame = match surface.hw_frame {
            Some(f) => f,
            None => self.read_back()?,
        };
        // SAFETY: valid frame owned by us.
        unsafe {
            (*frame.0).pts = pts;
            (*frame.0).pict_type =
                if keyframe { ffi::AVPictureType::AV_PICTURE_TYPE_I } else { ffi::AVPictureType::AV_PICTURE_TYPE_NONE };
        }
        self.ctx.encode(Some(&frame), out)
    }

    pub fn flush(&mut self, out: &mut Vec<Packet>) -> Result<()> {
        self.ctx.encode(None, out)
    }

    pub fn context(&self) -> &CodecContext {
        &self.ctx
    }

    fn read_back(&self) -> Result<Frame> {
        let Input::Readback { target, staging } = &self.input else { unreachable!("hardware frames carry their own frame") };
        let (w, h) = (self.config.width as usize, self.config.height as usize);
        let frame = Frame::alloc()?;
        // SAFETY: the AVFrame buffer is allocated for NV12 w×h; the mapped
        // staging texture is read row by row within its pitch; Unmap always runs.
        unsafe {
            (*frame.0).format = ffi::AVPixelFormat::AV_PIX_FMT_NV12 as i32;
            (*frame.0).width = w as i32;
            (*frame.0).height = h as i32;
            av_check(ffi::av_frame_get_buffer(frame.0, 0), "av_frame_get_buffer")?;
            self.gpu.context.CopyResource(staging, target);
            let mut map = D3D11_MAPPED_SUBRESOURCE::default();
            self.gpu.context.Map(staging, 0, D3D11_MAP_READ, 0, Some(&mut map)).ctx("Map NV12")?;
            let src = map.pData as *const u8;
            let pitch = map.RowPitch as usize;
            // Y plane, then interleaved UV plane (half height) right after it.
            for y in 0..h {
                std::ptr::copy_nonoverlapping(src.add(y * pitch), (*frame.0).data[0].add(y * (*frame.0).linesize[0] as usize), w);
            }
            let uv = src.add(pitch * h);
            for y in 0..h / 2 {
                std::ptr::copy_nonoverlapping(uv.add(y * pitch), (*frame.0).data[1].add(y * (*frame.0).linesize[1] as usize), w);
            }
            self.gpu.context.Unmap(staging, 0);
        }
        Ok(frame)
    }
}

/// Applies size, timing, colour and rate-control settings before `open`.
fn configure(ctx: &CodecContext, config: &VideoEncoderConfig, pix_fmt: ffi::AVPixelFormat) {
    let s = &config.settings;
    let fps = config.fps.max(1) as i32;
    // SAFETY: the context is allocated and not yet opened, so fields may be set.
    unsafe {
        let c = &mut *ctx.as_ptr();
        c.width = config.width as i32;
        c.height = config.height as i32;
        c.time_base = rational(1, fps);
        c.framerate = rational(fps, 1);
        c.pix_fmt = pix_fmt;
        c.gop_size = ((s.keyframe_interval_secs * fps as f32).round() as i32).max(1);
        // No B-frames: lower latency and simpler, exact keyframe cuts.
        c.max_b_frames = 0;
        c.color_primaries = ffi::AVColorPrimaries::AVCOL_PRI_BT709;
        c.color_trc = ffi::AVColorTransferCharacteristic::AVCOL_TRC_BT709;
        c.colorspace = ffi::AVColorSpace::AVCOL_SPC_BT709;
        c.color_range = ffi::AVColorRange::AVCOL_RANGE_MPEG;
        c.flags |= ffi::AV_CODEC_FLAG_GLOBAL_HEADER as i32;
        let br = i64::from(s.bitrate_kbps.max(500)) * 1000;
        let max = i64::from(s.max_bitrate_kbps.max(s.bitrate_kbps)) * 1000;
        match s.rate_control {
            RateControl::Cbr => {
                c.bit_rate = br;
                c.rc_max_rate = br;
                c.rc_buffer_size = br as i32;
            }
            RateControl::Vbr => {
                c.bit_rate = br;
                c.rc_max_rate = max;
                c.rc_buffer_size = (max * 2).min(i64::from(i32::MAX)) as i32;
            }
            RateControl::Cqp => {
                c.bit_rate = 0;
            }
        }
    }

    let q = s.quality.min(51).to_string();
    match config.backend {
        EncoderBackend::Nvenc => {
            ctx.set_opt("preset", match s.preset {
                EncoderPreset::Performance => "p2",
                EncoderPreset::Balanced => "p4",
                EncoderPreset::Quality => "p6",
            });
            ctx.set_opt("tune", "hq");
            ctx.set_opt("rc", match s.rate_control {
                RateControl::Cbr => "cbr",
                RateControl::Vbr => "vbr",
                RateControl::Cqp => "constqp",
            });
            if s.rate_control == RateControl::Cqp {
                ctx.set_opt("qp", &q);
            }
            ctx.set_opt("multipass", "disabled");
            ctx.set_opt("rc-lookahead", "0");
            ctx.set_opt("delay", "0");
            ctx.set_opt("forced-idr", "1");
        }
        EncoderBackend::Amf => {
            ctx.set_opt("usage", "transcoding");
            ctx.set_opt("quality", match s.preset {
                EncoderPreset::Performance => "speed",
                EncoderPreset::Balanced => "balanced",
                EncoderPreset::Quality => "quality",
            });
            ctx.set_opt("rc", match s.rate_control {
                RateControl::Cbr => "cbr",
                RateControl::Vbr => "vbr_peak",
                RateControl::Cqp => "cqp",
            });
            if s.rate_control == RateControl::Cqp {
                ctx.set_opt("qp_i", &q);
                ctx.set_opt("qp_p", &q);
            }
        }
        EncoderBackend::Qsv => {
            ctx.set_opt("preset", match s.preset {
                EncoderPreset::Performance => "veryfast",
                EncoderPreset::Balanced => "medium",
                EncoderPreset::Quality => "slower",
            });
            if s.rate_control == RateControl::Cqp {
                // SAFETY: not yet opened; QSV reads global_quality with the QSCALE flag as CQP.
                unsafe {
                    let c = &mut *ctx.as_ptr();
                    c.flags |= ffi::AV_CODEC_FLAG_QSCALE as i32;
                    c.global_quality = i32::from(s.quality.min(51)) * ffi::FF_QP2LAMBDA;
                }
            }
        }
        EncoderBackend::X264 => {
            ctx.set_opt("preset", match s.preset {
                EncoderPreset::Performance => "superfast",
                EncoderPreset::Balanced => "veryfast",
                EncoderPreset::Quality => "faster",
            });
            match s.rate_control {
                RateControl::Cqp => ctx.set_opt("crf", &q),
                RateControl::Cbr => ctx.set_opt("nal-hrd", "cbr"),
                RateControl::Vbr => {}
            }
        }
        EncoderBackend::Auto => {}
    }
}

fn nv12_texture(gpu: &Gpu, width: u32, height: u32, staging: bool) -> Result<ID3D11Texture2D> {
    let desc = D3D11_TEXTURE2D_DESC {
        Width: width,
        Height: height,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_NV12,
        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        Usage: if staging { D3D11_USAGE_STAGING } else { D3D11_USAGE_DEFAULT },
        BindFlags: if staging { 0 } else { D3D11_BIND_RENDER_TARGET.0 as u32 },
        CPUAccessFlags: if staging { D3D11_CPU_ACCESS_READ.0 as u32 } else { 0 },
        ..Default::default()
    };
    let mut tex = None;
    // SAFETY: valid descriptor and out-pointer.
    unsafe { gpu.device.CreateTexture2D(&desc, None, Some(&mut tex)) }.ctx("CreateTexture2D NV12")?;
    tex.ok_or_else(|| Error::Unsupported("NV12 texture creation returned nothing".into()))
}
