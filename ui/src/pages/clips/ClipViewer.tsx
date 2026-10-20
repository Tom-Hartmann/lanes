import { convertFileSrc } from "@tauri-apps/api/core";
import { useEffect, useRef, useState } from "react";
import { IconFolder, IconScissors, IconX } from "../../components/icons";
import { Badge, Button, Callout, ConfirmButton, NumberInput, Spinner } from "../../components/ui";
import { formatBytes, formatDate, formatDuration } from "../../lib/format";
import { api, errorMessage, type ClipInfo } from "../../lib/ipc";
import { attempt, notify } from "../../store/noticeStore";
import { forgetThumbnail } from "./thumbnails";
import { SharePanel } from "./SharePanel";
import { TrimBar } from "./TrimBar";

interface Props {
  clip: ClipInfo;
  onClose: () => void;
  /** The library changed (new trimmed/remuxed file, deleted clip). */
  onChanged: () => void;
}

const round1 = (n: number) => Math.round(n * 10) / 10;

export function ClipViewer({ clip, onClose, onChanged }: Props) {
  const video = useRef<HTMLVideoElement>(null);
  const [src, setSrc] = useState<string | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [duration, setDuration] = useState(clip.durationSecs ?? 0);
  const [range, setRange] = useState<[number, number]>([0, clip.durationSecs ?? 0]);
  const [playhead, setPlayhead] = useState(0);
  const [busy, setBusy] = useState<"trim" | "remux" | null>(null);
  const previewEnd = useRef<number | null>(null);

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

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

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
    <div className="fixed inset-0 z-40 flex items-center justify-center bg-black/70 p-4" onPointerDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className="flex max-h-full w-full max-w-4xl flex-col overflow-hidden rounded-lg border border-zinc-800 bg-zinc-900 shadow-2xl">
        <header className="flex items-center gap-2 border-b border-zinc-800 px-4 py-2">
          <Badge tone={clip.kind === "clip" ? "sky" : "neutral"}>{clip.kind === "clip" ? "Clip" : "Recording"}</Badge>
          <h2 className="min-w-0 flex-1 truncate text-sm font-medium text-zinc-200 select-text" title={clip.path}>
            {clip.fileName}
          </h2>
          <span className="hidden shrink-0 text-xs text-zinc-500 sm:inline">
            {formatDate(clip.createdMs)} · {formatBytes(clip.sizeBytes)}
          </span>
          <button onClick={onClose} className="rounded p-1 text-zinc-500 hover:bg-zinc-800 hover:text-zinc-200" aria-label="Close">
            <IconX size={16} />
          </button>
        </header>

        <div className="min-h-0 flex-1 overflow-y-auto">
          <div className="flex aspect-video max-h-[55vh] w-full items-center justify-center bg-black">
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
                controls
                autoPlay
                className="h-full w-full"
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
            <Callout>
              Trimming is lossless and instant: cuts snap to the nearest keyframe, so the copy may start up to a keyframe
              interval earlier than the selection.
            </Callout>
          </div>
        </div>

        <SharePanel clip={clip} />

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
