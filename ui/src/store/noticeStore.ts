import { create } from "zustand";
import { errorMessage } from "../lib/ipc";

export type NoticeKind = "info" | "success" | "error";

export interface Notice {
  id: number;
  kind: NoticeKind;
  title: string;
  body?: string;
  /** Optional action button, e.g. "Show in folder". */
  action?: { label: string; run: () => void };
}

interface NoticeState {
  notices: Notice[];
  push: (n: Omit<Notice, "id">, ttlMs?: number) => void;
  dismiss: (id: number) => void;
}

let seq = 0;

/** In-app toasts (bottom-right). */
export const useNoticeStore = create<NoticeState>((set, get) => ({
  notices: [],
  push: (n, ttlMs = 5000) => {
    const id = ++seq;
    set((s) => ({ notices: [...s.notices, { ...n, id }].slice(-4) }));
    setTimeout(() => get().dismiss(id), ttlMs);
  },
  dismiss: (id) => set((s) => ({ notices: s.notices.filter((x) => x.id !== id) })),
}));

export const notify = (n: Omit<Notice, "id">, ttlMs?: number) => useNoticeStore.getState().push(n, ttlMs);

/** Shows a failed command as an error toast. */
export const reportError = (title: string, err: unknown) =>
  notify({ kind: "error", title, body: errorMessage(err) }, 8000);

/** Runs a command, toasting its error. Resolves to undefined on failure. */
export async function attempt<T>(title: string, run: () => Promise<T>): Promise<T | undefined> {
  try {
    return await run();
  } catch (err) {
    reportError(title, err);
    return undefined;
  }
}
