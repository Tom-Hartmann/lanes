import { IconApp, IconGamepad, IconMic, IconSpeaker } from "../../components/icons";
import type { AudioApp, AudioDevices, AudioSource, SourceKind } from "../../lib/ipc";

export function SourceIcon({ kind, size = 14 }: { kind: SourceKind; size?: number }) {
  if (kind.type === "app") return <IconApp size={size} />;
  if (kind.type === "game") return <IconGamepad size={size} />;
  if (kind.type === "input") return <IconMic size={size} />;
  return <IconSpeaker size={size} />;
}

export const KIND_LABEL: Record<SourceKind["type"], string> = {
  systemOutput: "System output",
  app: "App",
  game: "Game",
  input: "Input",
};

/** "Default device", the device's name, or the app's exe. */
export function describeKind(kind: SourceKind, devices: AudioDevices | null): string {
  if (kind.type === "app") return kind.exe;
  if (kind.type === "game") return "Detected game (follows the running game)";
  const list = kind.type === "input" ? devices?.inputs : devices?.outputs;
  const name = kind.device === null ? "Default device" : (list?.find((d) => d.id === kind.device)?.name ?? "Unavailable device");
  return kind.type === "systemOutput" && kind.exclude ? `${name}, without ${kind.exclude}` : name;
}

/** Unique exe names from the app list, active ones first. */
export function uniqueApps(apps: AudioApp[] | null): AudioApp[] {
  const map = new Map<string, AudioApp>();
  for (const a of apps ?? []) {
    const key = a.exe.toLowerCase();
    const prev = map.get(key);
    if (!prev || (a.active && !prev.active)) map.set(key, a);
  }
  return [...map.values()].sort((a, b) => Number(b.active) - Number(a.active) || a.name.localeCompare(b.name));
}

/** Options for "leave out app": none, the known apps, and the current value if it is not running. */
export function excludeOptions(apps: AudioApp[] | null, current: string | null) {
  const list = uniqueApps(apps);
  const known = !current || list.some((a) => a.exe.toLowerCase() === current.toLowerCase());
  return [
    { value: "", label: "Nothing (record everything)" },
    ...list.map((a) => ({ value: a.exe, label: `${a.name} (${a.exe})` })),
    ...(known ? [] : [{ value: current, label: current }]),
  ];
}

/** Whether an app source currently plays audio (from the last listAudioApps snapshot). */
export function appActivity(s: AudioSource, apps: AudioApp[] | null): "active" | "running" | "absent" | null {
  if (s.kind.type !== "app" || !apps) return null;
  const exe = s.kind.exe.toLowerCase();
  const matches = apps.filter((a) => a.exe.toLowerCase() === exe);
  if (matches.length === 0) return "absent";
  return matches.some((a) => a.active) ? "active" : "running";
}
