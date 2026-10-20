// Live level meters. They never re-render on meter frames: a painter writes
// transforms directly to the DOM (see lib/meters.ts).

import { useEffect, useRef } from "react";
import { addPainter, dbToFraction, laneLevel, sourceLevel } from "../lib/meters";
import type { Id } from "../lib/ipc";
import { cx } from "./ui";

/** Peak hold falls back after this long. */
const HOLD_MS = 1200;

interface MeterProps {
  className?: string;
  /** Show the -60..0 dBFS scale ticks under the bar. */
  scale?: boolean;
}

function useMeterPainter(read: () => { peak: number; rms: number | null }) {
  const peakCover = useRef<HTMLDivElement>(null);
  const rmsCover = useRef<HTMLDivElement>(null);
  const hold = useRef<HTMLDivElement>(null);
  const readRef = useRef(read);
  readRef.current = read;

  useEffect(() => {
    let held = 0;
    let heldAt = 0;
    let lastPeak = -1;
    let lastRms = -1;
    let lastHeld = -1;
    return addPainter(() => {
      const { peak, rms } = readRef.current();
      const p = dbToFraction(peak);
      const r = rms === null ? p : dbToFraction(rms);
      const now = performance.now();
      if (p >= held || p === 0 || now - heldAt > HOLD_MS) {
        held = p;
        heldAt = now;
      }
      // Skip DOM writes when nothing changed (common when silent).
      if (p !== lastPeak && peakCover.current) peakCover.current.style.transform = `scaleX(${1 - p})`;
      if (r !== lastRms && rmsCover.current) rmsCover.current.style.transform = `scaleX(${1 - r})`;
      if (held !== lastHeld && hold.current) {
        hold.current.style.left = `calc(${held * 100}% - 1px)`;
        hold.current.style.opacity = held > 0 ? "1" : "0";
      }
      lastPeak = p;
      lastRms = r;
      lastHeld = held;
    });
  }, []);

  return { peakCover, rmsCover, hold };
}

function MeterBar({
  refs,
  height,
  className,
  scale,
}: {
  refs: ReturnType<typeof useMeterPainter>;
  height: string;
} & MeterProps) {
  return (
    <div className={cx("min-w-0", className)}>
      <div className={cx("lanes-meter relative overflow-hidden rounded-sm", height)}>
        {/* RMS shading sits under the peak cover: visible between rms and peak. */}
        <div ref={refs.rmsCover} className="absolute inset-0 origin-right bg-zinc-950/45" style={{ transform: "scaleX(1)" }} />
        <div ref={refs.peakCover} className="absolute inset-0 origin-right bg-zinc-800" style={{ transform: "scaleX(1)" }} />
        <div ref={refs.hold} className="absolute top-0 bottom-0 w-0.5 bg-zinc-100/80" style={{ opacity: 0 }} />
      </div>
      {scale && (
        <div className="relative mt-0.5 h-3 font-mono text-[9px] text-zinc-600">
          {[-60, -48, -36, -24, -12, -6, 0].map((db) => (
            <span
              key={db}
              className="absolute"
              style={{
                left: `${dbToFraction(db) * 100}%`,
                transform: db === 0 ? "translateX(-100%)" : db === -60 ? "none" : "translateX(-50%)",
              }}
            >
              {db}
            </span>
          ))}
        </div>
      )}
    </div>
  );
}

/** Lane meter: RMS body with brighter peak extension and a peak-hold tick. */
export function LaneMeter({ id, ...rest }: { id: Id } & MeterProps) {
  const refs = useMeterPainter(() => {
    const l = laneLevel(id);
    return { peak: l.peakDb, rms: l.rmsDb };
  });
  return <MeterBar refs={refs} height="h-2" {...rest} />;
}

/** Compact source meter (peak only). */
export function SourceMeter({ id, ...rest }: { id: Id } & MeterProps) {
  const refs = useMeterPainter(() => ({ peak: sourceLevel(id), rms: null }));
  return <MeterBar refs={refs} height="h-1" {...rest} />;
}
