import { useRef, type PointerEvent } from "react";
import { clamp } from "../../lib/format";

interface Props {
  duration: number;
  start: number;
  end: number;
  playhead: number;
  onChange: (start: number, end: number) => void;
  onSeek: (t: number) => void;
}

const MIN_LEN = 0.5;

/** A timeline with two draggable handles (start/end) and a playhead. Click the track to seek. */
export function TrimBar({ duration, start, end, playhead, onChange, onSeek }: Props) {
  const track = useRef<HTMLDivElement>(null);
  const dragging = useRef<"start" | "end" | null>(null);

  const timeAt = (clientX: number) => {
    const r = track.current?.getBoundingClientRect();
    if (!r || r.width === 0 || duration <= 0) return 0;
    return clamp(((clientX - r.left) / r.width) * duration, 0, duration);
  };

  const move = (e: PointerEvent) => {
    const which = dragging.current;
    if (!which) return;
    const t = timeAt(e.clientX);
    if (which === "start") {
      const s = Math.min(t, end - MIN_LEN);
      onChange(Math.max(0, s), end);
      onSeek(Math.max(0, s));
    } else {
      const en = Math.max(t, start + MIN_LEN);
      onChange(start, Math.min(duration, en));
      onSeek(Math.min(duration, en));
    }
  };

  const pct = (t: number) => `${duration > 0 ? (t / duration) * 100 : 0}%`;

  const handle = (which: "start" | "end") => (
    <div
      role="slider"
      aria-label={which === "start" ? "Trim start" : "Trim end"}
      aria-valuenow={which === "start" ? start : end}
      onPointerDown={(e) => {
        e.stopPropagation();
        dragging.current = which;
        (e.target as HTMLElement).setPointerCapture(e.pointerId);
      }}
      onPointerMove={move}
      onPointerUp={() => (dragging.current = null)}
      className="absolute top-0 bottom-0 z-10 w-3 -translate-x-1/2 cursor-ew-resize touch-none rounded-sm bg-sky-400 shadow hover:bg-sky-300"
      style={{ left: pct(which === "start" ? start : end) }}
    />
  );

  return (
    <div
      ref={track}
      onPointerDown={(e) => onSeek(timeAt(e.clientX))}
      className="relative h-8 cursor-pointer rounded-md bg-zinc-800"
    >
      {/* Kept region */}
      <div className="absolute top-0 bottom-0 bg-sky-500/20" style={{ left: pct(start), width: `calc(${pct(end)} - ${pct(start)})` }} />
      {/* Playhead */}
      <div className="absolute top-0 bottom-0 w-0.5 bg-zinc-100" style={{ left: pct(playhead) }} />
      {handle("start")}
      {handle("end")}
    </div>
  );
}
