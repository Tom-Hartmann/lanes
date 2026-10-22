import { useRef, useState, type PointerEvent } from "react";
import { clamp, formatDuration } from "../../lib/format";

interface Props {
  duration: number;
  time: number;
  /** Bookmarks (seconds). */
  markers?: number[];
  onSeek: (t: number) => void;
}

/** Player progress bar: click or drag to jump, hover shows the time there. */
export function SeekBar({ duration, time, markers = [], onSeek }: Props) {
  const track = useRef<HTMLDivElement>(null);
  const dragging = useRef(false);
  const [hover, setHover] = useState<{ x: number; t: number } | null>(null);

  const at = (clientX: number) => {
    const r = track.current?.getBoundingClientRect();
    if (!r || r.width === 0 || duration <= 0) return null;
    const x = clamp(clientX - r.left, 0, r.width);
    return { x, t: (x / r.width) * duration };
  };
  const move = (e: PointerEvent) => {
    const p = at(e.clientX);
    setHover(p);
    if (p && dragging.current) onSeek(p.t);
  };
  const pct = (t: number) => `${duration > 0 ? clamp(t / duration, 0, 1) * 100 : 0}%`;

  return (
    <div
      ref={track}
      role="slider"
      aria-label="Seek"
      aria-valuemin={0}
      aria-valuemax={duration}
      aria-valuenow={time}
      onPointerDown={(e) => {
        dragging.current = true;
        e.currentTarget.setPointerCapture(e.pointerId);
        const p = at(e.clientX);
        if (p) onSeek(p.t);
      }}
      onPointerMove={move}
      onPointerUp={() => (dragging.current = false)}
      onPointerLeave={() => setHover(null)}
      className="group relative flex h-4 cursor-pointer touch-none items-center"
    >
      <div className="relative h-1 w-full rounded-full bg-zinc-700 transition-[height] group-hover:h-1.5">
        {hover && <div className="absolute inset-y-0 left-0 rounded-full bg-zinc-500/60" style={{ width: hover.x }} />}
        <div className="absolute inset-y-0 left-0 rounded-full bg-sky-500" style={{ width: pct(time) }} />
        {markers.map((m, i) => (
          <div key={i} className="absolute inset-y-0 w-1 -translate-x-1/2 bg-amber-400" style={{ left: pct(m) }} />
        ))}
      </div>
      <div
        className="pointer-events-none absolute h-3 w-3 -translate-x-1/2 scale-0 rounded-full bg-sky-400 transition-transform group-hover:scale-100"
        style={{ left: pct(time) }}
      />
      {hover && (
        <div
          className="pointer-events-none absolute bottom-4 -translate-x-1/2 rounded bg-black/80 px-1.5 py-0.5 font-mono text-[11px] text-white"
          style={{ left: hover.x }}
        >
          {formatDuration(hover.t)}
        </div>
      )}
    </div>
  );
}
