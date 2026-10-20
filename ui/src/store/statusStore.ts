import { create } from "zustand";
import { api, events, type ClipSaved, type EngineError, type EngineStatus, type Stats } from "../lib/ipc";

export interface TimedError extends EngineError {
  id: number;
  atMs: number;
}

const MAX_ERRORS = 20;
let errorSeq = 0;

interface StatusState {
  status: EngineStatus | null;
  /** Latest 1 Hz stats frame; null until capture starts. */
  stats: Stats | null;
  statsAtMs: number;
  errors: TimedError[];
  /** Bumped on every clip_saved event so lists can refresh. */
  clipSeq: number;
  lastClip: ClipSaved | null;
  connect: () => () => void;
  refresh: () => Promise<void>;
  clearErrors: () => void;
  dismissError: (id: number) => void;
}

export const useStatusStore = create<StatusState>((set) => ({
  status: null,
  stats: null,
  statsAtMs: 0,
  errors: [],
  clipSeq: 0,
  lastClip: null,
  connect: () => {
    const subs = [
      events.onStatus((status) => set({ status })),
      events.onStats((stats) => set({ stats, statsAtMs: Date.now() })),
      events.onError((e) =>
        set((s) => ({ errors: [{ ...e, id: ++errorSeq, atMs: Date.now() }, ...s.errors].slice(0, MAX_ERRORS) })),
      ),
      events.onClipSaved((lastClip) => set((s) => ({ lastClip, clipSeq: s.clipSeq + 1 }))),
      // Recordings land in the library too; bump the sequence so it refreshes.
      events.onRecordingSaved(() => set((s) => ({ clipSeq: s.clipSeq + 1 }))),
    ];
    api.getStatus().then(
      (status) => set({ status }),
      () => {},
    );
    return () => subs.forEach((p) => void p.then((f) => f()));
  },
  refresh: async () => set({ status: await api.getStatus() }),
  clearErrors: () => set({ errors: [] }),
  dismissError: (id) => set((s) => ({ errors: s.errors.filter((e) => e.id !== id) })),
}));

export const useIsRecording = () => useStatusStore((s) => s.status?.recording.state === "recording");
