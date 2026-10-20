import { useEffect, useRef } from "react";
import { useLogStore } from "../store/logStore";
import type { LogEntry } from "../lib/ipc";

const LEVEL_STYLE: Record<LogEntry["level"], string> = {
  ERROR: "text-red-400",
  WARN: "text-amber-400",
  INFO: "text-sky-400",
  DEBUG: "text-zinc-500",
  TRACE: "text-zinc-600",
};

const time = (ms: number) =>
  new Date(ms).toLocaleTimeString(undefined, { hour12: false }) + "." + String(ms % 1000).padStart(3, "0");

export function LogPanel() {
  const entries = useLogStore((s) => s.entries);
  const clear = useLogStore((s) => s.clear);
  const bottom = useRef<HTMLDivElement>(null);

  useEffect(() => bottom.current?.scrollIntoView({ block: "end" }), [entries.length]);

  return (
    <section className="flex min-h-0 flex-1 flex-col rounded-lg border border-zinc-800 bg-zinc-900/60">
      <header className="flex items-center justify-between border-b border-zinc-800 px-4 py-2">
        <h2 className="text-sm font-medium text-zinc-300">Log</h2>
        <button onClick={clear} className="text-xs text-zinc-500 hover:text-zinc-300">
          Clear
        </button>
      </header>
      <div className="min-h-0 flex-1 overflow-y-auto px-4 py-2 font-mono text-xs leading-5 select-text">
        {entries.length === 0 && <p className="text-zinc-600">No log entries yet.</p>}
        {entries.map((e) => (
          <div key={e.seq} className="flex gap-3">
            <span className="shrink-0 text-zinc-600">{time(e.timeMs)}</span>
            <span className={`w-10 shrink-0 ${LEVEL_STYLE[e.level]}`}>{e.level}</span>
            <span className="break-all text-zinc-300">{e.message}</span>
          </div>
        ))}
        <div ref={bottom} />
      </div>
    </section>
  );
}
