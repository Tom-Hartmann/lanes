//! Audio encoding (AAC or Opus), one encoder per lane. Input is interleaved
//! stereo f32 at 48 kHz in arbitrary block sizes; frames are cut to the
//! codec's frame size here.

use crate::av::{CodecContext, Frame, Packet, ffi, rational};
use crate::config::AudioCodec;
use crate::error::{Result, av_check};

pub const SAMPLE_RATE: u32 = 48_000;
pub const CHANNELS: usize = 2;

pub struct AudioEncoder {
    ctx: CodecContext,
    planar: bool,
    frame_size: usize,
    /// Interleaved samples waiting for a full frame.
    pending: Vec<f32>,
    /// Next frame's pts, in samples.
    next_pts: i64,
}

// SAFETY: owned by one audio encode thread.
unsafe impl Send for AudioEncoder {}

impl AudioEncoder {
    pub fn open(codec: AudioCodec, bitrate_kbps: u32) -> Result<Self> {
        let (name, fmt, planar) = match codec {
            AudioCodec::Aac => ("aac", ffi::AVSampleFormat::AV_SAMPLE_FMT_FLTP, true),
            AudioCodec::Opus => ("libopus", ffi::AVSampleFormat::AV_SAMPLE_FMT_FLT, false),
        };
        let ctx = CodecContext::encoder(name)?;
        // SAFETY: not yet opened, so fields may be set; ch_layout is initialised by FFmpeg.
        unsafe {
            let c = &mut *ctx.as_ptr();
            c.sample_rate = SAMPLE_RATE as i32;
            c.sample_fmt = fmt;
            ffi::av_channel_layout_default(&mut c.ch_layout, CHANNELS as i32);
            c.bit_rate = i64::from(bitrate_kbps.clamp(32, 512)) * 1000;
            c.time_base = rational(1, SAMPLE_RATE as i32);
            c.flags |= ffi::AV_CODEC_FLAG_GLOBAL_HEADER as i32;
        }
        ctx.open()?;
        let frame_size = match ctx.frame_size() {
            n if n > 0 => n as usize,
            _ => 1024,
        };
        Ok(Self { ctx, planar, frame_size, pending: Vec::with_capacity(frame_size * CHANNELS * 4), next_pts: 0 })
    }

    /// Queues interleaved stereo samples; encodes every complete frame.
    pub fn push(&mut self, interleaved: &[f32], out: &mut Vec<Packet>) -> Result<()> {
        self.pending.extend_from_slice(interleaved);
        let chunk = self.frame_size * CHANNELS;
        while self.pending.len() >= chunk {
            let frame = self.make_frame(self.frame_size)?;
            self.fill(&frame, chunk);
            self.pending.drain(..chunk);
            self.ctx.encode(Some(&frame), out)?;
        }
        Ok(())
    }

    /// Encodes any remainder (zero-padded) and drains the encoder.
    pub fn flush(&mut self, out: &mut Vec<Packet>) -> Result<()> {
        if !self.pending.is_empty() {
            let n = self.pending.len() / CHANNELS;
            self.pending.resize(self.frame_size * CHANNELS, 0.0);
            let frame = self.make_frame(self.frame_size)?;
            self.fill(&frame, self.frame_size * CHANNELS);
            self.pending.clear();
            let _ = n;
            self.ctx.encode(Some(&frame), out)?;
        }
        self.ctx.encode(None, out)
    }

    pub fn context(&self) -> &CodecContext {
        &self.ctx
    }

    fn make_frame(&mut self, samples: usize) -> Result<Frame> {
        let frame = Frame::alloc()?;
        // SAFETY: frame fields set before av_frame_get_buffer, as required.
        unsafe {
            let f = &mut *frame.0;
            let c = &*self.ctx.as_ptr();
            f.nb_samples = samples as i32;
            f.format = c.sample_fmt as i32;
            f.sample_rate = c.sample_rate;
            av_check(ffi::av_channel_layout_copy(&mut f.ch_layout, &c.ch_layout), "av_channel_layout_copy")?;
            av_check(ffi::av_frame_get_buffer(frame.0, 0), "av_frame_get_buffer audio")?;
            f.pts = self.next_pts;
        }
        self.next_pts += samples as i64;
        Ok(frame)
    }

    fn fill(&self, frame: &Frame, len: usize) {
        let src = &self.pending[..len];
        // SAFETY: the frame buffer holds `frame_size` samples per channel in
        // the codec's sample format (planar: one plane per channel).
        unsafe {
            let f = &*frame.0;
            if self.planar {
                let l = std::slice::from_raw_parts_mut(f.data[0] as *mut f32, len / CHANNELS);
                let r = std::slice::from_raw_parts_mut(f.data[1] as *mut f32, len / CHANNELS);
                for (i, s) in src.as_chunks::<CHANNELS>().0.iter().enumerate() {
                    l[i] = s[0];
                    r[i] = s[1];
                }
            } else {
                std::ptr::copy_nonoverlapping(src.as_ptr(), f.data[0] as *mut f32, len);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aac_encodes_a_tone() {
        let mut enc = AudioEncoder::open(AudioCodec::Aac, 128).expect("FFmpeg's native AAC encoder");
        let mut out = Vec::new();
        let block: Vec<f32> = (0..480).flat_map(|i| {
            let s = (i as f32 * 0.05).sin() * 0.3;
            [s, s]
        }).collect();
        for _ in 0..100 {
            enc.push(&block, &mut out).unwrap();
        }
        enc.flush(&mut out).unwrap();
        // 48000 samples / 1024 per frame ~= 47 frames (+ priming).
        assert!(out.len() >= 40, "{} packets", out.len());
        assert!(out.iter().all(|p| !p.data().is_empty()));
    }

    #[test]
    fn opus_encodes_when_available() {
        let Ok(mut enc) = AudioEncoder::open(AudioCodec::Opus, 96) else { return };
        let mut out = Vec::new();
        enc.push(&vec![0.0; 48_000 * 2], &mut out).unwrap();
        enc.flush(&mut out).unwrap();
        assert!(!out.is_empty());
    }
}
