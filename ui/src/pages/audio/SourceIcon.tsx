import { IconApp, IconMic, IconSpeaker } from "../../components/icons";
import type { AudioApp, AudioDevices, AudioSource, SourceKind } from "../../lib/ipc";

export function SourceIcon({ kind, size = 14 }: { kind: SourceKind; size?: number }) {
  if (kind.type === "app") return <IconApp size={size} />;
  if (kind.type === "input") return <IconMic size={size} />;
  return <IconSpeaker size={size} />;
}

export const KIND_LABEL: Record<SourceKind["type"], string> = {
  systemOutput: "System output",
  app: "App",
  input: "Input",
};

/** "Default device", the device's name, or the app's exe. */
export function describeKind(kind: SourceKind, devices: AudioDevices | null): string {
  if (kind.type === "app") return kind.exe;
  if (kind.device === null) return "Default device";
  const list = kind.type === "input" ? devices?.inputs : devices?.outputs;
  return list?.find((d) => d.id === kind.device)?.name ?? "Unavailable device";
}

/** Whether an app source currently plays audio (from the last listAudioApps snapshot). */
export function appActivity(s: AudioSource, apps: AudioApp[] | null): "active" | "running" | "absent" | null {
  if (s.kind.type !== "app" || !apps) return null;
  const exe = s.kind.exe.toLowerCase();
  const matches = apps.filter((a) => a.exe.toLowerCase() === exe);
  if (matches.length === 0) return "absent";
  return matches.some((a) => a.active) ? "active" : "running";
}
