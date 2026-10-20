//! The replay buffer: a rolling window of already-encoded packets.
//!
//! Only compressed packets are kept (about 2.5 MB/s at 20 Mbit/s), never raw
//! frames. Eviction happens in whole GOPs so the oldest packet is always a
//! video keyframe, which makes "save the last N seconds" an instant,
//! copy-free snapshot of `Arc`s. With disk storage, payloads live in
//! rotating temp-file segments and only the index stays in RAM.

use std::collections::VecDeque;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use crate::Timestamp;
use crate::av::{EncodedPacket, SharedPacket, StreamId};
use crate::config::ReplayStorage;
use crate::error::Result;

/// The stream whose keyframes define cut points.
const PRIMARY: StreamId = StreamId::Video(0);
const SEGMENT_BYTES: u64 = 32 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct ReplayLimits {
    pub duration: Duration,
    pub max_bytes: usize,
}

enum Payload {
    Ram(SharedPacket),
    Disk { segment: Arc<Segment>, offset: u64, len: u32, meta: Meta },
}

#[derive(Clone)]
struct Meta {
    stream: StreamId,
    pts: i64,
    dts: i64,
    duration: i64,
    keyframe: bool,
    time: Timestamp,
}

impl Payload {
    fn stream(&self) -> StreamId {
        match self {
            Payload::Ram(p) => p.stream,
            Payload::Disk { meta, .. } => meta.stream,
        }
    }
    fn time(&self) -> Timestamp {
        match self {
            Payload::Ram(p) => p.time,
            Payload::Disk { meta, .. } => meta.time,
        }
    }
    fn keyframe(&self) -> bool {
        match self {
            Payload::Ram(p) => p.keyframe,
            Payload::Disk { meta, .. } => meta.keyframe,
        }
    }
    fn len(&self) -> usize {
        match self {
            Payload::Ram(p) => p.data.len(),
            Payload::Disk { len, .. } => *len as usize,
        }
    }
}

/// A temp file holding packet payloads; deleted when the last packet in it is evicted
/// and no pending clip still references it.
struct Segment {
    path: PathBuf,
}

impl Drop for Segment {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

struct DiskWriter {
    dir: PathBuf,
    current: Option<(Arc<Segment>, File, u64)>,
    next_id: u64,
}

impl DiskWriter {
    fn new(dir: PathBuf) -> Result<Self> {
        fs::create_dir_all(&dir)?;
        // Leftovers from a previous run.
        if let Ok(entries) = fs::read_dir(&dir) {
            for e in entries.flatten() {
                if e.file_name().to_string_lossy().starts_with("segment-") {
                    let _ = fs::remove_file(e.path());
                }
            }
        }
        Ok(Self { dir, current: None, next_id: 0 })
    }

    fn write(&mut self, data: &[u8]) -> Result<(Arc<Segment>, u64)> {
        if self.current.as_ref().is_none_or(|(_, _, len)| *len >= SEGMENT_BYTES) {
            let path = self.dir.join(format!("segment-{:06}.bin", self.next_id));
            self.next_id += 1;
            let file = OpenOptions::new().create(true).truncate(true).write(true).open(&path)?;
            self.current = Some((Arc::new(Segment { path }), file, 0));
        }
        let (seg, file, len) = self.current.as_mut().expect("just created");
        file.write_all(data)?;
        let offset = *len;
        *len += data.len() as u64;
        Ok((seg.clone(), offset))
    }
}

/// A snapshot entry handed to the clip writer thread.
pub struct ClipPacket(Payload);

impl ClipPacket {
    /// Materialises the packet (reads it from disk for disk-backed buffers).
    pub fn load(&self) -> std::io::Result<SharedPacket> {
        match &self.0 {
            Payload::Ram(p) => Ok(p.clone()),
            Payload::Disk { segment, offset, len, meta } => {
                let mut f = File::open(&segment.path)?;
                f.seek(SeekFrom::Start(*offset))?;
                let mut data = vec![0u8; *len as usize];
                f.read_exact(&mut data)?;
                Ok(Arc::new(EncodedPacket {
                    stream: meta.stream,
                    data: data.into_boxed_slice(),
                    pts: meta.pts,
                    dts: meta.dts,
                    duration: meta.duration,
                    keyframe: meta.keyframe,
                    time: meta.time,
                }))
            }
        }
    }

    pub fn stream(&self) -> StreamId {
        self.0.stream()
    }
}

pub struct ReplayBuffer {
    entries: VecDeque<Payload>,
    bytes: usize,
    limits: ReplayLimits,
    disk: Option<DiskWriter>,
}

impl ReplayBuffer {
    pub fn new(limits: ReplayLimits, storage: ReplayStorage, disk_dir: &Path) -> Result<Self> {
        let disk = match storage {
            ReplayStorage::Ram => None,
            ReplayStorage::Disk => Some(DiskWriter::new(disk_dir.to_owned())?),
        };
        Ok(Self { entries: VecDeque::new(), bytes: 0, limits, disk })
    }

    pub fn set_limits(&mut self, limits: ReplayLimits) {
        self.limits = limits;
        self.evict();
    }

    pub fn push(&mut self, pkt: SharedPacket) -> Result<()> {
        // Nothing before the first primary keyframe can be decoded.
        if self.entries.is_empty() && !(pkt.stream == PRIMARY && pkt.keyframe) {
            return Ok(());
        }
        let payload = match &mut self.disk {
            None => Payload::Ram(pkt),
            Some(disk) => {
                let (segment, offset) = disk.write(&pkt.data)?;
                Payload::Disk {
                    segment,
                    offset,
                    len: pkt.data.len() as u32,
                    meta: Meta {
                        stream: pkt.stream,
                        pts: pkt.pts,
                        dts: pkt.dts,
                        duration: pkt.duration,
                        keyframe: pkt.keyframe,
                        time: pkt.time,
                    },
                }
            }
        };
        // RAM accounting covers payloads kept in memory; disk mode only keeps the index.
        if matches!(payload, Payload::Ram(_)) {
            self.bytes += payload.len();
        }
        self.entries.push_back(payload);
        self.evict();
        Ok(())
    }

    /// Drops whole GOPs from the front while over the duration or byte limit.
    /// The newest GOP is never dropped.
    fn evict(&mut self) {
        let Some(newest) = self.entries.back().map(Payload::time) else { return };
        loop {
            let Some(front) = self.entries.front() else { return };
            let too_old = newest.saturating_since(front.time()) > self.limits.duration;
            let too_big = self.bytes > self.limits.max_bytes;
            if !too_old && !too_big {
                return;
            }
            // Find the next primary keyframe after the front one.
            let Some(next_kf) = self.entries.iter().skip(1).position(|p| p.stream() == PRIMARY && p.keyframe()) else {
                return;
            };
            // Keep the window at least `duration` long: only drop the front GOP
            // if what remains still covers the duration (or we are over budget).
            let next_time = self.entries[next_kf + 1].time();
            if !too_big && newest.saturating_since(next_time) < self.limits.duration {
                return;
            }
            for _ in 0..=next_kf {
                if let Some(Payload::Ram(p)) = self.entries.pop_front() {
                    self.bytes -= p.data.len();
                }
            }
        }
    }

    /// Seconds of video currently buffered.
    pub fn buffered(&self) -> Duration {
        match (self.entries.front(), self.entries.back()) {
            (Some(a), Some(b)) => b.time().saturating_since(a.time()),
            _ => Duration::ZERO,
        }
    }

    pub fn bytes(&self) -> usize {
        self.bytes
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.bytes = 0;
    }

    /// The packets of the last `length` (starting at the latest primary
    /// keyframe at or before `now - length`), limited to `streams`. Returns
    /// the cut time and the packets in buffer order.
    pub fn snapshot(&mut self, now: Timestamp, length: Duration, streams: &[StreamId]) -> Option<(Timestamp, Vec<ClipPacket>)> {
        let want = Timestamp(now.0 - (length.as_nanos() / 100) as i64);
        let mut start_idx = None;
        for (i, p) in self.entries.iter().enumerate() {
            if p.stream() == PRIMARY && p.keyframe() {
                if p.time() <= want || start_idx.is_none() {
                    start_idx = Some(i);
                }
                if p.time() > want {
                    break;
                }
            }
        }
        let start_idx = start_idx?;
        let cut = self.entries[start_idx].time();
        if let Some(DiskWriter { current: Some((_, f, _)), .. }) = &mut self.disk {
            let _ = f.flush();
        }
        // Encoders deliver with different latencies (video can lag audio by
        // hundreds of ms while an encoder warms up), so arrival order is not
        // time order. Video is taken in arrival order from the cut keyframe;
        // every other stream is selected by time from anywhere in the buffer.
        let mut packets: Vec<ClipPacket> = self
            .entries
            .iter()
            .enumerate()
            .filter(|(i, p)| {
                let in_window = match p.stream() {
                    StreamId::Video(_) => *i >= start_idx,
                    StreamId::Audio(_) => p.time() >= cut,
                };
                in_window && streams.contains(&p.stream()) && p.time() <= now
            })
            .map(|(_, p)| {
                ClipPacket(match p {
                    Payload::Ram(pkt) => Payload::Ram(pkt.clone()),
                    Payload::Disk { segment, offset, len, meta } => {
                        Payload::Disk { segment: segment.clone(), offset: *offset, len: *len, meta: meta.clone() }
                    }
                })
            })
            .collect();
        // Stable: keeps decode order within each stream.
        packets.sort_by_key(|p| p.0.time());
        Some((cut, packets))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEC: i64 = Timestamp::TICKS_PER_SEC;

    /// 60 fps video with a keyframe every second, plus one audio packet per 20 ms.
    fn feed(buf: &mut ReplayBuffer, secs: i64) {
        for f in 0..secs * 60 {
            let t = f * SEC / 60;
            buf.push(pkt(StreamId::Video(0), f, f % 60 == 0, t, 1000)).unwrap();
            if f % 3 == 0 {
                buf.push(pkt(StreamId::Audio(0), f / 3, true, t, 100)).unwrap();
            }
        }
    }

    fn pkt(stream: StreamId, pts: i64, keyframe: bool, t: i64, size: usize) -> SharedPacket {
        Arc::new(EncodedPacket {
            stream,
            data: vec![0u8; size].into_boxed_slice(),
            pts,
            dts: pts,
            duration: 1,
            keyframe,
            time: Timestamp(t),
        })
    }

    fn ram(duration_secs: u64, max_bytes: usize) -> ReplayBuffer {
        let limits = ReplayLimits { duration: Duration::from_secs(duration_secs), max_bytes };
        ReplayBuffer::new(limits, ReplayStorage::Ram, Path::new(".")).unwrap()
    }

    #[test]
    fn keeps_at_least_the_configured_duration_in_whole_gops() {
        let mut buf = ram(10, usize::MAX);
        feed(&mut buf, 30);
        let held = buf.buffered();
        assert!(held >= Duration::from_secs(10), "{held:?}");
        assert!(held < Duration::from_secs(12), "{held:?}");
        let front = buf.entries.front().unwrap();
        assert!(front.stream() == PRIMARY && front.keyframe());
    }

    #[test]
    fn byte_cap_wins_over_duration() {
        // ~60 KB/s video + audio; cap at 200 KB -> about 3 seconds.
        let mut buf = ram(60, 200_000);
        feed(&mut buf, 30);
        assert!(buf.bytes() <= 200_000 + 61 * 1100, "{}", buf.bytes());
        assert!(buf.buffered() < Duration::from_secs(5));
    }

    #[test]
    fn drops_leading_non_keyframes() {
        let mut buf = ram(10, usize::MAX);
        buf.push(pkt(StreamId::Audio(0), 0, true, 0, 10)).unwrap();
        buf.push(pkt(StreamId::Video(0), 0, false, 0, 10)).unwrap();
        assert_eq!(buf.entries.len(), 0);
        buf.push(pkt(StreamId::Video(0), 1, true, 1, 10)).unwrap();
        assert_eq!(buf.entries.len(), 1);
    }

    #[test]
    fn snapshot_cuts_on_keyframe_before_the_window() {
        let mut buf = ram(60, usize::MAX);
        feed(&mut buf, 30);
        let now = Timestamp(30 * SEC);
        // Last 5.5 s -> the keyframe at 24 s (the one at or before 24.5 s).
        let (cut, pkts) = buf.snapshot(now, Duration::from_millis(5500), &[StreamId::Video(0), StreamId::Audio(0)]).unwrap();
        assert_eq!(cut, Timestamp(24 * SEC));
        let first = pkts[0].load().unwrap();
        assert!(first.keyframe && first.stream == StreamId::Video(0));
        assert!(pkts.iter().any(|p| p.stream() == StreamId::Audio(0)));
    }

    #[test]
    fn snapshot_includes_audio_that_arrived_before_lagging_video() {
        let mut buf = ram(60, usize::MAX);
        // Video lags 0.5 s behind audio in arrival order (encoder warm-up).
        for f in 0..300i64 {
            let t = f * SEC / 60;
            if f >= 30 {
                let vf = f - 30;
                buf.push(pkt(StreamId::Video(0), vf, vf % 60 == 0, vf * SEC / 60, 100)).unwrap();
            }
            if f % 3 == 0 && !buf.entries.is_empty() {
                buf.push(pkt(StreamId::Audio(0), f / 3, true, t, 10)).unwrap();
            }
        }
        // Cut at the keyframe at 2 s; audio from 2.0 s arrived before that keyframe.
        let (cut, pkts) = buf.snapshot(Timestamp(4 * SEC), Duration::from_secs(2), &[StreamId::Video(0), StreamId::Audio(0)]).unwrap();
        assert_eq!(cut, Timestamp(2 * SEC));
        let first_audio = pkts.iter().map(|p| p.load().unwrap()).find(|p| p.stream == StreamId::Audio(0)).unwrap();
        assert_eq!(first_audio.time, Timestamp(2 * SEC));
        let times: Vec<_> = pkts.iter().map(|p| p.load().unwrap().time).collect();
        assert!(times.windows(2).all(|w| w[0] <= w[1]), "sorted by time");
    }

    #[test]
    fn snapshot_filters_lanes() {
        let mut buf = ram(60, usize::MAX);
        feed(&mut buf, 5);
        let (_, pkts) = buf.snapshot(Timestamp(5 * SEC), Duration::from_secs(2), &[StreamId::Video(0)]).unwrap();
        assert!(pkts.iter().all(|p| p.stream() == StreamId::Video(0)));
    }

    #[test]
    fn snapshot_longer_than_buffer_returns_everything() {
        let mut buf = ram(60, usize::MAX);
        feed(&mut buf, 3);
        let (cut, _) = buf.snapshot(Timestamp(3 * SEC), Duration::from_secs(600), &[StreamId::Video(0)]).unwrap();
        assert_eq!(cut, Timestamp(0));
    }

    #[test]
    fn disk_storage_roundtrips_payloads_and_deletes_segments() {
        let dir = std::env::temp_dir().join(format!("lanes-replay-test-{}", std::process::id()));
        let limits = ReplayLimits { duration: Duration::from_secs(2), max_bytes: usize::MAX };
        let mut buf = ReplayBuffer::new(limits, ReplayStorage::Disk, &dir).unwrap();
        let mut p = EncodedPacket { stream: StreamId::Video(0), data: vec![7u8; 1234].into(), pts: 0, dts: 0, duration: 1, keyframe: true, time: Timestamp(0) };
        buf.push(Arc::new(EncodedPacket { data: p.data.clone(), ..p })).unwrap();
        p.data = vec![9u8; 10].into();
        let (_, pkts) = buf.snapshot(Timestamp(1), Duration::from_secs(1), &[StreamId::Video(0)]).unwrap();
        let loaded = pkts[0].load().unwrap();
        assert_eq!(&*loaded.data, &[7u8; 1234][..]);
        assert_eq!(buf.bytes(), 0, "disk mode keeps payloads out of RAM");
        drop(pkts);
        drop(buf);
        assert!(fs::read_dir(&dir).unwrap().next().is_none(), "segments removed");
        let _ = fs::remove_dir_all(dir);
    }
}
