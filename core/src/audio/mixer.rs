//! The lane mixer. Sources deliver chunks stamped with QPC time; the mixer
//! places them on a sample-accurate timeline anchored at the session start
//! (the same anchor video uses), mixes every 10 ms block into the lanes each
//! source feeds, and applies per-source and per-lane processing.
//!
//! Pure logic with no threads or OS calls, so it is fully unit-tested.

use std::collections::{HashMap, VecDeque};

use super::dsp::{self, Meter, NoiseGate, NoiseSuppressor, SmoothGain};
use crate::Timestamp;
use crate::config::{AudioLayout, Id};

pub const BLOCK_FRAMES: usize = 480;
const RATE: i64 = 48_000;
/// Timestamp jitter tolerated before a source is re-synchronised.
const RESYNC_FRAMES: i64 = 960;
/// Never buffer more than this per source (protects against a stalled mixer).
const MAX_BUFFER_FRAMES: usize = 48_000 * 2;

/// Timestamp → sample index relative to `t0`.
fn frame_index(t0: Timestamp, t: Timestamp) -> i64 {
    let ticks = t.0 - t0.0;
    // Round to nearest sample.
    (ticks * RATE + Timestamp::TICKS_PER_SEC / 2).div_euclid(Timestamp::TICKS_PER_SEC)
}

/// Samples of one source laid out on the timeline.
#[derive(Default)]
struct Timeline {
    /// Frame index of `data[0]`.
    start: i64,
    /// Interleaved stereo.
    data: VecDeque<f32>,
    synced: bool,
}

impl Timeline {
    fn end(&self) -> i64 {
        self.start + (self.data.len() / 2) as i64
    }

    fn write(&mut self, at: i64, samples: &[f32]) {
        if !self.synced {
            self.start = at;
            self.data.clear();
            self.synced = true;
        }
        let end = self.end();
        let drift = at - end;
        if drift.abs() <= RESYNC_FRAMES {
            // Contiguous within jitter: append as-is so small timestamp noise
            // never produces clicks.
            self.data.extend(samples.iter().copied());
        } else if drift > 0 {
            // A real gap (e.g. loopback with nothing playing): fill with silence.
            self.data.extend(std::iter::repeat_n(0.0, drift as usize * 2));
            self.data.extend(samples.iter().copied());
        } else {
            // Time went backwards (device reset): restart the timeline here.
            self.start = at;
            self.data.clear();
            self.data.extend(samples.iter().copied());
        }
        let max = MAX_BUFFER_FRAMES * 2;
        if self.data.len() > max {
            let drop = self.data.len() - max;
            self.data.drain(..drop);
            self.start += (drop / 2) as i64;
        }
    }

    /// Copies `[at, at + frames)` into `out` (silence where missing) and
    /// discards everything before `at + frames`.
    fn read(&mut self, at: i64, out: &mut [f32]) {
        let frames = (out.len() / 2) as i64;
        out.fill(0.0);
        for i in 0..frames {
            let idx = at + i - self.start;
            if idx >= 0 && ((idx * 2 + 1) as usize) < self.data.len() {
                out[(i * 2) as usize] = self.data[(idx * 2) as usize];
                out[(i * 2 + 1) as usize] = self.data[(idx * 2 + 1) as usize];
            }
        }
        let consumed = (at + frames - self.start).clamp(0, (self.data.len() / 2) as i64);
        self.data.drain(..(consumed * 2) as usize);
        self.start += consumed;
    }
}

struct MixSource {
    id: Id,
    timeline: Timeline,
    gain: SmoothGain,
    volume: f32,
    mono: bool,
    gate: Option<(NoiseGate, f32)>,
    suppressor: Option<NoiseSuppressor>,
    lanes: Vec<usize>,
    meter: Meter,
}

struct MixLane {
    id: Id,
    gain: SmoothGain,
    volume: f32,
    mono: bool,
    meter: Meter,
}

/// Level readings for the UI.
#[derive(Clone, Debug, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeterFrame {
    pub lanes: Vec<LaneMeter>,
    pub sources: Vec<SourceMeter>,
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LaneMeter {
    pub id: Id,
    pub peak_db: f32,
    pub rms_db: f32,
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceMeter {
    pub id: Id,
    pub peak_db: f32,
}

pub struct Mixer {
    t0: Timestamp,
    /// Frame index of the next block to mix.
    cursor: i64,
    sources: Vec<MixSource>,
    lanes: Vec<MixLane>,
    scratch: Vec<f32>,
}

impl Mixer {
    /// Lanes are fixed for the session (they are file tracks); sources can change live.
    pub fn new(t0: Timestamp, layout: &AudioLayout) -> Self {
        let lanes = layout
            .lanes
            .iter()
            .map(|l| MixLane {
                id: l.id.clone(),
                gain: SmoothGain::new(dsp::gain_for(l.volume_db, l.muted)),
                volume: dsp::gain_for(l.volume_db, l.muted),
                mono: l.mono,
                meter: Meter::default(),
            })
            .collect();
        let mut m = Self { t0, cursor: 0, sources: Vec::new(), lanes, scratch: vec![0.0; BLOCK_FRAMES * 2] };
        m.update(layout);
        m
    }

    pub fn lane_ids(&self) -> Vec<Id> {
        self.lanes.iter().map(|l| l.id.clone()).collect()
    }

    /// Applies volumes, mutes, processing and routing live. New sources start
    /// with an empty timeline; removed sources are dropped. Lane additions or
    /// removals are ignored until the next session.
    pub fn update(&mut self, layout: &AudioLayout) {
        for lane in &mut self.lanes {
            if let Some(cfg) = layout.lanes.iter().find(|l| l.id == lane.id) {
                lane.volume = dsp::gain_for(cfg.volume_db, cfg.muted);
                lane.mono = cfg.mono;
            }
        }
        let mut old: HashMap<Id, MixSource> = self.sources.drain(..).map(|s| (s.id.clone(), s)).collect();
        for cfg in &layout.sources {
            let mut src = old.remove(&cfg.id).unwrap_or_else(|| MixSource {
                id: cfg.id.clone(),
                timeline: Timeline::default(),
                gain: SmoothGain::new(dsp::gain_for(cfg.volume_db, cfg.muted)),
                volume: 1.0,
                mono: false,
                gate: None,
                suppressor: None,
                lanes: Vec::new(),
                meter: Meter::default(),
            });
            src.volume = dsp::gain_for(cfg.volume_db, cfg.muted);
            src.mono = cfg.mono;
            src.gate = cfg.noise_gate.as_ref().map(|g| (src.gate.take().map(|(st, _)| st).unwrap_or_default(), g.threshold_db));
            src.suppressor = if cfg.noise_suppression { src.suppressor.take().or_else(|| Some(NoiseSuppressor::default())) } else { None };
            src.lanes = cfg.lanes.iter().filter_map(|id| self.lanes.iter().position(|l| &l.id == id)).collect();
            self.sources.push(src);
        }
    }

    /// Adds captured samples (interleaved stereo, 48 kHz) whose first frame was captured at `time`.
    pub fn push(&mut self, source: &str, time: Timestamp, samples: &[f32]) {
        let at = frame_index(self.t0, time);
        if let Some(s) = self.sources.iter_mut().find(|s| s.id == source) {
            s.timeline.write(at, samples);
        }
    }

    /// Time at which the next block ends; it can be mixed once all sources had a chance to deliver it.
    pub fn next_block_end(&self) -> Timestamp {
        let end = self.cursor + BLOCK_FRAMES as i64;
        Timestamp(self.t0.0 + end * Timestamp::TICKS_PER_SEC / RATE)
    }

    /// Mixes the next 10 ms block into `out[lane]` (resized as needed).
    pub fn mix(&mut self, out: &mut [Vec<f32>]) {
        for buf in out.iter_mut() {
            buf.clear();
            buf.resize(BLOCK_FRAMES * 2, 0.0);
        }
        for src in &mut self.sources {
            src.timeline.read(self.cursor, &mut self.scratch);
            if let Some(ns) = &mut src.suppressor {
                ns.process(&mut self.scratch);
            }
            if src.mono {
                dsp::to_mono(&mut self.scratch);
            }
            if let Some((gate, threshold)) = &mut src.gate {
                gate.process(&mut self.scratch, *threshold);
            }
            src.gain.apply(&mut self.scratch, src.volume);
            src.meter.feed(&self.scratch);
            for &lane in &src.lanes {
                if let Some(buf) = out.get_mut(lane) {
                    for (o, s) in buf.iter_mut().zip(&self.scratch) {
                        *o += s;
                    }
                }
            }
        }
        for (lane, buf) in self.lanes.iter_mut().zip(out.iter_mut()) {
            if lane.mono {
                dsp::to_mono(buf);
            }
            lane.gain.apply(buf, lane.volume);
            dsp::soft_clip(buf);
            lane.meter.feed(buf);
        }
        self.cursor += BLOCK_FRAMES as i64;
    }

    /// Frame index of the next block (samples since the session start).
    pub fn cursor(&self) -> i64 {
        self.cursor
    }

    /// Meter readings since the last call.
    pub fn take_meters(&mut self) -> MeterFrame {
        MeterFrame {
            lanes: self
                .lanes
                .iter_mut()
                .map(|l| {
                    let (peak_db, rms_db) = l.meter.take();
                    LaneMeter { id: l.id.clone(), peak_db, rms_db }
                })
                .collect(),
            sources: self.sources.iter_mut().map(|s| SourceMeter { id: s.id.clone(), peak_db: s.meter.take().0 }).collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AudioSource, Lane, SourceKind};

    fn layout(sources: &[(&str, &[&str], f32)], lanes: &[&str]) -> AudioLayout {
        AudioLayout {
            lanes: lanes.iter().map(|id| Lane { id: (*id).into(), name: (*id).into(), volume_db: 0.0, muted: false, mono: false }).collect(),
            sources: sources
                .iter()
                .map(|(id, lanes, vol)| AudioSource {
                    id: (*id).into(),
                    name: (*id).into(),
                    kind: SourceKind::SystemOutput { device: None },
                    volume_db: *vol,
                    muted: false,
                    mono: false,
                    noise_gate: None,
                    noise_suppression: false,
                    lanes: lanes.iter().map(|l| (*l).into()).collect(),
                })
                .collect(),
        }
    }

    fn at_frame(f: i64) -> Timestamp {
        Timestamp(f * Timestamp::TICKS_PER_SEC / RATE)
    }

    fn mix_once(m: &mut Mixer, lanes: usize) -> Vec<Vec<f32>> {
        let mut out = vec![Vec::new(); lanes];
        m.mix(&mut out);
        out
    }

    #[test]
    fn routes_one_source_to_several_lanes_and_sums_sources() {
        let l = layout(&[("game", &["a", "b"], 0.0), ("mic", &["b"], 0.0)], &["a", "b"]);
        let mut m = Mixer::new(Timestamp(0), &l);
        m.push("game", at_frame(0), &vec![0.25; BLOCK_FRAMES * 2]);
        m.push("mic", at_frame(0), &vec![0.25; BLOCK_FRAMES * 2]);
        let out = mix_once(&mut m, 2);
        assert!((out[0][100] - 0.25).abs() < 1e-6);
        assert!((out[1][100] - 0.5).abs() < 1e-6);
    }

    #[test]
    fn places_chunks_by_timestamp_and_fills_gaps_with_silence() {
        let l = layout(&[("s", &["a"], 0.0)], &["a"]);
        let mut m = Mixer::new(Timestamp(0), &l);
        // First chunk at 0, then a 30 ms gap (no playback), then audio at 40 ms.
        m.push("s", at_frame(0), &vec![1.0 * 0.5; BLOCK_FRAMES * 2]);
        m.push("s", at_frame(4 * BLOCK_FRAMES as i64), &vec![0.5; BLOCK_FRAMES * 2]);
        let blocks: Vec<_> = (0..5).map(|_| mix_once(&mut m, 1)).collect();
        assert!(blocks[0][0][10] > 0.4);
        assert!(blocks[1][0].iter().all(|s| *s == 0.0));
        assert!(blocks[3][0].iter().all(|s| *s == 0.0));
        assert!(blocks[4][0][10] > 0.4);
    }

    #[test]
    fn small_jitter_is_absorbed() {
        let l = layout(&[("s", &["a"], 0.0)], &["a"]);
        let mut m = Mixer::new(Timestamp(0), &l);
        m.push("s", at_frame(0), &vec![0.5; BLOCK_FRAMES * 2]);
        // 3 ms late timestamp: still appended contiguously, no hole.
        m.push("s", at_frame(BLOCK_FRAMES as i64 + 144), &vec![0.5; BLOCK_FRAMES * 2]);
        let _ = mix_once(&mut m, 1);
        let b = mix_once(&mut m, 1);
        assert!(b[0].iter().all(|s| *s > 0.4));
    }

    #[test]
    fn source_starting_late_is_aligned_to_the_session_clock() {
        let l = layout(&[("s", &["a"], 0.0)], &["a"]);
        // Session started at t0 = 1 s; the source's first chunk is at 1.005 s.
        let t0 = Timestamp(Timestamp::TICKS_PER_SEC);
        let mut m = Mixer::new(t0, &l);
        m.push("s", Timestamp(t0.0 + 50_000), &vec![0.5; BLOCK_FRAMES * 2]);
        let b = mix_once(&mut m, 1);
        // First 5 ms (240 frames) silent, rest audible.
        assert!(b[0][..480].iter().all(|s| *s == 0.0));
        assert!(b[0][480..].iter().all(|s| *s > 0.4));
    }

    #[test]
    fn live_volume_and_routing_updates() {
        let mut l = layout(&[("s", &["a"], 0.0)], &["a", "b"]);
        let mut m = Mixer::new(Timestamp(0), &l);
        l.sources[0].lanes = vec!["b".into()];
        l.sources[0].muted = true;
        m.update(&l);
        // First block ramps down; the second is fully muted.
        m.push("s", at_frame(0), &vec![0.5; BLOCK_FRAMES * 4]);
        let _ = mix_once(&mut m, 2);
        let out = mix_once(&mut m, 2);
        assert!(out[0].iter().all(|s| *s == 0.0));
        assert!(out[1].iter().all(|s| s.abs() < 1e-6));
    }

    #[test]
    fn unknown_lane_ids_are_ignored_and_lanes_stay_fixed() {
        let mut l = layout(&[("s", &["a", "zzz"], 0.0)], &["a"]);
        let mut m = Mixer::new(Timestamp(0), &l);
        l.lanes.push(Lane { id: "new".into(), name: "new".into(), volume_db: 0.0, muted: false, mono: false });
        m.update(&l);
        assert_eq!(m.lane_ids(), vec!["a".to_string()]);
    }

    #[test]
    fn meters_report_per_lane() {
        let l = layout(&[("s", &["a"], -6.0)], &["a"]);
        let mut m = Mixer::new(Timestamp(0), &l);
        m.push("s", at_frame(0), &vec![1.0; BLOCK_FRAMES * 2]);
        let _ = mix_once(&mut m, 1);
        let meters = m.take_meters();
        assert!((meters.lanes[0].peak_db - (-6.0)).abs() < 0.2, "{:?}", meters);
        assert_eq!(meters.sources[0].id, "s");
    }

    #[test]
    fn frame_index_rounds_to_nearest_sample() {
        assert_eq!(frame_index(Timestamp(0), Timestamp(Timestamp::TICKS_PER_SEC)), 48_000);
        assert_eq!(frame_index(Timestamp(100), Timestamp(100)), 0);
        assert_eq!(frame_index(Timestamp(0), Timestamp(208)), 1); // 20.8 µs ≈ 1 sample
    }
}
