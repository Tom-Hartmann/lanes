//! One video output stream: capture source(s) → GPU convert → encoder, paced
//! at a constant frame rate on the shared clock.
//!
//! Frame `n` is due at `t0 + n / fps`. At each tick the newest captured frame
//! is converted straight into an encoder surface. If no new frame arrived the
//! previous one is encoded again (a duplicate); if the thread falls behind,
//! whole ticks are skipped (dropped frames). Either way the output stays CFR
//! and in sync with audio, which is anchored at the same `t0`.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

use crossbeam_channel::{Receiver, Sender};

use crate::Timestamp;
use crate::av::{EncodedPacket, SharedPacket, StreamId, StreamInfo};
use crate::capture::{Capture, CaptureOptions, Frame, FrameSlot, Target};
use crate::config::CaptureApi;
use crate::convert::{Converter, Layer, OutputFormat, Rect};
use crate::encode::video::{VideoEncoder, VideoEncoderConfig};
use crate::error::{Error, Result};
use crate::gpu::Gpu;
use crate::util::{PacingTimer, Priority, spawn};

/// One captured input placed on the output.
#[derive(Clone, Debug, PartialEq)]
pub struct LayerPlan {
    pub target: Target,
    pub area: Rect,
    /// Capture rate cap for this input (canvas mode per-monitor caps).
    pub max_fps: u32,
}

#[derive(Clone, Debug)]
pub struct VideoPlan {
    pub index: u8,
    pub title: String,
    pub layers: Vec<LayerPlan>,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
}

/// Counters read by the stats ticker.
#[derive(Default)]
pub struct VideoStats {
    pub frames: AtomicU64,
    pub dropped: AtomicU64,
    pub duplicated: AtomicU64,
    pub encode_ns: AtomicU64,
}

pub enum VideoCommand {
    SetLayers(Vec<LayerPlan>),
    Stop,
}

/// What the stream reports back to the engine.
pub enum VideoEvent {
    /// A capture source ended (window closed, monitor unplugged).
    SourceClosed { stream: u8, target: Target },
    Failed { stream: u8, error: Error },
}

pub struct VideoStream {
    pub plan: VideoPlan,
    pub info: StreamInfo,
    pub stats: Arc<VideoStats>,
    pub force_keyframe: Arc<AtomicBool>,
    pub encoder_label: String,
    commands: Sender<VideoCommand>,
    thread: Option<JoinHandle<Option<()>>>,
}

pub struct VideoStart<'a> {
    pub gpu: &'a Gpu,
    pub plan: VideoPlan,
    pub encoder: VideoEncoderConfig,
    pub api: CaptureApi,
    pub cursor: bool,
    pub t0: Timestamp,
    pub packets: Sender<SharedPacket>,
    pub events: Sender<VideoEvent>,
}

impl VideoStream {
    pub fn start(args: VideoStart<'_>) -> Result<Self> {
        let VideoStart { gpu, plan, mut encoder, api, cursor, t0, packets, events } = args;
        encoder.width = plan.width;
        encoder.height = plan.height;
        encoder.fps = plan.fps;
        // Open the encoder here so failures surface synchronously and the
        // stream parameters (extradata) are known before muxers are created.
        let enc = VideoEncoder::open(gpu, &encoder)?;
        let conv = Converter::new(gpu, plan.width, plan.height, OutputFormat::Nv12)?;
        let info = StreamInfo {
            id: StreamId::Video(plan.index),
            params: Arc::new(enc.context().parameters()?),
            time_base: enc.context().time_base(),
            title: plan.title.clone(),
        };
        let encoder_label = format!(
            "{} · {}{}",
            crate::encode::backend_label(encoder.backend),
            crate::encode::codec_label(encoder.codec),
            if enc.is_zero_copy() { "" } else { " (readback)" }
        );
        let stats = Arc::new(VideoStats::default());
        let force_keyframe = Arc::new(AtomicBool::new(false));
        let (commands, command_rx) = crossbeam_channel::unbounded();
        let worker = Worker {
            gpu: gpu.clone(),
            plan: plan.clone(),
            enc,
            conv,
            api,
            cursor,
            t0,
            packets,
            events,
            stats: stats.clone(),
            force_keyframe: force_keyframe.clone(),
            commands: command_rx,
            captures: Vec::new(),
            last_frames: Vec::new(),
            last_seqs: Vec::new(),
        };
        let thread = spawn(&format!("lanes-video-{}", plan.index), Priority::AboveNormal, move || worker.run())?;
        Ok(Self { plan, info, stats, force_keyframe, encoder_label, commands, thread: Some(thread) })
    }

    pub fn set_layers(&self, layers: Vec<LayerPlan>) {
        let _ = self.commands.send(VideoCommand::SetLayers(layers));
    }

    pub fn request_keyframe(&self) {
        self.force_keyframe.store(true, Ordering::Relaxed);
    }
}

impl Drop for VideoStream {
    fn drop(&mut self) {
        let _ = self.commands.send(VideoCommand::Stop);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

struct ActiveCapture {
    plan: LayerPlan,
    slot: Arc<FrameSlot>,
    _capture: Option<Capture>,
    reported_closed: bool,
}

struct Worker {
    gpu: Gpu,
    plan: VideoPlan,
    enc: VideoEncoder,
    conv: Converter,
    api: CaptureApi,
    cursor: bool,
    t0: Timestamp,
    packets: Sender<SharedPacket>,
    events: Sender<VideoEvent>,
    stats: Arc<VideoStats>,
    force_keyframe: Arc<AtomicBool>,
    commands: Receiver<VideoCommand>,
    captures: Vec<ActiveCapture>,
    last_frames: Vec<Option<Frame>>,
    last_seqs: Vec<u64>,
}

impl Worker {
    fn run(mut self) {
        if let Err(error) = self.run_inner() {
            log::error!("video stream {} failed: {error}", self.plan.index);
            let _ = self.events.send(VideoEvent::Failed { stream: self.plan.index, error });
        }
    }

    fn start_captures(&mut self, layers: Vec<LayerPlan>) {
        self.captures.clear();
        self.last_frames = vec![None; layers.len()];
        self.last_seqs = vec![0; layers.len()];
        for plan in layers {
            let slot = FrameSlot::new();
            let opts = CaptureOptions { cursor: self.cursor, max_fps: plan.max_fps };
            let capture = match Capture::start(&self.gpu, &plan.target, self.api, &opts, slot.clone()) {
                Ok(c) => {
                    log::info!("capturing {} via {}", plan.target.label(), c.api_name());
                    Some(c)
                }
                Err(e) => {
                    log::error!("cannot capture {}: {e}", plan.target.label());
                    slot.close();
                    None
                }
            };
            self.captures.push(ActiveCapture { plan, slot, _capture: capture, reported_closed: false });
        }
    }

    fn run_inner(&mut self) -> Result<()> {
        self.start_captures(self.plan.layers.clone());
        let timer = PacingTimer::new().map_err(|e| Error::Windows { context: "pacing timer", source: e })?;
        let period = Timestamp::TICKS_PER_SEC / i64::from(self.plan.fps.max(1));
        // Start at the first frame slot after now, so pts line up with t0.
        let mut n = ((Timestamp::now().0 - self.t0.0) / period).max(0) + 1;
        let mut pending = Vec::new();
        let mut first = true;
        loop {
            while let Ok(cmd) = self.commands.try_recv() {
                match cmd {
                    VideoCommand::Stop => return self.flush(),
                    VideoCommand::SetLayers(layers) => {
                        self.start_captures(layers);
                        self.force_keyframe.store(true, Ordering::Relaxed);
                    }
                }
            }
            let due = Timestamp(self.t0.0 + n * period);
            let now = Timestamp::now();
            if now < due {
                // Wake slightly early and spin-free: the timer is high resolution.
                timer.sleep(due.saturating_since(now));
                continue;
            }
            let behind = (now.0 - due.0) / period;
            if behind >= 1 {
                self.stats.dropped.fetch_add(behind as u64, Ordering::Relaxed);
                n += behind;
            }

            // Collect the newest frame of every layer.
            let mut fresh = false;
            for (i, cap) in self.captures.iter_mut().enumerate() {
                if let Some((frame, seq)) = cap.slot.latest() {
                    if seq != self.last_seqs[i] {
                        fresh = true;
                        self.last_seqs[i] = seq;
                    }
                    self.last_frames[i] = Some(frame);
                }
                if cap.slot.is_closed() && !cap.reported_closed {
                    cap.reported_closed = true;
                    let _ = self.events.send(VideoEvent::SourceClosed { stream: self.plan.index, target: cap.plan.target.clone() });
                }
            }
            // Until the first frame arrives there is nothing to show; encoding an
            // empty surface would put a garbage (green) keyframe at the start.
            if self.last_frames.iter().all(Option::is_none) {
                n += 1;
                continue;
            }
            if !fresh && !first {
                self.stats.duplicated.fetch_add(1, Ordering::Relaxed);
            }

            let started = std::time::Instant::now();
            let layers: Vec<Layer<'_>> = self
                .captures
                .iter()
                .zip(&self.last_frames)
                .filter_map(|(cap, f)| f.as_ref().map(|frame| Layer { frame, dest: cap.plan.area }))
                .collect();
            let surface = self.enc.next_surface()?;
            self.conv.convert(&layers, &surface.texture, surface.index)?;
            let keyframe = first || self.force_keyframe.swap(false, Ordering::Relaxed);
            self.enc.submit(surface, n, keyframe, &mut pending)?;
            self.stats.encode_ns.fetch_add(started.elapsed().as_nanos() as u64, Ordering::Relaxed);
            self.stats.frames.fetch_add(1, Ordering::Relaxed);
            self.emit(&mut pending);
            first = false;
            n += 1;

            if n % 120 == 0 {
                self.gpu.check_alive()?;
            }
        }
    }

    fn emit(&self, pending: &mut Vec<crate::av::Packet>) {
        let tb = self.enc.context().time_base();
        for p in pending.drain(..) {
            let pkt = Arc::new(EncodedPacket::from_av(StreamId::Video(self.plan.index), &p, tb, self.t0));
            if self.packets.send_timeout(pkt, Duration::from_millis(50)).is_err() {
                log::warn!("packet router is not keeping up; dropping a video packet");
            }
        }
    }

    fn flush(&mut self) -> Result<()> {
        let mut pending = Vec::new();
        self.enc.flush(&mut pending)?;
        self.emit(&mut pending);
        Ok(())
    }
}
