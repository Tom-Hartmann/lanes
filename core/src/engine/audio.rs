//! The audio side of a session: one WASAPI capture per source, the mixer
//! thread and the lane encoder thread.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

use crossbeam_channel::{Receiver, Sender, bounded};

use crate::Timestamp;
use crate::audio::mixer::{MeterFrame, Mixer};
use crate::audio::wasapi::{CaptureSpec, CaptureStatus, Chunk, SourceCapture};
use crate::av::{EncodedPacket, SharedPacket, StreamId, StreamInfo};
use crate::capture::windows_list;
use crate::config::{AudioLayout, AudioSource, EncoderSettings, SourceKind};
use crate::encode::audio::AudioEncoder;
use crate::error::Result;
use crate::util::{PacingTimer, Priority, spawn};

/// How far behind real time the mixer runs, so every source has delivered its
/// samples for a block before it is mixed (WASAPI periods are ~10 ms).
const LATENCY: Duration = Duration::from_millis(100);

pub type MeterCallback = Arc<dyn Fn(&MeterFrame) + Send + Sync>;
pub type StatusCallback = Arc<dyn Fn(&str, CaptureStatus) + Send + Sync>;

enum MixerCommand {
    Update(AudioLayout),
    Stop,
}

struct RunningSource {
    spec: CaptureSpec,
    capture: SourceCapture,
}

pub struct AudioSession {
    pub streams: Vec<StreamInfo>,
    layout: AudioLayout,
    chunks: Sender<Chunk>,
    sources: HashMap<String, RunningSource>,
    on_status: StatusCallback,
    mixer_cmd: Sender<MixerCommand>,
    mixer: Option<JoinHandle<Option<()>>>,
    encoder: Option<JoinHandle<Option<()>>>,
}

impl AudioSession {
    pub fn start(
        t0: Timestamp,
        layout: &AudioLayout,
        settings: &EncoderSettings,
        packets: Sender<SharedPacket>,
        on_meters: MeterCallback,
        on_status: StatusCallback,
    ) -> Result<Self> {
        // One encoder per lane, opened up front for the stream parameters.
        let mut encoders = Vec::new();
        let mut streams = Vec::new();
        for (i, lane) in layout.lanes.iter().enumerate() {
            let enc = AudioEncoder::open(settings.audio_codec, settings.audio_bitrate_kbps)?;
            streams.push(StreamInfo {
                id: StreamId::Audio(i as u8),
                params: Arc::new(enc.context().parameters()?),
                time_base: enc.context().time_base(),
                title: lane.name.clone(),
            });
            encoders.push(enc);
        }

        let (chunks, chunk_rx) = bounded::<Chunk>(512);
        let (blocks_tx, blocks_rx) = bounded::<Vec<Vec<f32>>>(64);
        let (mixer_cmd, mixer_cmd_rx) = crossbeam_channel::unbounded();

        let mixer_layout = layout.clone();
        let mixer = spawn("lanes-mixer", Priority::Audio, move || {
            mixer_loop(t0, mixer_layout, chunk_rx, mixer_cmd_rx, blocks_tx, on_meters);
        })?;
        let encoder = spawn("lanes-audio-encode", Priority::AboveNormal, move || {
            encode_loop(t0, encoders, blocks_rx, packets);
        })?;

        let mut session = Self {
            streams,
            layout: layout.clone(),
            chunks,
            sources: HashMap::new(),
            on_status,
            mixer_cmd,
            mixer: Some(mixer),
            encoder: Some(encoder),
        };
        session.sync_sources();
        Ok(session)
    }

    /// Applies volumes, routing and source changes live. Lane changes wait for the next session.
    pub fn update(&mut self, layout: &AudioLayout) {
        self.layout = layout.clone();
        let _ = self.mixer_cmd.send(MixerCommand::Update(layout.clone()));
        self.sync_sources();
    }

    /// Starts/stops captures to match the layout, and follows per-app sources
    /// to their current process (apps restart; Discord updates itself...).
    /// Called on changes and periodically by the engine.
    pub fn sync_sources(&mut self) {
        let procs = if self.layout.sources.iter().any(|s| matches!(s.kind, SourceKind::App { .. })) {
            windows_list::processes()
        } else {
            Vec::new()
        };
        let wanted: HashMap<String, Option<CaptureSpec>> =
            self.layout.sources.iter().map(|s| (s.id.clone(), spec_for(s, &procs))).collect();
        self.sources.retain(|id, running| wanted.get(id).is_some_and(|w| w.as_ref() == Some(&running.spec)));
        for (id, spec) in wanted {
            let Some(spec) = spec else {
                (self.on_status)(&id, CaptureStatus::Waiting { reason: "application is not running".into() });
                continue;
            };
            if self.sources.contains_key(&id) {
                continue;
            }
            let status = self.on_status.clone();
            match SourceCapture::start(Arc::from(id.as_str()), spec.clone(), self.chunks.clone(), move |src, st| status(src, st)) {
                Ok(capture) => {
                    self.sources.insert(id, RunningSource { spec, capture });
                }
                Err(e) => log::error!("audio source {id}: {e}"),
            }
        }
    }

    /// Reopens captures that follow the default device of the given direction.
    pub fn default_device_changed(&self, render: bool) {
        for running in self.sources.values() {
            let follows = match &running.spec {
                CaptureSpec::Loopback { device: None } => render,
                CaptureSpec::Input { device: None } => !render,
                _ => false,
            };
            if follows {
                running.capture.reopen();
            }
        }
    }
}

impl Drop for AudioSession {
    fn drop(&mut self) {
        self.sources.clear();
        let _ = self.mixer_cmd.send(MixerCommand::Stop);
        if let Some(t) = self.mixer.take() {
            let _ = t.join();
        }
        // The mixer dropped its block sender; the encoder flushes and exits.
        if let Some(t) = self.encoder.take() {
            let _ = t.join();
        }
    }
}

fn spec_for(source: &AudioSource, procs: &[windows_list::ProcessEntry]) -> Option<CaptureSpec> {
    Some(match &source.kind {
        SourceKind::SystemOutput { device } => CaptureSpec::Loopback { device: device.clone() },
        SourceKind::Input { device } => CaptureSpec::Input { device: device.clone() },
        SourceKind::App { exe } => CaptureSpec::Process { pid: *windows_list::root_pids(procs, exe).first()? },
    })
}

fn mixer_loop(
    t0: Timestamp,
    layout: AudioLayout,
    chunks: Receiver<Chunk>,
    commands: Receiver<MixerCommand>,
    blocks: Sender<Vec<Vec<f32>>>,
    on_meters: MeterCallback,
) {
    let mut mixer = Mixer::new(t0, &layout);
    let lanes = mixer.lane_ids().len();
    let Ok(timer) = PacingTimer::new() else {
        log::error!("mixer: no pacing timer");
        return;
    };
    let mut since_meters = 0;
    let stop = AtomicBool::new(false);
    while !stop.load(Ordering::Relaxed) {
        while let Ok(cmd) = commands.try_recv() {
            match cmd {
                MixerCommand::Update(l) => mixer.update(&l),
                MixerCommand::Stop => stop.store(true, Ordering::Relaxed),
            }
        }
        for chunk in chunks.try_iter() {
            mixer.push(&chunk.source, chunk.time, &chunk.samples);
        }
        let horizon = Timestamp(Timestamp::now().0 - (LATENCY.as_nanos() / 100) as i64);
        while mixer.next_block_end() <= horizon {
            let mut out = vec![Vec::new(); lanes];
            mixer.mix(&mut out);
            if blocks.send_timeout(out, Duration::from_millis(50)).is_err() {
                log::warn!("audio encoder is not keeping up; dropping a block");
            }
            since_meters += 1;
            if since_meters >= 5 {
                since_meters = 0;
                on_meters(&mixer.take_meters());
            }
        }
        timer.sleep(Duration::from_millis(5));
    }
}

fn encode_loop(t0: Timestamp, mut encoders: Vec<AudioEncoder>, blocks: Receiver<Vec<Vec<f32>>>, packets: Sender<SharedPacket>) {
    let mut out = Vec::new();
    let send = |lane: usize, enc: &AudioEncoder, out: &mut Vec<crate::av::Packet>| {
        let tb = enc.context().time_base();
        for p in out.drain(..) {
            let pkt = Arc::new(EncodedPacket::from_av(StreamId::Audio(lane as u8), &p, tb, t0));
            let _ = packets.send_timeout(pkt, Duration::from_millis(50));
        }
    };
    for block in blocks.iter() {
        for (lane, (enc, samples)) in encoders.iter_mut().zip(&block).enumerate() {
            if let Err(e) = enc.push(samples, &mut out) {
                log::error!("audio encode (lane {lane}): {e}");
            }
            send(lane, enc, &mut out);
        }
    }
    for (lane, enc) in encoders.iter_mut().enumerate() {
        let _ = enc.flush(&mut out);
        send(lane, enc, &mut out);
    }
}
