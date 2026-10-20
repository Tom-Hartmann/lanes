//! Encoder selection and capability probing.

pub mod audio;
pub mod video;

use std::sync::OnceLock;

use serde::Serialize;

use crate::config::{EncoderBackend, VideoCodec};
use crate::gpu::{Gpu, GpuVendor};

/// FFmpeg encoder name for a backend/codec pair.
pub fn encoder_name(backend: EncoderBackend, codec: VideoCodec) -> Option<&'static str> {
    Some(match (backend, codec) {
        (EncoderBackend::Nvenc, VideoCodec::H264) => "h264_nvenc",
        (EncoderBackend::Nvenc, VideoCodec::Hevc) => "hevc_nvenc",
        (EncoderBackend::Nvenc, VideoCodec::Av1) => "av1_nvenc",
        (EncoderBackend::Amf, VideoCodec::H264) => "h264_amf",
        (EncoderBackend::Amf, VideoCodec::Hevc) => "hevc_amf",
        (EncoderBackend::Amf, VideoCodec::Av1) => "av1_amf",
        (EncoderBackend::Qsv, VideoCodec::H264) => "h264_qsv",
        (EncoderBackend::Qsv, VideoCodec::Hevc) => "hevc_qsv",
        (EncoderBackend::Qsv, VideoCodec::Av1) => "av1_qsv",
        (EncoderBackend::X264, VideoCodec::H264) => "libx264",
        _ => return None,
    })
}

pub fn backend_label(backend: EncoderBackend) -> &'static str {
    match backend {
        EncoderBackend::Auto => "Automatic",
        EncoderBackend::Nvenc => "NVIDIA NVENC",
        EncoderBackend::Amf => "AMD AMF",
        EncoderBackend::Qsv => "Intel Quick Sync",
        EncoderBackend::X264 => "x264 (CPU)",
    }
}

pub fn codec_label(codec: VideoCodec) -> &'static str {
    match codec {
        VideoCodec::H264 => "H.264",
        VideoCodec::Hevc => "HEVC",
        VideoCodec::Av1 => "AV1",
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EncoderInfo {
    pub backend: EncoderBackend,
    pub name: String,
    pub codecs: Vec<VideoCodec>,
    pub hardware: bool,
}

/// Encoders that actually open on this machine. Probed once (a few hundred
/// milliseconds) and cached; hardware encoders are only tried on GPUs from
/// the matching vendor.
pub fn available(gpu: &Gpu) -> &'static [EncoderInfo] {
    static CACHE: OnceLock<Vec<EncoderInfo>> = OnceLock::new();
    CACHE.get_or_init(|| probe(gpu))
}

fn probe(gpu: &Gpu) -> Vec<EncoderInfo> {
    let vendors: Vec<GpuVendor> = crate::gpu::adapters().into_iter().map(|a| a.vendor).collect();
    let mut out = Vec::new();
    for (backend, vendor) in
        [(EncoderBackend::Nvenc, GpuVendor::Nvidia), (EncoderBackend::Amf, GpuVendor::Amd), (EncoderBackend::Qsv, GpuVendor::Intel)]
    {
        // Only the capture GPU can encode zero-copy.
        if gpu.adapter.vendor != vendor {
            if vendors.contains(&vendor) {
                log::info!("{} is on a different GPU than capture; not used", backend_label(backend));
            }
            continue;
        }
        let codecs: Vec<VideoCodec> = [VideoCodec::H264, VideoCodec::Hevc, VideoCodec::Av1]
            .into_iter()
            .filter(|&c| video::VideoEncoder::probe(gpu, backend, c))
            .collect();
        if !codecs.is_empty() {
            out.push(EncoderInfo { backend, name: backend_label(backend).into(), codecs, hardware: true });
        }
    }
    if video::VideoEncoder::probe(gpu, EncoderBackend::X264, VideoCodec::H264) {
        out.push(EncoderInfo {
            backend: EncoderBackend::X264,
            name: backend_label(EncoderBackend::X264).into(),
            codecs: vec![VideoCodec::H264],
            hardware: false,
        });
    }
    log::info!(
        "encoders: {}",
        out.iter().map(|e| format!("{} {:?}", e.name, e.codecs)).collect::<Vec<_>>().join(", ")
    );
    out
}

/// Resolves `Auto` and unsupported choices to a working backend/codec pair:
/// the requested one if it opens, else the GPU's hardware encoder with the
/// requested codec, else hardware H.264, else x264.
pub fn resolve(available: &[EncoderInfo], backend: EncoderBackend, codec: VideoCodec) -> Option<(EncoderBackend, VideoCodec)> {
    let has = |b: EncoderBackend, c: VideoCodec| available.iter().any(|e| e.backend == b && e.codecs.contains(&c));
    if backend != EncoderBackend::Auto && has(backend, codec) {
        return Some((backend, codec));
    }
    let hw = available.iter().find(|e| e.hardware);
    if let Some(e) = hw {
        if e.codecs.contains(&codec) {
            return Some((e.backend, codec));
        }
        if e.codecs.contains(&VideoCodec::H264) {
            return Some((e.backend, VideoCodec::H264));
        }
    }
    has(EncoderBackend::X264, VideoCodec::H264).then_some((EncoderBackend::X264, VideoCodec::H264))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(b: EncoderBackend, codecs: &[VideoCodec], hw: bool) -> EncoderInfo {
        EncoderInfo { backend: b, name: String::new(), codecs: codecs.to_vec(), hardware: hw }
    }

    #[test]
    fn resolve_prefers_request_then_hardware_then_x264() {
        use EncoderBackend::*;
        use VideoCodec::*;
        let all = [info(Nvenc, &[H264, Hevc], true), info(X264, &[H264], false)];
        assert_eq!(resolve(&all, Auto, Hevc), Some((Nvenc, Hevc)));
        assert_eq!(resolve(&all, Nvenc, H264), Some((Nvenc, H264)));
        assert_eq!(resolve(&all, X264, H264), Some((X264, H264)));
        // AV1 unsupported -> hardware H.264.
        assert_eq!(resolve(&all, Auto, Av1), Some((Nvenc, H264)));
        // Requested AMF missing -> best available.
        assert_eq!(resolve(&all, Amf, H264), Some((Nvenc, H264)));
        let sw = [info(X264, &[H264], false)];
        assert_eq!(resolve(&sw, Auto, Hevc), Some((X264, H264)));
        assert_eq!(resolve(&[], Auto, H264), None);
    }

    #[test]
    fn names_exist_for_supported_pairs() {
        assert_eq!(encoder_name(EncoderBackend::X264, VideoCodec::Hevc), None);
        assert_eq!(encoder_name(EncoderBackend::Nvenc, VideoCodec::Av1), Some("av1_nvenc"));
    }
}
