import { create } from "zustand";
import { api, events, type BindingInfo, type HotkeyBackend } from "../lib/ipc";

interface HotkeyState {
  bindings: BindingInfo[];
  load: () => Promise<void>;
  /** Follows the backend's `hotkeys` event (sent when presets or bindings change). */
  connect: () => () => void;
  /** Rebinds an action. Rejects with the backend's message if the combo cannot be bound. */
  set: (action: string, combo: string | null, backend: HotkeyBackend) => Promise<void>;
}

export const useHotkeyStore = create<HotkeyState>((set) => ({
  bindings: [],
  load: async () => set({ bindings: await api.listHotkeys() }),
  connect: () => {
    const unlisten = events.onHotkeys((bindings) => set({ bindings }));
    return () => void unlisten.then((f) => f());
  },
  set: async (action, combo, backend) => {
    try {
      await api.setHotkey(action, combo, backend);
    } finally {
      // Reload either way: a failed bind restores the previous combo.
      set({ bindings: await api.listHotkeys() });
    }
  },
}));
