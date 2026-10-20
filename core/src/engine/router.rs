//! The packet router thread: every encoded packet (video and all lanes) passes
//! through here once and is fanned out to the replay buffer and the recorder.
//! Encoding happens once no matter how many consumers there are.
//!
//! Disk writes happen on this thread, never on capture or encode threads;
//! clip files are written on their own short-lived threads.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

use crossbeam_channel::{Receiver, Sender, select};

use crate::Timestamp;
use crate::av::{self, SharedPacket, StreamId, StreamInfo};
use crate::config::ReplayStorage;
use crate::error::{Error, Result};
use crate::mux::{Container, Muxer};
use crate::replay::{ReplayBuffer, ReplayLimits};
use crate::util::{Priority, spawn};

/// One output file: a video stream plus the audio lanes stored with it.
#[derive(Clone, Debug)]
pub struct FilePlan {
    pub path: PathBuf,
    pub streams: Vec<StreamId>,
}

pub type ClipDone = Box<dyn FnOnce(Result<(Vec<PathBuf>, f64)>) + Send>;

pub enum RouterCommand {
    StartRecording { files: Vec<FilePlan>, reply: Sender<Result<()>> },
    StopRecording { reply: Sender<Vec<PathBuf>> },
    Pause,
    Resume,
    SetReplay(Option<(ReplayLimits, ReplayStorage, PathBuf)>),
    SaveClip { now: Timestamp, length: Duration, files: Vec<FilePlan>, container: Container, done: ClipDone },
}

/// Counters for the stats overlay.
#[derive(Default)]
pub struct RouterStats {
    pub replay_bytes: AtomicU64,
    pub replay_ms: AtomicU64,
    pub recorded_bytes: AtomicU64,
    pub queued: AtomicU64,
}

pub struct Router {
    pub commands: Sender<RouterCommand>,
    pub stats: Arc<RouterStats>,
    thread: Option<JoinHandle<Option<()>>>,
    stop: Arc<AtomicBool>,
}

impl Router {
    pub fn start(
        t0: Timestamp,
        streams: Vec<StreamInfo>,
        packets: Receiver<SharedPacket>,
        keyframe_requests: Vec<Arc<AtomicBool>>,
        on_error: Arc<dyn Fn(Error) + Send + Sync>,
    ) -> std::io::Result<Self> {
        let (commands, command_rx) = crossbeam_channel::unbounded();
        let stats = Arc::new(RouterStats::default());
        let stop = Arc::new(AtomicBool::new(false));
        let worker = Worker {
            t0,
            streams,
            replay: None,
            recorder: None,
            keyframe_requests,
            stats: stats.clone(),
            on_error,
        };
        let stop_flag = stop.clone();
        let thread = spawn("lanes-router", Priority::AboveNormal, move || worker.run(packets, command_rx, stop_flag))?;
        Ok(Self { commands, stats, thread: Some(thread), stop })
    }
}

impl Drop for Router {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

struct Worker {
    t0: Timestamp,
    streams: Vec<StreamInfo>,
    replay: Option<ReplayBuffer>,
    recorder: Option<Recorder>,
    keyframe_requests: Vec<Arc<AtomicBool>>,
    stats: Arc<RouterStats>,
    on_error: Arc<dyn Fn(Error) + Send + Sync>,
}

impl Worker {
    fn run(mut self, packets: Receiver<SharedPacket>, commands: Receiver<RouterCommand>, stop: Arc<AtomicBool>) {
        loop {
            select! {
                recv(packets) -> pkt => match pkt {
                    Ok(pkt) => self.route(pkt),
                    // All encoders gone: the session is over.
                    Err(_) => break,
                },
                recv(commands) -> cmd => match cmd {
                    Ok(cmd) => self.command(cmd),
                    Err(_) => break,
                },
                default(Duration::from_millis(200)) => {
                    if stop.load(Ordering::Relaxed) && packets.is_empty() {
                        break;
                    }
                }
            }
            self.stats.queued.store(packets.len() as u64, Ordering::Relaxed);
        }
        if let Some(rec) = self.recorder.take() {
            let _ = rec.finish();
        }
    }

    fn route(&mut self, pkt: SharedPacket) {
        if let Some(replay) = &mut self.replay {
            if let Err(e) = replay.push(pkt.clone()) {
                (self.on_error)(e);
                self.replay = None;
            } else {
                self.stats.replay_bytes.store(replay.bytes() as u64, Ordering::Relaxed);
                self.stats.replay_ms.store(replay.buffered().as_millis() as u64, Ordering::Relaxed);
            }
        }
        if let Some(rec) = &mut self.recorder {
            if let Err(e) = rec.write(&pkt) {
                (self.on_error)(e);
                if let Some(rec) = self.recorder.take() {
                    let _ = rec.finish();
                }
            } else {
                self.stats.recorded_bytes.store(rec.bytes(), Ordering::Relaxed);
            }
        }
    }

    fn request_keyframes(&self) {
        for k in &self.keyframe_requests {
            k.store(true, Ordering::Relaxed);
        }
    }

    fn command(&mut self, cmd: RouterCommand) {
        match cmd {
            RouterCommand::StartRecording { files, reply } => {
                let result = Recorder::create(self.t0, &self.streams, &files).map(|r| {
                    self.recorder = Some(r);
                    self.request_keyframes();
                });
                let _ = reply.send(result);
            }
            RouterCommand::StopRecording { reply } => {
                let paths = self.recorder.take().map(|r| r.finish()).unwrap_or_default();
                self.stats.recorded_bytes.store(0, Ordering::Relaxed);
                let _ = reply.send(paths);
            }
            RouterCommand::Pause => {
                if let Some(r) = &mut self.recorder {
                    r.pause();
                }
            }
            RouterCommand::Resume => {
                if let Some(r) = &mut self.recorder {
                    r.resume();
                    self.request_keyframes();
                }
            }
            RouterCommand::SetReplay(cfg) => match cfg {
                None => {
                    self.replay = None;
                    self.stats.replay_bytes.store(0, Ordering::Relaxed);
                    self.stats.replay_ms.store(0, Ordering::Relaxed);
                }
                Some((limits, storage, dir)) => match &mut self.replay {
                    Some(r) => r.set_limits(limits),
                    None => match ReplayBuffer::new(limits, storage, &dir) {
                        Ok(r) => self.replay = Some(r),
                        Err(e) => (self.on_error)(e),
                    },
                },
            },
            RouterCommand::SaveClip { now, length, files, container, done } => self.save_clip(now, length, files, container, done),
        }
    }

    fn save_clip(&mut self, now: Timestamp, length: Duration, files: Vec<FilePlan>, container: Container, done: ClipDone) {
        let Some(replay) = &mut self.replay else {
            done(Err(Error::Invalid("the replay buffer is not running".into())));
            return;
        };
        let wanted: Vec<StreamId> = files.iter().flat_map(|f| f.streams.iter().copied()).collect();
        let Some((cut, packets)) = replay.snapshot(now, length, &wanted) else {
            done(Err(Error::Invalid("the replay buffer is still empty".into())));
            return;
        };
        let streams = self.streams.clone();
        let t0 = self.t0;
        // Writing happens off the router thread so routing never stalls.
        let spawned = spawn("lanes-clip-writer", Priority::Low, move || {
            let result = (|| -> Result<(Vec<PathBuf>, f64)> {
                let mut paths = Vec::new();
                let mut duration = 0.0f64;
                for file in &files {
                    let infos: Vec<StreamInfo> = streams.iter().filter(|s| file.streams.contains(&s.id)).cloned().collect();
                    let pkts: Vec<SharedPacket> = packets
                        .iter()
                        .filter(|p| file.streams.contains(&p.stream()))
                        .map(|p| p.load())
                        .collect::<std::io::Result<_>>()?;
                    duration = duration.max(crate::mux::write_packets(&file.path, container, &infos, &pkts, cut, t0)?);
                    paths.push(file.path.clone());
                }
                Ok((paths, duration))
            })();
            done(result);
        });
        if let Err(e) = spawned {
            log::error!("could not start the clip writer: {e}");
        }
    }
}

/// One recording file in progress.
struct RecFile {
    muxer: Muxer,
    streams: Vec<StreamId>,
    video: StreamId,
    /// Waiting for a video keyframe (at start and after resume).
    gated: bool,
    /// When the current gate started (session start, or the pause moment).
    gate_since: Timestamp,
    /// Total ticks removed from the timeline (start offset + pauses).
    shift: i64,
    /// Packets earlier than this belong to the cut-out part.
    open_from: Timestamp,
}

/// How much recent audio the recorder keeps while waiting for a keyframe.
/// Audio arrives earlier than video relative to its timestamps, so when the
/// keyframe that opens a file arrives, its matching audio is already here.
const PREROLL: Duration = Duration::from_secs(3);

struct Recorder {
    files: Vec<RecFile>,
    preroll: std::collections::VecDeque<SharedPacket>,
    infos: Vec<StreamInfo>,
    t0: Timestamp,
    paused_at: Option<Timestamp>,
}

impl Recorder {
    fn create(t0: Timestamp, streams: &[StreamInfo], plans: &[FilePlan]) -> Result<Self> {
        let mut files = Vec::new();
        for plan in plans {
            let infos: Vec<StreamInfo> = streams.iter().filter(|s| plan.streams.contains(&s.id)).cloned().collect();
            let video = infos
                .iter()
                .map(|s| s.id)
                .find(|id| matches!(id, StreamId::Video(_)))
                .ok_or_else(|| Error::Invalid("a recording file needs a video stream".into()))?;
            let muxer = Muxer::create(&plan.path, Container::Matroska, &infos)?;
            log::info!("recording to {}", plan.path.display());
            files.push(RecFile {
                muxer,
                streams: plan.streams.clone(),
                video,
                gated: true,
                gate_since: t0,
                shift: 0,
                open_from: Timestamp(i64::MAX),
            });
        }
        Ok(Self { files, preroll: std::collections::VecDeque::new(), infos: streams.to_vec(), t0, paused_at: None })
    }

    fn write(&mut self, pkt: &SharedPacket) -> Result<()> {
        if !matches!(pkt.stream, StreamId::Video(_)) {
            self.preroll.push_back(pkt.clone());
            let horizon = pkt.time.0 - (PREROLL.as_nanos() / 100) as i64;
            while self.preroll.front().is_some_and(|p| p.time.0 < horizon) {
                self.preroll.pop_front();
            }
        }
        if self.paused_at.is_some() {
            return Ok(());
        }
        for i in 0..self.files.len() {
            let f = &mut self.files[i];
            if !f.streams.contains(&pkt.stream) {
                continue;
            }
            if f.gated {
                if pkt.stream != f.video || !pkt.keyframe {
                    continue;
                }
                // Cut out everything between the gate start and this keyframe.
                f.shift += pkt.time.0 - f.gate_since.0;
                f.open_from = pkt.time;
                f.gated = false;
                Self::write_one(&self.infos, f, pkt)?;
                // Audio for this moment usually arrived before the keyframe did.
                for p in &self.preroll {
                    if f.streams.contains(&p.stream) && p.time >= f.open_from {
                        Self::write_one(&self.infos, f, p)?;
                    }
                }
                continue;
            }
            if pkt.time < f.open_from {
                continue;
            }
            Self::write_one(&self.infos, f, pkt)?;
        }
        Ok(())
    }

    fn write_one(infos: &[StreamInfo], f: &mut RecFile, pkt: &SharedPacket) -> Result<()> {
        let tb = infos.iter().find(|s| s.id == pkt.stream).map(|s| s.time_base).unwrap_or(av::rational(1, 1000));
        let offset = av::rescale(f.shift, av::rational(1, Timestamp::TICKS_PER_SEC as i32), tb);
        f.muxer.write(pkt, offset)
    }

    fn pause(&mut self) {
        if self.paused_at.is_none() {
            self.paused_at = Some(Timestamp::now());
        }
    }

    fn resume(&mut self) {
        if let Some(at) = self.paused_at.take() {
            for f in &mut self.files {
                f.gated = true;
                f.gate_since = at;
            }
        }
    }

    fn bytes(&self) -> u64 {
        self.files.iter().map(|f| f.muxer.bytes).sum()
    }

    fn finish(self) -> Vec<PathBuf> {
        let _ = self.t0;
        self.files
            .into_iter()
            .filter_map(|f| match f.muxer.finish() {
                Ok(p) => Some(p),
                Err(e) => {
                    log::error!("finishing recording: {e}");
                    None
                }
            })
            .collect()
    }
}
