// Clip viewer keyboard actions and their default keys. Settings store only
// the actions the user changed (settings.viewer.keys); everything else uses
// these defaults.

import type { Settings } from "./ipc";

export interface ViewerAction {
  id: string;
  label: string;
  defaults: string[];
}

export const VIEWER_ACTIONS: ViewerAction[] = [
  { id: "playPause", label: "Play / pause", defaults: [" ", "k"] },
  { id: "back", label: "Back (short step, Shift: long)", defaults: ["ArrowLeft"] },
  { id: "forward", label: "Forward (short step, Shift: long)", defaults: ["ArrowRight"] },
  { id: "backLong", label: "Back (long step, Shift: double)", defaults: ["j"] },
  { id: "forwardLong", label: "Forward (long step, Shift: double)", defaults: ["l"] },
  { id: "frameBack", label: "Previous frame", defaults: [","] },
  { id: "frameForward", label: "Next frame", defaults: ["."] },
  { id: "slowMotion", label: "Slow motion on / off", defaults: ["s"] },
  { id: "slower", label: "Slower", defaults: ["[", "-"] },
  { id: "faster", label: "Faster", defaults: ["]", "+"] },
  { id: "toStart", label: "Jump to start", defaults: ["Home"] },
  { id: "toEnd", label: "Jump to end", defaults: ["End"] },
  { id: "trimStart", label: "Set trim start at playhead", defaults: ["i"] },
  { id: "trimEnd", label: "Set trim end at playhead", defaults: ["o"] },
  { id: "preview", label: "Preview trim selection", defaults: ["p"] },
  { id: "mute", label: "Mute / unmute", defaults: ["m"] },
  { id: "fullscreen", label: "Fullscreen", defaults: ["f"] },
  { id: "help", label: "Show shortcuts", defaults: ["?"] },
];

/** Keys that are not configurable: Esc closes, 0-9 jump to 0-90 %. */
export const FIXED_KEYS = new Set(["Escape", "0", "1", "2", "3", "4", "5", "6", "7", "8", "9"]);

/** Letters are matched case-insensitively (so Shift+L still means L). */
export const normalizeKey = (key: string) => (key.length === 1 ? key.toLowerCase() : key);

/** The effective keys per action. */
export function resolveViewerKeys(viewer: Settings["viewer"] | undefined): Record<string, string[]> {
  const out: Record<string, string[]> = {};
  for (const a of VIEWER_ACTIONS) out[a.id] = viewer?.keys[a.id] ?? a.defaults;
  return out;
}

/** Action bound to `key`, or null. */
export function actionForKey(keys: Record<string, string[]>, key: string): string | null {
  const k = normalizeKey(key);
  for (const [id, list] of Object.entries(keys)) if (list.some((x) => normalizeKey(x) === k)) return id;
  return null;
}

const NAMES: Record<string, string> = {
  " ": "Space",
  ArrowLeft: "←",
  ArrowRight: "→",
  ArrowUp: "↑",
  ArrowDown: "↓",
  Escape: "Esc",
  PageUp: "PgUp",
  PageDown: "PgDn",
  Delete: "Del",
  Backspace: "Backspace",
  Enter: "Enter",
  Tab: "Tab",
};

export const keyLabel = (key: string) => NAMES[key] ?? (key.length === 1 ? key.toUpperCase() : key);

/** Action ids that share a key with another action. */
export function conflicts(keys: Record<string, string[]>): Set<string> {
  const seen = new Map<string, string>();
  const bad = new Set<string>();
  for (const [id, list] of Object.entries(keys)) {
    for (const k of list.map(normalizeKey)) {
      const other = seen.get(k);
      if (other && other !== id) {
        bad.add(id);
        bad.add(other);
      } else seen.set(k, id);
    }
  }
  return bad;
}
