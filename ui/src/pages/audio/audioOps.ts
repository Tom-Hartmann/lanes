// Pure edits of an AudioLayout. Every function returns a new layout.

import type { AudioApp, AudioLayout, AudioSource, Id, Lane, SourceKind } from "../../lib/ipc";
import { newLane, newSource, uniqueName } from "../../lib/settings";

export const MAX_LANES = 16;

/** Voice chat apps that "voice splitting" gives their own lane. */
export const VOICE_EXES = [
  "discord.exe",
  "discordptb.exe",
  "discordcanary.exe",
  "ts3client_win64.exe",
  "ts3client_win32.exe",
  "teamspeak.exe",
  "mumble.exe",
];

export const isVoiceExe = (exe: string) => VOICE_EXES.includes(exe.toLowerCase());

export function addLane(a: AudioLayout, name?: string): { layout: AudioLayout; lane: Lane } {
  const lane = newLane(uniqueName(name ?? `Lane ${a.lanes.length + 1}`, a.lanes.map((l) => l.name)));
  return { layout: { ...a, lanes: [...a.lanes, lane] }, lane };
}

export function removeLane(a: AudioLayout, id: Id): AudioLayout {
  return {
    lanes: a.lanes.filter((l) => l.id !== id),
    sources: a.sources.map((s) => (s.lanes.includes(id) ? { ...s, lanes: s.lanes.filter((x) => x !== id) } : s)),
  };
}

export function moveLane(a: AudioLayout, id: Id, delta: -1 | 1): AudioLayout {
  const i = a.lanes.findIndex((l) => l.id === id);
  const j = i + delta;
  if (i < 0 || j < 0 || j >= a.lanes.length) return a;
  const lanes = [...a.lanes];
  [lanes[i], lanes[j]] = [lanes[j], lanes[i]];
  return { ...a, lanes };
}

export const patchLane = (a: AudioLayout, id: Id, p: Partial<Lane>): AudioLayout => ({
  ...a,
  lanes: a.lanes.map((l) => (l.id === id ? { ...l, ...p } : l)),
});

export const patchSource = (a: AudioLayout, id: Id, p: Partial<AudioSource>): AudioLayout => ({
  ...a,
  sources: a.sources.map((s) => (s.id === id ? { ...s, ...p } : s)),
});

export const removeSource = (a: AudioLayout, id: Id): AudioLayout => ({
  ...a,
  sources: a.sources.filter((s) => s.id !== id),
});

export function setAssigned(a: AudioLayout, sourceId: Id, laneId: Id, on: boolean): AudioLayout {
  return {
    ...a,
    sources: a.sources.map((s) => {
      if (s.id !== sourceId) return s;
      const has = s.lanes.includes(laneId);
      if (on === has) return s;
      // Keep assignments in lane order for a stable display.
      const lanes = on ? a.lanes.map((l) => l.id).filter((id) => id === laneId || s.lanes.includes(id)) : s.lanes.filter((x) => x !== laneId);
      return { ...s, lanes };
    }),
  };
}

export function moveAssignment(a: AudioLayout, sourceId: Id, from: Id, to: Id): AudioLayout {
  if (from === to) return a;
  return setAssigned(setAssigned(a, sourceId, to, true), sourceId, from, false);
}

export function addSource(
  a: AudioLayout,
  name: string,
  kind: SourceKind,
  ownLane: boolean,
): { layout: AudioLayout; source: AudioSource } {
  let layout = a;
  const lanes: Id[] = [];
  if (ownLane && a.lanes.length < MAX_LANES) {
    const r = addLane(layout, name);
    layout = r.layout;
    lanes.push(r.lane.id);
  }
  const source = newSource(uniqueName(name, a.sources.map((s) => s.name)), kind, lanes);
  if (kind.type === "input") source.noiseGate = { thresholdDb: -50 };
  return { layout: { ...layout, sources: [...layout.sources, source] }, source };
}

const isVoiceSource = (s: AudioSource) => s.kind.type === "input" || (s.kind.type === "app" && isVoiceExe(s.kind.exe));

/** True if one of the source's lanes is fed by this source alone. */
function hasOwnLane(a: AudioLayout, s: AudioSource): boolean {
  return s.lanes.some((laneId) => a.sources.every((o) => o.id === s.id || !o.lanes.includes(laneId)));
}

/**
 * Gives every mic and voice-chat app its own lane. Running voice apps that are
 * not sources yet are added (when per-app capture is available) and a default
 * mic is added if there is none. Existing assignments are kept, so a mixed lane
 * still contains everything.
 */
export function voiceSplit(a: AudioLayout, apps: AudioApp[], perAppSupported: boolean): { layout: AudioLayout; lanesAdded: number; sourcesAdded: number } {
  let layout = a;
  let sourcesAdded = 0;
  let lanesAdded = 0;

  if (perAppSupported) {
    const have = new Set(
      layout.sources.flatMap((s) => (s.kind.type === "app" ? [s.kind.exe.toLowerCase()] : [])),
    );
    for (const app of apps) {
      const exe = app.exe.toLowerCase();
      if (!isVoiceExe(exe) || have.has(exe)) continue;
      have.add(exe);
      layout = { ...layout, sources: [...layout.sources, newSource(app.name || app.exe, { type: "app", exe: app.exe })] };
      sourcesAdded++;
    }
  }
  if (!layout.sources.some((s) => s.kind.type === "input")) {
    const mic = newSource("Microphone", { type: "input", device: null });
    mic.noiseGate = { thresholdDb: -50 };
    layout = { ...layout, sources: [...layout.sources, mic] };
    sourcesAdded++;
  }

  for (const s of layout.sources) {
    if (!isVoiceSource(s) || hasOwnLane(layout, s) || layout.lanes.length >= MAX_LANES) continue;
    const r = addLane(layout, s.name);
    layout = setAssigned(r.layout, s.id, r.lane.id, true);
    lanesAdded++;
  }
  return { layout, lanesAdded, sourcesAdded };
}
