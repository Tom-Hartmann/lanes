//! GPU colour conversion and scaling with the D3D11 video processor.
//!
//! Captured BGRA textures are scaled (aspect-preserving, letterboxed) and
//! converted to NV12 BT.709 limited range directly into the encoder's input
//! surface. This runs on the GPU's fixed-function video engine, so it costs
//! almost nothing on the 3D queue the game uses. Several layers can be
//! composed in one pass for the multi-monitor canvas.

use std::mem::ManuallyDrop;

use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BIND_RENDER_TARGET, D3D11_BIND_SHADER_RESOURCE, D3D11_TEX2D_ARRAY_VPOV, D3D11_TEX2D_VPIV, D3D11_TEX2D_VPOV,
    D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT, D3D11_VIDEO_COLOR, D3D11_VIDEO_COLOR_0, D3D11_VIDEO_COLOR_RGBA,
    D3D11_VIDEO_COLOR_YCbCrA, D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE, D3D11_VIDEO_PROCESSOR_CONTENT_DESC,
    D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC, D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC_0,
    D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC, D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC_0, D3D11_VIDEO_PROCESSOR_STREAM,
    D3D11_VIDEO_USAGE_OPTIMAL_SPEED, D3D11_VPIV_DIMENSION_TEXTURE2D, D3D11_VPOV_DIMENSION_TEXTURE2D,
    D3D11_VPOV_DIMENSION_TEXTURE2DARRAY, ID3D11Texture2D, ID3D11VideoContext1, ID3D11VideoDevice,
    ID3D11VideoProcessor, ID3D11VideoProcessorEnumerator, ID3D11VideoProcessorInputView,
    ID3D11VideoProcessorOutputView,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_COLOR_SPACE_RGB_FULL_G22_NONE_P709, DXGI_COLOR_SPACE_YCBCR_STUDIO_G22_LEFT_P709, DXGI_FORMAT_B8G8R8A8_UNORM,
    DXGI_RATIONAL, DXGI_SAMPLE_DESC,
};
use windows::core::Interface;

use crate::capture::Frame;
use crate::error::{Error, Result, WinContext};
use crate::gpu::Gpu;

/// A destination rectangle in output pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

impl Rect {
    pub fn full(w: u32, h: u32) -> Self {
        Self { x: 0, y: 0, w, h }
    }

    fn to_win(self) -> RECT {
        RECT { left: self.x as i32, top: self.y as i32, right: (self.x + self.w) as i32, bottom: (self.y + self.h) as i32 }
    }
}

/// Fits a `src_w`×`src_h` image into `area`, preserving aspect ratio and
/// centring it. Sizes are rounded to even numbers (required by NV12).
pub fn fit(src_w: u32, src_h: u32, area: Rect) -> Rect {
    if src_w == 0 || src_h == 0 || area.w == 0 || area.h == 0 {
        return area;
    }
    let (sw, sh, aw, ah) = (u64::from(src_w), u64::from(src_h), u64::from(area.w), u64::from(area.h));
    let (w, h) = if sw * ah >= sh * aw { (aw, (sh * aw / sw).max(2)) } else { ((sw * ah / sh).max(2), ah) };
    let (w, h) = ((w as u32) & !1, (h as u32) & !1);
    Rect { x: area.x + (((area.w - w) / 2) & !1), y: area.y + (((area.h - h) / 2) & !1), w, h }
}

/// Output size for a source scaled to `max_height` (keeping aspect, even dimensions).
pub fn scaled_size(src_w: u32, src_h: u32, max_height: Option<u32>) -> (u32, u32) {
    let (w, h) = match max_height {
        Some(mh) if mh < src_h && src_h > 0 => ((u64::from(src_w) * u64::from(mh) / u64::from(src_h)) as u32, mh),
        _ => (src_w, src_h),
    };
    ((w.max(2)) & !1, (h.max(2)) & !1)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputFormat {
    /// NV12, BT.709 limited range: encoder input.
    Nv12,
    /// BGRA full range: thumbnails.
    Bgra,
}

/// One input placed on the output.
pub struct Layer<'a> {
    pub frame: &'a Frame,
    pub dest: Rect,
}

pub struct Converter {
    gpu: Gpu,
    vdev: ID3D11VideoDevice,
    vctx: ID3D11VideoContext1,
    enumerator: ID3D11VideoProcessorEnumerator,
    processor: ID3D11VideoProcessor,
    format: OutputFormat,
    out_w: u32,
    out_h: u32,
    max_streams: u32,
    input_views: Vec<(usize, u32, ID3D11VideoProcessorInputView)>,
    output_views: Vec<(usize, u32, ID3D11VideoProcessorOutputView)>,
    /// Some drivers reject input views on capture textures; copy into this instead.
    staging_inputs: Vec<ID3D11Texture2D>,
    copy_inputs: bool,
}

// SAFETY: see `Gpu`; the converter is used by one encoder thread at a time.
unsafe impl Send for Converter {}

const VIEW_CACHE: usize = 8;

impl Converter {
    pub fn new(gpu: &Gpu, out_w: u32, out_h: u32, format: OutputFormat) -> Result<Self> {
        let vdev: ID3D11VideoDevice = gpu.device.cast().ctx("ID3D11VideoDevice")?;
        let vctx: ID3D11VideoContext1 = gpu.context.cast().ctx("ID3D11VideoContext1")?;
        let desc = D3D11_VIDEO_PROCESSOR_CONTENT_DESC {
            InputFrameFormat: D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
            InputFrameRate: DXGI_RATIONAL { Numerator: 60, Denominator: 1 },
            InputWidth: out_w.max(16),
            InputHeight: out_h.max(16),
            OutputFrameRate: DXGI_RATIONAL { Numerator: 60, Denominator: 1 },
            OutputWidth: out_w,
            OutputHeight: out_h,
            Usage: D3D11_VIDEO_USAGE_OPTIMAL_SPEED,
        };
        // SAFETY: valid descriptor; interfaces are owned by the returned struct.
        unsafe {
            let enumerator = vdev.CreateVideoProcessorEnumerator(&desc).ctx("CreateVideoProcessorEnumerator")?;
            let mut caps = Default::default();
            enumerator.GetVideoProcessorCaps(&mut caps).ctx("GetVideoProcessorCaps")?;
            let caps: windows::Win32::Graphics::Direct3D11::D3D11_VIDEO_PROCESSOR_CAPS = caps;
            let processor = vdev.CreateVideoProcessor(&enumerator, 0).ctx("CreateVideoProcessor")?;
            let out_space = match format {
                OutputFormat::Nv12 => DXGI_COLOR_SPACE_YCBCR_STUDIO_G22_LEFT_P709,
                OutputFormat::Bgra => DXGI_COLOR_SPACE_RGB_FULL_G22_NONE_P709,
            };
            vctx.VideoProcessorSetOutputColorSpace1(&processor, out_space);
            let black = match format {
                OutputFormat::Nv12 => D3D11_VIDEO_COLOR {
                    Anonymous: D3D11_VIDEO_COLOR_0 { YCbCr: D3D11_VIDEO_COLOR_YCbCrA { Y: 16.0 / 255.0, Cb: 0.5, Cr: 0.5, A: 1.0 } },
                },
                OutputFormat::Bgra => D3D11_VIDEO_COLOR {
                    Anonymous: D3D11_VIDEO_COLOR_0 { RGBA: D3D11_VIDEO_COLOR_RGBA { R: 0.0, G: 0.0, B: 0.0, A: 1.0 } },
                },
            };
            vctx.VideoProcessorSetOutputBackgroundColor(&processor, format == OutputFormat::Nv12, &black);
            let full = Rect::full(out_w, out_h).to_win();
            vctx.VideoProcessorSetOutputTargetRect(&processor, true, Some(&full));
            let max_streams = caps.MaxInputStreams.max(1);
            for i in 0..max_streams.min(8) {
                vctx.VideoProcessorSetStreamColorSpace1(&processor, i, DXGI_COLOR_SPACE_RGB_FULL_G22_NONE_P709);
                vctx.VideoProcessorSetStreamFrameFormat(&processor, i, D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE);
                vctx.VideoProcessorSetStreamAutoProcessingMode(&processor, i, false);
            }
            Ok(Self {
                gpu: gpu.clone(),
                vdev,
                vctx,
                enumerator,
                processor,
                format,
                out_w,
                out_h,
                max_streams,
                input_views: Vec::new(),
                output_views: Vec::new(),
                staging_inputs: Vec::new(),
                copy_inputs: false,
            })
        }
    }

    pub fn output_size(&self) -> (u32, u32) {
        (self.out_w, self.out_h)
    }

    pub fn max_layers(&self) -> u32 {
        self.max_streams
    }

    /// Draws `layers` onto `target` (array slice `index`); the rest is black.
    pub fn convert(&mut self, layers: &[Layer<'_>], target: &ID3D11Texture2D, index: u32) -> Result<()> {
        let layers = &layers[..layers.len().min(self.max_streams as usize)];
        let output = self.output_view(target, index)?;
        let mut streams = Vec::with_capacity(layers.len());
        for (i, layer) in layers.iter().enumerate() {
            let input = self.input_view(layer.frame, i)?;
            let src = Rect::full(layer.frame.width, layer.frame.height).to_win();
            let dst = fit(layer.frame.width, layer.frame.height, layer.dest).to_win();
            // SAFETY: rect pointers are valid for the duration of the call.
            unsafe {
                self.vctx.VideoProcessorSetStreamSourceRect(&self.processor, i as u32, true, Some(&src));
                self.vctx.VideoProcessorSetStreamDestRect(&self.processor, i as u32, true, Some(&dst));
            }
            streams.push(D3D11_VIDEO_PROCESSOR_STREAM {
                Enable: true.into(),
                pInputSurface: ManuallyDrop::new(Some(input)),
                ..Default::default()
            });
        }
        // SAFETY: streams and views are valid; the stream structs' COM
        // references are released manually right after.
        let result = unsafe { self.vctx.VideoProcessorBlt(&self.processor, &output, 0, &streams) };
        for s in &mut streams {
            // SAFETY: each pInputSurface was initialised above and is dropped once.
            unsafe { ManuallyDrop::drop(&mut s.pInputSurface) };
        }
        result.ctx("VideoProcessorBlt")
    }

    fn output_view(&mut self, target: &ID3D11Texture2D, index: u32) -> Result<ID3D11VideoProcessorOutputView> {
        let key = target.as_raw() as usize;
        if let Some((_, _, v)) = self.output_views.iter().find(|(k, i, _)| *k == key && *i == index) {
            return Ok(v.clone());
        }
        let mut tdesc = D3D11_TEXTURE2D_DESC::default();
        // SAFETY: valid out-pointer.
        unsafe { target.GetDesc(&mut tdesc) };
        let desc = if tdesc.ArraySize > 1 {
            D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC {
                ViewDimension: D3D11_VPOV_DIMENSION_TEXTURE2DARRAY,
                Anonymous: D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC_0 {
                    Texture2DArray: D3D11_TEX2D_ARRAY_VPOV { MipSlice: 0, FirstArraySlice: index, ArraySize: 1 },
                },
            }
        } else {
            D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC {
                ViewDimension: D3D11_VPOV_DIMENSION_TEXTURE2D,
                Anonymous: D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC_0 { Texture2D: D3D11_TEX2D_VPOV { MipSlice: 0 } },
            }
        };
        let mut view = None;
        // SAFETY: valid resource, enumerator, descriptor and out-pointer.
        unsafe { self.vdev.CreateVideoProcessorOutputView(target, &self.enumerator, &desc, Some(&mut view)) }
            .ctx("CreateVideoProcessorOutputView")?;
        let view = view.ok_or_else(|| Error::Unsupported("no output view".into()))?;
        if self.output_views.len() >= 64 {
            self.output_views.remove(0);
        }
        self.output_views.push((key, index, view.clone()));
        Ok(view)
    }

    fn input_view(&mut self, frame: &Frame, slot: usize) -> Result<ID3D11VideoProcessorInputView> {
        if !self.copy_inputs {
            let key = frame.texture.as_raw() as usize;
            if let Some((_, _, v)) = self.input_views.iter().find(|(k, i, _)| *k == key && *i == frame.array_index) {
                return Ok(v.clone());
            }
            match self.create_input_view(&frame.texture, frame.array_index) {
                Ok(v) => {
                    if self.input_views.len() >= VIEW_CACHE {
                        self.input_views.remove(0);
                    }
                    self.input_views.push((key, frame.array_index, v.clone()));
                    return Ok(v);
                }
                Err(e) => {
                    log::warn!("video processor rejected the capture texture ({e}); copying frames first");
                    self.copy_inputs = true;
                    self.input_views.clear();
                }
            }
        }
        // Fallback: copy into a texture we created with the right bind flags.
        let mut desc = D3D11_TEXTURE2D_DESC::default();
        // SAFETY: valid out-pointer.
        unsafe { frame.texture.GetDesc(&mut desc) };
        while self.staging_inputs.len() <= slot {
            self.staging_inputs.push(make_texture(&self.gpu, desc.Width, desc.Height)?);
        }
        let mut sdesc = D3D11_TEXTURE2D_DESC::default();
        // SAFETY: valid out-pointer.
        unsafe { self.staging_inputs[slot].GetDesc(&mut sdesc) };
        if sdesc.Width != desc.Width || sdesc.Height != desc.Height {
            self.staging_inputs[slot] = make_texture(&self.gpu, desc.Width, desc.Height)?;
        }
        let copy = self.staging_inputs[slot].clone();
        // SAFETY: same size and format; subresource index computed for mip level 0.
        unsafe { self.gpu.context.CopySubresourceRegion(&copy, 0, 0, 0, 0, &frame.texture, frame.array_index, None) };
        self.create_input_view(&copy, 0)
    }

    fn create_input_view(&self, texture: &ID3D11Texture2D, index: u32) -> Result<ID3D11VideoProcessorInputView> {
        let desc = D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC {
            FourCC: 0,
            ViewDimension: D3D11_VPIV_DIMENSION_TEXTURE2D,
            Anonymous: D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC_0 { Texture2D: D3D11_TEX2D_VPIV { MipSlice: 0, ArraySlice: index } },
        };
        let mut view = None;
        // SAFETY: valid resource, enumerator, descriptor and out-pointer.
        unsafe { self.vdev.CreateVideoProcessorInputView(texture, &self.enumerator, &desc, Some(&mut view)) }
            .ctx("CreateVideoProcessorInputView")?;
        view.ok_or_else(|| Error::Unsupported("no input view".into()))
    }

    pub fn format(&self) -> OutputFormat {
        self.format
    }
}

/// A BGRA texture usable as video processor input or output.
pub fn make_texture(gpu: &Gpu, width: u32, height: u32) -> Result<ID3D11Texture2D> {
    let desc = D3D11_TEXTURE2D_DESC {
        Width: width,
        Height: height,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_B8G8R8A8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: (D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_SHADER_RESOURCE.0) as u32,
        ..Default::default()
    };
    let mut tex = None;
    // SAFETY: valid descriptor and out-pointer.
    unsafe { gpu.device.CreateTexture2D(&desc, None, Some(&mut tex)) }.ctx("CreateTexture2D")?;
    tex.ok_or_else(|| Error::Unsupported("texture creation returned nothing".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_letterboxes_wider_sources() {
        // 21:9 into 16:9 -> bars top and bottom.
        let r = fit(3440, 1440, Rect::full(1920, 1080));
        assert_eq!(r.w, 1920);
        assert!(r.h < 1080 && r.h.is_multiple_of(2));
        assert_eq!(r.y, ((1080 - r.h) / 2) & !1);
    }

    #[test]
    fn fit_pillarboxes_taller_sources() {
        let r = fit(1080, 1920, Rect::full(1920, 1080));
        assert_eq!(r.h, 1080);
        assert!(r.w < 1920 && r.x > 0);
    }

    #[test]
    fn fit_same_aspect_fills() {
        assert_eq!(fit(2560, 1440, Rect::full(1920, 1080)), Rect::full(1920, 1080));
    }

    #[test]
    fn fit_respects_area_offset() {
        let r = fit(1920, 1080, Rect { x: 1920, y: 0, w: 1280, h: 720 });
        assert_eq!(r, Rect { x: 1920, y: 0, w: 1280, h: 720 });
    }

    #[test]
    fn scaled_size_keeps_aspect_and_even() {
        assert_eq!(scaled_size(2560, 1440, Some(720)), (1280, 720));
        assert_eq!(scaled_size(1920, 1080, None), (1920, 1080));
        assert_eq!(scaled_size(1920, 1080, Some(2160)), (1920, 1080));
        let (w, h) = scaled_size(1366, 768, Some(721));
        assert!(w.is_multiple_of(2) && h.is_multiple_of(2));
    }
}
