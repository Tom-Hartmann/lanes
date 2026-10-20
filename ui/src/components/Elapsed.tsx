import { useEffect, useRef, useState } from "react";
import { formatDuration } from "../lib/format";

/**
 * Converts the backend's `sinceMs` to a local wall-clock start. Values that look
 * like Unix epoch milliseconds are used directly; anything else (e.g. an engine
 * monotonic clock) is anchored to the moment this UI first saw it.
 */
const anchors = new Map<number, number>();
function startOf(sinceMs: number): number {
  if (sinceMs > 1e12) return sinceMs;
  let a = anchors.get(sinceMs);
  if (a === undefined) {
    a = Date.now();
    anchors.set(sinceMs, a);
  }
  return a;
}

/** Recording timer. Re-renders only itself, once per second, and freezes while paused. */
export function Elapsed({ sinceMs, paused, className }: { sinceMs: number; paused: boolean; className?: string }) {
  const [, tick] = useState(0);
  const frozen = useRef<number | null>(null);
  const start = startOf(sinceMs);
  const live = Math.max(0, (Date.now() - start) / 1000);

  if (paused && frozen.current === null) frozen.current = live;
  if (!paused) frozen.current = null;

  useEffect(() => {
    if (paused) return;
    const t = setInterval(() => tick((n) => n + 1), 1000);
    return () => clearInterval(t);
  }, [paused]);

  return <span className={className}>{formatDuration(frozen.current ?? live)}</span>;
}
