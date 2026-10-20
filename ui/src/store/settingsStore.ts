import { create } from "zustand";
import { api, errorMessage, events, type Profile, type Settings } from "../lib/ipc";
import { findActiveProfile, withActiveProfile, withProfile } from "../lib/settings";

/** "now" saves immediately (toggles, buttons); "debounced" waits ~400 ms (sliders, text). */
export type SaveMode = "now" | "debounced";

const DEBOUNCE_MS = 400;

interface SettingsState {
  settings: Settings | null;
  loadError: string | null;
  saveError: string | null;
  load: () => Promise<void>;
  /** Subscribes to backend `settings` events. Returns an unsubscribe function. */
  connect: () => () => void;
  /** Applies an immutable edit locally and persists it with updateSettings. */
  update: (fn: (s: Settings) => Settings, mode?: SaveMode) => void;
  updateActiveProfile: (fn: (p: Profile) => Profile, mode?: SaveMode) => void;
  updateProfile: (id: string, fn: (p: Profile) => Profile, mode?: SaveMode) => void;
  /** Saves pending debounced edits right away. */
  flush: () => Promise<void>;
  clearSaveError: () => void;
}

let timer: ReturnType<typeof setTimeout> | null = null;
/** Incremented on every local edit; a save result is applied only if no newer edit happened. */
let editSeq = 0;
let inflight = 0;
let chain: Promise<void> = Promise.resolve();

export const useSettingsStore = create<SettingsState>((set, get) => {
  const persist = (): Promise<void> => {
    if (timer) {
      clearTimeout(timer);
      timer = null;
    }
    const snapshot = get().settings;
    if (!snapshot) return chain;
    const seq = editSeq;
    inflight++;
    // Serialize saves so they reach the backend in edit order.
    chain = chain.then(async () => {
      try {
        const stored = await api.updateSettings(snapshot);
        if (seq === editSeq) set({ settings: stored, saveError: null });
      } catch (err) {
        set({ saveError: errorMessage(err) });
        // Fall back to what the backend actually has, unless the user kept editing.
        if (seq === editSeq) {
          try {
            const current = await api.getSettings();
            if (seq === editSeq) set({ settings: current });
          } catch {
            /* keep the local copy */
          }
        }
      } finally {
        inflight--;
      }
    });
    return chain;
  };

  const update: SettingsState["update"] = (fn, mode = "now") => {
    const s = get().settings;
    if (!s) return;
    editSeq++;
    set({ settings: fn(s) });
    if (mode === "debounced") {
      if (timer) clearTimeout(timer);
      timer = setTimeout(() => void persist(), DEBOUNCE_MS);
    } else {
      void persist();
    }
  };

  return {
    settings: null,
    loadError: null,
    saveError: null,
    load: async () => {
      try {
        const settings = await api.getSettings();
        if (timer === null && inflight === 0) set({ settings, loadError: null });
      } catch (err) {
        set({ loadError: errorMessage(err) });
      }
    },
    connect: () => {
      const unlisten = events.onSettings((settings) => {
        // Ignore echoes while local edits are pending; the save result wins.
        if (timer === null && inflight === 0) set({ settings });
      });
      return () => void unlisten.then((f) => f());
    },
    update,
    updateActiveProfile: (fn, mode) => update((s) => withActiveProfile(s, fn), mode),
    updateProfile: (id, fn, mode) => update((s) => withProfile(s, id, fn), mode),
    flush: () => (timer ? persist() : chain),
    clearSaveError: () => set({ saveError: null }),
  };
});

/** The active profile, or undefined while settings load. */
export const useActiveProfile = (): Profile | undefined =>
  useSettingsStore((s) => (s.settings ? findActiveProfile(s.settings) : undefined));
