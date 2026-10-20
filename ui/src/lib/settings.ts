// Pure, immutable helpers over the Settings tree.

import type { AudioLayout, AudioSource, Id, Lane, Profile, Settings, SourceKind, VideoSettings } from "./ipc";

export function newId(prefix: string): Id {
  const rnd =
    typeof crypto !== "undefined" && "randomUUID" in crypto
      ? crypto.randomUUID().replace(/-/g, "").slice(0, 10)
      : Math.random().toString(36).slice(2, 12);
  return `${prefix}-${rnd}`;
}

export const findActiveProfile = (s: Settings): Profile | undefined =>
  s.profiles.find((p) => p.id === s.activeProfile) ?? s.profiles[0];

export function withProfile(s: Settings, id: Id, fn: (p: Profile) => Profile): Settings {
  return { ...s, profiles: s.profiles.map((p) => (p.id === id ? fn(p) : p)) };
}

export function withActiveProfile(s: Settings, fn: (p: Profile) => Profile): Settings {
  const active = findActiveProfile(s);
  return active ? withProfile(s, active.id, fn) : s;
}

export const withVideo =
  (fn: (v: VideoSettings) => VideoSettings) =>
  (p: Profile): Profile => ({ ...p, video: fn(p.video) });

export const withAudio =
  (fn: (a: AudioLayout) => AudioLayout) =>
  (p: Profile): Profile => ({ ...p, audio: fn(p.audio) });

export const DEFAULT_VIDEO: VideoSettings = {
  mode: "followGame",
  api: "auto",
  monitors: [],
  layout: "separateFiles",
  windowExe: null,
  fps: 60,
  outputHeight: null,
  captureCursor: true,
};

export function defaultAudio(): AudioLayout {
  return {
    lanes: [
      { ...newLane("Game"), id: "game" },
      { ...newLane("Mic"), id: "mic" },
    ],
    sources: [
      { ...newSource("System audio", { type: "systemOutput", device: null }), id: "system", lanes: ["game"] },
      {
        ...newSource("Microphone", { type: "input", device: null }),
        id: "mic",
        mono: true,
        noiseGate: { thresholdDb: -50 },
        lanes: ["mic"],
      },
    ],
  };
}

export function newLane(name: string): Lane {
  return { id: newId("lane"), name, volumeDb: 0, muted: false, mono: false };
}

export function newSource(name: string, kind: SourceKind, lanes: Id[] = []): AudioSource {
  return {
    id: newId("src"),
    name,
    kind,
    volumeDb: 0,
    muted: false,
    mono: kind.type === "input",
    noiseGate: null,
    noiseSuppression: false,
    lanes,
  };
}

/**
 * Deep copy for duplicating a profile. Lane and source ids are kept: they are
 * scoped to a profile, and keeping them keeps clip preset lane subsets valid.
 */
export function cloneProfile(p: Profile, name: string): Profile {
  return {
    ...p,
    id: newId("profile"),
    name,
    video: { ...p.video, monitors: p.video.monitors.map((m) => ({ ...m })) },
    audio: {
      lanes: p.audio.lanes.map((l) => ({ ...l })),
      sources: p.audio.sources.map((s) => ({
        ...s,
        kind: { ...s.kind },
        noiseGate: s.noiseGate ? { ...s.noiseGate } : null,
        lanes: [...s.lanes],
      })),
    },
  };
}

/** A name not yet used in `taken`, e.g. "Lane 3". */
export function uniqueName(base: string, taken: string[]): string {
  const set = new Set(taken.map((t) => t.toLowerCase()));
  if (!set.has(base.toLowerCase())) return base;
  for (let i = 2; ; i++) if (!set.has(`${base} ${i}`.toLowerCase())) return `${base} ${i}`;
}
