import { useEffect, useState } from "react";
import { useStatusStore } from "../store/statusStore";
import { useSettingsStore } from "../store/settingsStore";
import { sourceLevel } from "../lib/meters";

export function RecordingHealth() {
  const status = useStatusStore((s) => s.status);
  const stats = useStatusStore((s) => s.stats);
  const settings = useSettingsStore((s) => s.settings);
  const [quiet, setQuiet] = useState<string[]>([]);
  const active = status?.recording.state === "recording" && !status.recording.paused;
  useEffect(() => {
    const since = new Map<string, number>();
    if (!active) { setQuiet([]); return; }
    const timer = setInterval(() => {
      const microphones = settings?.profiles.find((p) => p.id === settings.activeProfile)?.audio.sources.filter((s) => s.kind.type === "input" && !s.muted && s.lanes.some((l) => status?.sessionLanes.includes(l))) ?? [];
      const now = Date.now();
      const silent: string[] = [];
      for (const source of microphones) {
        if (sourceLevel(source.id) > -60) since.delete(source.id);
        else { if (!since.has(source.id)) since.set(source.id, now); if (now - since.get(source.id)! >= 15000) silent.push(source.name); }
      }
      setQuiet(silent);
    }, 1000);
    return () => clearInterval(timer);
  }, [active, settings, status?.sessionLanes.join(",")]);
  if (!active) return null;
  const warnings = [
    stats && stats.freeDiskBytes < 5_000_000_000 ? "Less than 5 GB of free recording space." : "",
    stats && stats.encoderLoad > 1 ? "Encoder is falling behind. Consider a lower resolution or frame rate." : "",
    ...quiet.map((name) => `No audible signal from ${name} for 15 seconds. Check the microphone if you are speaking.`),
  ].filter(Boolean);
  return warnings.length ? <div role="status" className="border-b border-amber-700 bg-amber-950 px-4 py-2 text-sm text-amber-200">{warnings.map((w) => <p key={w}>{w}</p>)}</div> : null;
}
