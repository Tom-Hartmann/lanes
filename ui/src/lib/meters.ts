// Meter bus: the ~20 Hz `meters` event is kept out of React state. Meter
// components register a DOM-writing painter; one requestAnimationFrame per
// incoming frame runs them. With no meter on screen nothing is scheduled.

import { events, type Id } from "./ipc";

export interface LaneLevel {
  peakDb: number;
  rmsDb: number;
}

const SILENCE = -100;
const STALE_MS = 400;
const SILENT_LEVEL: LaneLevel = { peakDb: SILENCE, rmsDb: SILENCE };

const lanes = new Map<Id, LaneLevel>();
const sources = new Map<Id, number>();
const painters = new Set<() => void>();
let rafId = 0;
let staleTimer: ReturnType<typeof setTimeout> | null = null;

function schedule() {
  if (rafId || painters.size === 0) return;
  rafId = requestAnimationFrame(() => {
    rafId = 0;
    painters.forEach((p) => p());
  });
}

/** Subscribes to the backend meter event. Call once at app start. */
export function connectMeters(): () => void {
  const unlisten = events.onMeters((frame) => {
    lanes.clear();
    sources.clear();
    for (const l of frame.lanes) lanes.set(l.id, { peakDb: l.peakDb, rmsDb: l.rmsDb });
    for (const s of frame.sources) sources.set(s.id, s.peakDb);
    schedule();
    // When frames stop (engine idle), drop meters to silence instead of freezing.
    if (staleTimer) clearTimeout(staleTimer);
    staleTimer = setTimeout(() => {
      lanes.clear();
      sources.clear();
      schedule();
    }, STALE_MS);
  });
  return () => void unlisten.then((f) => f());
}

export const laneLevel = (id: Id): LaneLevel => lanes.get(id) ?? SILENT_LEVEL;
export const sourceLevel = (id: Id): number => sources.get(id) ?? SILENCE;

/** Registers a painter run after each meter frame. Returns an unregister function. */
export function addPainter(paint: () => void): () => void {
  painters.add(paint);
  paint();
  return () => {
    painters.delete(paint);
  };
}

/** dBFS on the -60..0 meter scale, as 0..1. */
export const dbToFraction = (db: number) => (db <= -60 ? 0 : db >= 0 ? 1 : (db + 60) / 60);
