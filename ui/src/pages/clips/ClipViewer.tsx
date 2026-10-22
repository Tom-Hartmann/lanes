import { convertFileSrc } from "@tauri-apps/api/core";
import { useEffect, useMemo, useRef, useState } from "react";
import { IconFolder, IconPlay, IconScissors, IconX } from "../../components/icons";
import { Badge, Button, Callout, ConfirmButton, NumberInput, Spinner } from "../../components/ui";
import { formatBytes, formatDate, formatDuration } from "../../lib/format";
import { api, errorMessage, type ClipInfo } from "../../lib/ipc";
import { actionForKey, keyLabel, resolveViewerKeys, VIEWER_ACTIONS } from "../../lib/viewerKeys";
import { attempt, notify } from "../../store/noticeStore";
import { useSettingsStore } from "../../store/settingsStore";
import { LaneMixerPanel, resumeAudio, useLaneMixer } from "./LaneMixer";
import { forgetThumbnail } from "./thumbnails";
import { SeekBar } from "./SeekBar";
import { DragHandle, SharePanel } from "./SharePanel";
import { TrimBar } from "./TrimBar";

interface Props {
  clip: ClipInfo;
  onClose: () => void;
  /** The library changed (new trimmed/remuxed file, deleted clip). */
  onChanged: () => void;
}

const round1 = (n: number) => Math.round(n * 10) / 10;

const SPEEDS = [0.1, 0.25, 0.5, 0.75, 1, 1.5, 2];
const SPEED_KEY = "lanes.playbackSpeed";
const FRAME = 1 / 60;

function loadSpeed(): number {
  try {
    const v = Number(localStorage.getItem(SPEED_KEY));
    return SPEEDS.includes(v) ? v : 1;
  } catch {
    return 1;
  }
}

const VOLUME_KEY = "lanes.viewerVolume";

function loadVolume(): { volume: number; muted: boolean } {
  try {
    const v = JSON.parse(localStorage.getItem(VOLUME_KEY) ?? "null") as { volume?: unknown; muted?: unknown } | null;
    const volume = typeof v?.volume === "number" ? Math.min(Math.max(v.volume, 0), 1) : 1;
    return { volume, muted: v?.muted === true };
  } catch {
    return { volume: 1, muted: false };
  }
}

/** m:ss.t, so playback visibly advances even in slow motion. */
const formatTime = (t: number) => `${formatDuration(t)}.${Math.floor((Math.max(0, t) % 1) * 10)}`;

function saveSpeed(v: number) {
  try {
    localStorage.setItem(SPEED_KEY, String(v));
  } catch {
    // Storage unavailable: the speed is just not remembered.
  }
}

const isTyping = (t: EventTarget | null) =>
  t instanceof HTMLElement && (t.isContentEditable || ["INPUT", "TEXTAREA", "SELECT"].includes(t.tagName));

export function ClipViewer({ clip, onClose, onChanged }: Props) {
  const video = useRef<HTMLVideoElement>(null);
  const [src, setSrc] = useState<string | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [duration, setDuration] = useState(clip.durationSecs ?? 0);
  const [range, setRange] = useState<[number, number]>([0, clip.durationSecs ?? 0]);
  const [playhead, setPlayhead] = useState(0);
  const [busy, setBusy] = useState<"trim" | "remux" | null>(null);
  const previewEnd = useRef<number | null>(null);
  const viewerSettings = useSettingsStore((s) => s.settings?.viewer);
  const keyMap = useMemo(() => resolveViewerKeys(viewerSettings), [viewerSettings]);
  const shortSeek = viewerSettings?.shortSeekSecs ?? 1;
  const longSeek = viewerSettings?.longSeekSecs ?? 5;
  const slowSpeed = viewerSettings?.slowMotionSpeed ?? 0.25;
  const [speed, setSpeedState] = useState(loadSpeed);
  const [help, setHelp] = useState(false);
  const [flash, setFlash] = useState<{ text: string; id: number } | null>(null);
  const setSpeed = (v: number) => {
    setSpeedState(v);
    saveSpeed(v);
  };
  const show = (text: string) => setFlash({ text, id: Date.now() });
  const [{ volume, muted }, setVolumeState] = useState(loadVolume);
  const setVolume = (p: Partial<{ volume: number; muted: boolean }>) =>
    setVolumeState((cur) => {
      const next = { ...cur, ...p };
      try {
        localStorage.setItem(VOLUME_KEY, JSON.stringify(next));
      } catch {
        // Not remembered; fine.
      }
      return next;
    });
  const [playing, setPlaying] = useState(false);
  const [fullscreen, setFullscreen] = useState(false);
  const player = useRef<HTMLDivElement>(null);
  const mixer = useLaneMixer(video, src, clip, muted ? 0 : volume);

  // While the lane mix plays, the video itself stays muted (see LaneMixer).
  useEffect(() => {
    const v = video.current;
    if (!v) return;
    v.muted = mixer.ready || muted;
    v.volume = volume;
  }, [mixer.ready, muted, volume, src]);

  // Smooth playhead while playing (timeupdate only fires ~4 times a second).
  useEffect(() => {
    if (!playing) return;
    let id = 0;
    const tick = () => {
      if (video.current) setPlayhead(video.current.currentTime);
      id = requestAnimationFrame(tick);
    };
    id = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(id);
  }, [playing]);

  useEffect(() => {
    const onFs = () => setFullscreen(document.fullscreenElement === player.current && player.current !== null);
    document.addEventListener("fullscreenchange", onFs);
    return () => document.removeEventListener("fullscreenchange", onFs);
  }, []);
  const togglePlay = () => {
    const v = video.current;
    if (!v) return;
    resumeAudio();
    if (v.paused) void v.play();
    else v.pause();
  };
  const toggleFullscreen = () => {
    if (document.fullscreenElement) void document.exitFullscreen();
    else void player.current?.requestFullscreen();
  };
  const markers = useMemo(() => [...clip.markers].sort((a, b) => a - b), [clip.markers]);
  const jumpMarker = (dir: 1 | -1) => {
    const now = video.current?.currentTime ?? playhead;
    // Small tolerance, so pressing again moves on instead of re-hitting the same marker.
    const m = dir > 0 ? markers.find((x) => x > now + 0.25) : [...markers].reverse().find((x) => x < now - 0.25);
    if (m === undefined) return show(markers.length ? "No more bookmarks" : "No bookmarks in this clip");
    seek(m);
    show(`Bookmark ${markers.indexOf(m) + 1}/${markers.length}`);
  };

  useEffect(() => {
    let alive = true;
    api.prepareClipPlayback(clip.path).then(
      (p) => alive && setSrc(convertFileSrc(p)),
      (err: unknown) => alive && setLoadError(errorMessage(err)),
    );
    return () => {
      alive = false;
    };
  }, [clip.path]);

  // Apply the playback speed; keep the audio pitch natural in slow motion.
  useEffect(() => {
    const v = video.current;
    if (!v) return;
    v.preservesPitch = true;
    v.playbackRate = speed;
  }, [speed, src]);

  useEffect(() => {
    if (!flash) return;
    const t = window.setTimeout(() => setFlash(null), 700);
    return () => window.clearTimeout(t);
  }, [flash]);

  const [start, end] = range;
  const seek = (t: number) => {
    if (video.current) video.current.currentTime = t;
    setPlayhead(t);
  };
  const preview = () => {
    const v = video.current;
    if (!v) return;
    previewEnd.current = end;
    v.currentTime = start;
    void v.play();
  };

  // Keyboard controls. Capture phase, so the native <video> controls do not
  // also react to the same keys; ignored while typing in a field.
  const keys = useRef<(e: KeyboardEvent) => void>(() => {});
  keys.current = (e: KeyboardEvent) => {
    if (e.key === "Escape") {
      if (help) setHelp(false);
      else onClose();
      return;
    }
    if (isTyping(e.target) || e.ctrlKey || e.altKey || e.metaKey) return;
    const v = video.current;
    const nudge = (d: number) => {
      if (!v) return;
      seek(Math.min(Math.max(v.currentTime + d, 0), duration || v.duration || 0));
      show(Math.abs(d) < 1 ? (d > 0 ? "+1 frame" : "−1 frame") : `${d > 0 ? "+" : "−"}${Math.round(Math.abs(d) * 10) / 10} s`);
    };
    const step = (dir: 1 | -1) => {
      const i = SPEEDS.indexOf(speed);
      const next = SPEEDS[Math.min(Math.max((i < 0 ? 4 : i) + dir, 0), SPEEDS.length - 1)];
      setSpeed(next);
      show(`${next}×`);
    };
    if (/^[0-9]$/.test(e.key)) {
      if (duration > 0) seek((Number(e.key) / 10) * duration);
      e.preventDefault();
      e.stopPropagation();
      return;
    }
    const action = actionForKey(keyMap, e.key);
    let handled = true;
    switch (action) {
      case "playPause":
        if (v) {
          show(v.paused ? "Play" : "Pause");
          togglePlay();
        }
        break;
      case "back":
        nudge(-(e.shiftKey ? longSeek : shortSeek));
        break;
      case "forward":
        nudge(e.shiftKey ? longSeek : shortSeek);
        break;
      case "backLong":
        nudge(-(e.shiftKey ? longSeek * 2 : longSeek));
        break;
      case "forwardLong":
        nudge(e.shiftKey ? longSeek * 2 : longSeek);
        break;
      case "frameBack":
        v?.pause();
        nudge(-FRAME);
        break;
      case "frameForward":
        v?.pause();
        nudge(FRAME);
        break;
      case "slowMotion": {
        const next = speed === 1 ? slowSpeed : 1;
        setSpeed(next);
        show(next === 1 ? "Normal speed" : `Slow motion ${next}×`);
        break;
      }
      case "slower":
        step(-1);
        break;
      case "faster":
        step(1);
        break;
      case "toStart":
        seek(0);
        break;
      case "toEnd":
        seek(duration);
        break;
      case "trimStart":
        setRange([Math.min(playhead, end - 0.5), end]);
        show("Trim start");
        break;
      case "trimEnd":
        setRange([start, Math.max(playhead, start + 0.5)]);
        show("Trim end");
        break;
      case "preview":
        preview();
        break;
      case "prevMarker":
        jumpMarker(-1);
        break;
      case "nextMarker":
        jumpMarker(1);
        break;
      case "mute":
        setVolume({ muted: !muted });
        show(muted ? "Sound on" : "Muted");
        break;
      case "fullscreen":
        toggleFullscreen();
        break;
      case "help":
        setHelp((h) => !h);
        break;
      default:
        handled = false;
    }
    if (handled) {
      e.preventDefault();
      e.stopPropagation();
    }
  };
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => keys.current(e);
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, []);

  const trim = async () => {
    setBusy("trim");
    const out = await attempt("Could not trim the clip", () => api.trimClip(clip.path, start, end));
    setBusy(null);
    if (out) {
      notify({ kind: "success", title: "Trimmed copy saved", body: out, action: { label: "Show in folder", run: () => void api.revealClip(out) } });
      onChanged();
    }
  };

  const remux = async () => {
    setBusy("remux");
    const out = await attempt("Could not remux the clip", () => api.remuxClip(clip.path));
    setBusy(null);
    if (out) {
      notify({ kind: "success", title: "Saved as MP4", body: out });
      onChanged();
    }
  };

  const del = async () => {
    const ok = await attempt("Could not delete the clip", () => api.deleteClip(clip.path).then(() => true));
    if (ok) {
      forgetThumbnail(clip.path);
      notify({ kind: "info", title: "Moved to the recycle bin", body: clip.fileName });
      onChanged();
      onClose();
    }
  };

  const isMp4 = /\.mp4$/i.test(clip.path);
  const trimmed = start > 0.05 || end < duration - 0.05;

  return (
    <div
      className="fixed inset-0 z-40 flex items-center justify-center bg-black/70 p-4"
      onPointerDown={(e) => {
        resumeAudio();
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div className="flex max-h-full w-full max-w-4xl flex-col overflow-hidden rounded-lg border border-zinc-800 bg-zinc-900 shadow-2xl">
        <header className="flex items-center gap-2 border-b border-zinc-800 px-4 py-2">
          <Badge tone={clip.kind === "clip" ? "sky" : "neutral"}>{clip.kind === "clip" ? "Clip" : "Recording"}</Badge>
          <h2 className="min-w-0 flex-1 truncate text-sm font-medium text-zinc-200 select-text" title={clip.path}>
            {clip.fileName}
          </h2>
          <span className="hidden shrink-0 text-xs text-zinc-500 sm:inline">
            {formatDate(clip.createdMs)} · {formatBytes(clip.sizeBytes)}
          </span>
          <DragHandle
            path={clip.path}
            label="Drag to share"
            title={
              (mixer.titles?.length ?? 0) > 1 && !mixer.hasMix
                ? `This older file has no mixed track: Discord only plays "${mixer.firstTrack ?? "the first lane"}". Use "Create share copy" below for all lanes.`
                : "Drag this file into Discord, a chat or a folder"
            }
          />
          <button onClick={onClose} className="rounded p-1 text-zinc-500 hover:bg-zinc-800 hover:text-zinc-200" aria-label="Close">
            <IconX size={16} />
          </button>
        </header>

        <div className="min-h-0 flex-1 overflow-y-auto">
          <div ref={player} className={fullscreen ? "flex h-full flex-col bg-black" : "bg-black"}>
          <div className={`relative flex w-full items-center justify-center bg-black ${fullscreen ? "min-h-0 flex-1" : "aspect-video max-h-[55vh]"}`}>
            {flash && (
              <div
                key={flash.id}
                className="pointer-events-none absolute top-3 left-1/2 z-10 -translate-x-1/2 rounded-md bg-black/70 px-3 py-1 font-mono text-sm text-white"
              >
                {flash.text}
              </div>
            )}
            {help && (
              <div className="absolute inset-0 z-20 flex items-center justify-center bg-black/80 p-4" onPointerDown={() => setHelp(false)}>
                <table className="text-sm">
                  <tbody>
                    {VIEWER_ACTIONS.map((a) => (
                      <tr key={a.id}>
                        <td className="py-0.5 pr-6 text-right font-mono text-sky-300">
                          {keyMap[a.id].length ? keyMap[a.id].map(keyLabel).join(" / ") : "—"}
                        </td>
                        <td className="py-0.5 text-zinc-200">{a.label}</td>
                      </tr>
                    ))}
                    <tr>
                      <td className="py-0.5 pr-6 text-right font-mono text-sky-300">0–9 / Esc</td>
                      <td className="py-0.5 text-zinc-200">Jump to 0–90 % / close</td>
                    </tr>
                    <tr>
                      <td />
                      <td className="pt-2 text-xs text-zinc-500">Change these in Settings → Clip viewer</td>
                    </tr>
                  </tbody>
                </table>
              </div>
            )}
            {loadError ? (
              <p className="px-6 text-center text-sm text-red-400">Cannot play this file: {loadError}</p>
            ) : !src ? (
              <span className="flex items-center gap-2 text-sm text-zinc-500">
                <Spinner /> Preparing playback…
              </span>
            ) : (
              <video
                ref={video}
                src={src}
                autoPlay
                onClick={togglePlay}
                onPlay={() => setPlaying(true)}
                onPause={() => setPlaying(false)}
                onEnded={() => setPlaying(false)}
                className="h-full w-full cursor-pointer"
                onLoadedMetadata={(e) => {
                  const d = e.currentTarget.duration;
                  if (Number.isFinite(d) && d > 0) {
                    setDuration(d);
                    setRange(([s, en]) => [Math.min(s, d), en <= 0 || en > d ? d : en]);
                  }
                }}
                onTimeUpdate={(e) => {
                  const t = e.currentTarget.currentTime;
                  setPlayhead(t);
                  if (previewEnd.current !== null && t >= previewEnd.current) {
                    previewEnd.current = null;
                    e.currentTarget.pause();
                  }
                }}
                onSeeking={() => (previewEnd.current = null)}
              />
            )}
          </div>
          <div className="bg-zinc-950 px-3 pt-1">
            <SeekBar duration={duration} time={playhead} markers={markers} onSeek={seek} />
          </div>
          <div className="flex items-center gap-3 border-b border-zinc-800 bg-zinc-950 px-3 pb-1.5 text-xs text-zinc-300">
            <button type="button" onClick={togglePlay} disabled={!src} className="rounded p-1 hover:bg-zinc-800" title={playing ? "Pause" : "Play"}>
              {playing ? <span className="block h-3.5 w-3.5 border-x-4 border-current" /> : <IconPlay size={14} />}
            </button>
            <span className="font-mono tabular-nums">
              {formatTime(playhead)} <span className="text-zinc-500">/ {formatTime(duration)}</span>
            </span>
            <span className="flex-1" />
            <button
              type="button"
              onClick={() => setVolume({ muted: !muted })}
              className={`rounded px-1.5 py-0.5 ${muted ? "bg-red-600/80 text-white" : "hover:bg-zinc-800"}`}
              title={muted ? "Unmute" : "Mute"}
            >
              {muted ? "Muted" : "Vol"}
            </button>
            <input
              type="range"
              min={0}
              max={1}
              step={0.01}
              value={volume}
              onChange={(e) => setVolume({ volume: Number(e.target.value), muted: false })}
              className="w-24 accent-sky-500"
              aria-label="Volume"
            />
            <button type="button" onClick={toggleFullscreen} className="rounded px-1.5 py-0.5 hover:bg-zinc-800" title="Fullscreen (F)">
              {fullscreen ? "Exit fullscreen" : "Fullscreen"}
            </button>
          </div>
          </div>

          <div className="flex flex-wrap items-center gap-1 border-b border-zinc-800 px-4 py-2 text-xs">
            <span className="mr-1 text-zinc-500">Speed</span>
            {SPEEDS.map((x) => (
              <button
                key={x}
                onClick={() => setSpeed(x)}
                className={`rounded px-2 py-0.5 font-mono ${
                  speed === x ? "bg-sky-600 text-white" : "text-zinc-400 hover:bg-zinc-800 hover:text-zinc-200"
                }`}
              >
                {x}×
              </button>
            ))}
            {speed < 1 && <span className="ml-2 text-sky-400">slow motion</span>}
            <button onClick={() => setHelp((h) => !h)} className="ml-auto text-zinc-500 hover:text-zinc-200" title="Keyboard shortcuts">
              Shortcuts <kbd className="rounded border border-zinc-700 px-1 font-mono">?</kbd>
            </button>
          </div>

          <LaneMixerPanel mixer={mixer} />

          <div className="space-y-3 p-4">
            <div className="flex items-center gap-2 text-sm text-zinc-300">
              <IconScissors size={14} /> Trim
              <span className="ml-auto font-mono text-xs text-zinc-500">
                {formatDuration(start)} – {formatDuration(end)} · {(end - start).toFixed(1)} s
              </span>
            </div>
            {duration > 0 ? (
              <TrimBar
                duration={duration}
                start={start}
                end={end}
                playhead={playhead}
                markers={markers}
                onSeek={seek}
                onChange={(s, e) => setRange([s, e])}
              />
            ) : (
              <p className="text-xs text-zinc-500">Waiting for the video length…</p>
            )}
            <div className="flex flex-wrap items-center gap-2 text-xs">
              <span className="text-zinc-500">Start</span>
              <NumberInput
                value={round1(start)}
                integer={false}
                step={0.1}
                min={0}
                max={Math.max(0, end - 0.5)}
                suffix="s"
                onChange={(s) => setRange([s, end])}
              />
              <Button size="sm" variant="ghost" onClick={() => setRange([Math.min(playhead, end - 0.5), end])}>
                Set to playhead
              </Button>
              <span className="ml-2 text-zinc-500">End</span>
              <NumberInput
                value={round1(end)}
                integer={false}
                step={0.1}
                min={start + 0.5}
                max={duration}
                suffix="s"
                onChange={(e) => setRange([start, e])}
              />
              <Button size="sm" variant="ghost" onClick={() => setRange([start, Math.max(playhead, start + 0.5)])}>
                Set to playhead
              </Button>
              <Button size="sm" onClick={preview} disabled={!src} className="ml-auto">
                Preview selection
              </Button>
            </div>
            {markers.length > 0 && (
              <div className="flex flex-wrap items-center gap-1.5 text-xs">
                <span className="text-zinc-500">Bookmarks</span>
                {markers.map((m, i) => (
                  <span key={i} className="flex items-center overflow-hidden rounded-md border border-amber-500/40">
                    <button type="button" onClick={() => seek(m)} className="px-2 py-0.5 font-mono text-amber-300 hover:bg-amber-500/10" title="Jump here">
                      {formatDuration(m)}
                    </button>
                    <button
                      type="button"
                      title="Trim to 10 s before and 5 s after this bookmark"
                      onClick={() => {
                        const s = Math.max(0, m - 10);
                        setRange([s, Math.min(duration || m + 5, Math.max(m + 5, s + 0.5))]);
                        seek(s);
                      }}
                      className="border-l border-amber-500/40 px-1.5 py-0.5 text-zinc-400 hover:bg-amber-500/10 hover:text-zinc-200"
                    >
                      <IconScissors size={12} />
                    </button>
                  </span>
                ))}
              </div>
            )}
            <Callout>
              Trimming is lossless and instant: cuts snap to the nearest keyframe, so the copy may start up to a keyframe
              interval earlier than the selection.
            </Callout>
          </div>
        </div>

        <SharePanel clip={clip} mix={mixer.choice} mixed={mixer.changed} hasAudio={(mixer.titles?.length ?? 0) > 0} onSaved={onChanged} />

        <footer className="flex flex-wrap items-center gap-2 border-t border-zinc-800 px-4 py-2.5">
          <Button size="sm" variant="primary" disabled={!trimmed || busy !== null || duration <= 0} onClick={() => void trim()}>
            {busy === "trim" ? <Spinner /> : <IconScissors size={14} />} Save trimmed copy
          </Button>
          {!isMp4 && (
            <Button size="sm" disabled={busy !== null} onClick={() => void remux()} title="Copy the streams into an MP4 container (no re-encode)">
              {busy === "remux" && <Spinner />} Remux to MP4
            </Button>
          )}
          <Button size="sm" variant="ghost" onClick={() => void attempt("Could not open the folder", () => api.revealClip(clip.path))}>
            <IconFolder size={14} /> Show in folder
          </Button>
          <div className="ml-auto">
            <ConfirmButton onConfirm={() => void del()} confirmLabel="Click again to delete">
              Delete
            </ConfirmButton>
          </div>
        </footer>
      </div>
    </div>
  );
}
