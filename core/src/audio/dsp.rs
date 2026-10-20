//! Per-block audio processing: gain ramps, noise gate, noise suppression and
//! metering. All operate on interleaved stereo f32 blocks of 480 frames
//! (10 ms at 48 kHz).

use crate::util::{db_to_gain, gain_to_db};

pub const RATE: f32 = 48_000.0;

/// Gain that ramps linearly across a block when it changes, avoiding clicks.
#[derive(Clone, Debug)]
pub struct SmoothGain {
    current: f32,
}

impl SmoothGain {
    pub fn new(gain: f32) -> Self {
        Self { current: gain }
    }

    /// Multiplies `block` (interleaved stereo) by a ramp from the current gain to `target`.
    pub fn apply(&mut self, block: &mut [f32], target: f32) {
        let frames = block.len() / 2;
        if frames == 0 {
            return;
        }
        if (self.current - target).abs() < 1e-6 {
            if (target - 1.0).abs() > 1e-6 {
                block.iter_mut().for_each(|s| *s *= target);
            }
            return;
        }
        let step = (target - self.current) / frames as f32;
        for (i, f) in block.as_chunks_mut::<2>().0.iter_mut().enumerate() {
            let g = self.current + step * (i + 1) as f32;
            f[0] *= g;
            f[1] *= g;
        }
        self.current = target;
    }
}

/// Folds stereo to mono (both channels get the average).
pub fn to_mono(block: &mut [f32]) {
    for f in block.as_chunks_mut::<2>().0 {
        let m = (f[0] + f[1]) * 0.5;
        f[0] = m;
        f[1] = m;
    }
}

/// A simple downward gate with hysteresis, hold and smooth attack/release.
#[derive(Clone, Debug)]
pub struct NoiseGate {
    open: bool,
    hold_left: f32,
    gain: f32,
}

impl Default for NoiseGate {
    fn default() -> Self {
        Self { open: false, hold_left: 0.0, gain: 0.0 }
    }
}

impl NoiseGate {
    const HYSTERESIS_DB: f32 = 4.0;
    const HOLD_S: f32 = 0.15;
    const ATTACK_S: f32 = 0.005;
    const RELEASE_S: f32 = 0.12;

    pub fn process(&mut self, block: &mut [f32], threshold_db: f32) {
        let frames = block.len() / 2;
        let level = gain_to_db(rms(block));
        let dt = frames as f32 / RATE;
        if level >= threshold_db {
            self.open = true;
            self.hold_left = Self::HOLD_S;
        } else if level < threshold_db - Self::HYSTERESIS_DB {
            self.hold_left -= dt;
            if self.hold_left <= 0.0 {
                self.open = false;
            }
        }
        let target = if self.open { 1.0 } else { 0.0 };
        let time = if self.open { Self::ATTACK_S } else { Self::RELEASE_S };
        let per_frame = 1.0 / (time * RATE);
        for f in block.as_chunks_mut::<2>().0 {
            if self.gain < target {
                self.gain = (self.gain + per_frame).min(target);
            } else if self.gain > target {
                self.gain = (self.gain - per_frame).max(target);
            }
            f[0] *= self.gain;
            f[1] *= self.gain;
        }
    }
}

/// RNNoise-based noise suppression (via `nnnoiseless`). Works on mono 480-frame blocks.
pub struct NoiseSuppressor {
    state: Box<nnnoiseless::DenoiseState<'static>>,
    input: Vec<f32>,
    output: Vec<f32>,
}

impl Default for NoiseSuppressor {
    fn default() -> Self {
        Self {
            state: nnnoiseless::DenoiseState::new(),
            input: vec![0.0; nnnoiseless::DenoiseState::FRAME_SIZE],
            output: vec![0.0; nnnoiseless::DenoiseState::FRAME_SIZE],
        }
    }
}

impl NoiseSuppressor {
    /// Denoises an interleaved stereo block of exactly 480 frames (output is mono on both channels).
    pub fn process(&mut self, block: &mut [f32]) {
        let n = nnnoiseless::DenoiseState::FRAME_SIZE;
        if block.len() != n * 2 {
            return;
        }
        // nnnoiseless expects 16-bit sample scale.
        for (i, f) in block.as_chunks::<2>().0.iter().enumerate() {
            self.input[i] = (f[0] + f[1]) * 0.5 * 32767.0;
        }
        self.state.process_frame(&mut self.output, &self.input);
        for (i, f) in block.as_chunks_mut::<2>().0.iter_mut().enumerate() {
            let s = self.output[i] / 32767.0;
            f[0] = s;
            f[1] = s;
        }
    }
}

pub fn rms(block: &[f32]) -> f32 {
    if block.is_empty() {
        return 0.0;
    }
    (block.iter().map(|s| s * s).sum::<f32>() / block.len() as f32).sqrt()
}

pub fn peak(block: &[f32]) -> f32 {
    block.iter().fold(0.0f32, |m, s| m.max(s.abs()))
}

/// Soft limiter: transparent below about -1 dBFS, smoothly saturates above.
pub fn soft_clip(block: &mut [f32]) {
    const KNEE: f32 = 0.89;
    for s in block.iter_mut() {
        let a = s.abs();
        if a > KNEE {
            let over = (a - KNEE) / (1.0 - KNEE);
            *s = s.signum() * (KNEE + (1.0 - KNEE) * over.tanh());
        }
    }
}

/// Accumulates peak and RMS between UI updates.
#[derive(Clone, Debug, Default)]
pub struct Meter {
    peak: f32,
    sum_sq: f64,
    count: usize,
}

impl Meter {
    pub fn feed(&mut self, block: &[f32]) {
        self.peak = self.peak.max(peak(block));
        self.sum_sq += block.iter().map(|s| f64::from(s * s)).sum::<f64>();
        self.count += block.len();
    }

    /// Returns (peak dBFS, RMS dBFS) since the last call and resets.
    pub fn take(&mut self) -> (f32, f32) {
        let rms = if self.count == 0 { 0.0 } else { (self.sum_sq / self.count as f64).sqrt() as f32 };
        let out = (gain_to_db(self.peak), gain_to_db(rms));
        *self = Self::default();
        out
    }
}

pub fn gain_for(volume_db: f32, muted: bool) -> f32 {
    if muted { 0.0 } else { db_to_gain(volume_db) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(amplitude: f32) -> Vec<f32> {
        (0..480).flat_map(|i| {
            let s = (i as f32 * 0.1).sin() * amplitude;
            [s, s]
        }).collect()
    }

    #[test]
    fn smooth_gain_ramps_then_holds() {
        let mut g = SmoothGain::new(1.0);
        let mut b = vec![1.0; 960];
        g.apply(&mut b, 0.0);
        assert!(b[0] > 0.9 && b[959].abs() < 1e-6);
        let mut b = vec![1.0; 960];
        g.apply(&mut b, 0.0);
        assert!(b.iter().all(|s| *s == 0.0));
    }

    #[test]
    fn gate_closes_on_quiet_and_opens_on_loud() {
        let mut gate = NoiseGate::default();
        for _ in 0..50 {
            let mut quiet = tone(0.001); // ~ -63 dBFS
            gate.process(&mut quiet, -45.0);
        }
        let mut quiet = tone(0.001);
        gate.process(&mut quiet, -45.0);
        assert!(peak(&quiet) < 1e-6, "gate should be closed");
        let mut loud = tone(0.5);
        for _ in 0..3 {
            loud = tone(0.5);
            gate.process(&mut loud, -45.0);
        }
        assert!(peak(&loud) > 0.4, "gate should be open");
    }

    #[test]
    fn soft_clip_limits_and_is_transparent_below_knee() {
        let mut b = vec![0.5, -0.5, 2.0, -3.0];
        soft_clip(&mut b);
        assert_eq!(&b[..2], &[0.5, -0.5]);
        assert!(b[2] <= 1.0 && b[2] > 0.89);
        assert!(b[3] >= -1.0 && b[3] < -0.89);
    }

    #[test]
    fn meter_reports_db() {
        let mut m = Meter::default();
        m.feed(&vec![0.5; 960]);
        let (p, r) = m.take();
        assert!((p - (-6.02)).abs() < 0.1);
        assert!((r - (-6.02)).abs() < 0.1);
        assert_eq!(m.take().0, -100.0);
    }

    #[test]
    fn noise_suppressor_runs() {
        let mut ns = NoiseSuppressor::default();
        let mut b = tone(0.2);
        ns.process(&mut b);
        assert!(b.iter().all(|s| s.is_finite()));
    }
}
