import { create } from "zustand";
import { api, events, type LogEntry } from "../lib/ipc";

const MAX_ENTRIES = 500;

interface LogState {
  entries: LogEntry[];
  /** Subscribes to live logs and loads the backend's history. Returns an unsubscribe function. */
  connect: () => () => void;
  clear: () => void;
}

/** Merges entries by sequence number, keeping order and dropping duplicates. */
function merge(current: LogEntry[], incoming: LogEntry[]): LogEntry[] {
  const bySeq = new Map<number, LogEntry>();
  for (const e of current) bySeq.set(e.seq, e);
  for (const e of incoming) bySeq.set(e.seq, e);
  return [...bySeq.values()].sort((a, b) => a.seq - b.seq).slice(-MAX_ENTRIES);
}

export const useLogStore = create<LogState>((set) => ({
  entries: [],
  connect: () => {
    // Listen first, then fetch history, so nothing logged in between is lost.
    const unlisten = events.onLog((e) =>
      set((s) => ({
        entries:
          s.entries.length > 0 && e.seq > s.entries[s.entries.length - 1].seq
            ? [...s.entries, e].slice(-MAX_ENTRIES)
            : merge(s.entries, [e]),
      })),
    );
    api.getLogs().then((history) => set((s) => ({ entries: merge(s.entries, history) })));
    return () => void unlisten.then((f) => f());
  },
  clear: () => set({ entries: [] }),
}));
