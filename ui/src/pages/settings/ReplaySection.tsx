import { Callout, Card, Field, NumberInput, Segmented, Slider } from "../../components/ui";
import { formatSecs } from "../../lib/format";
import type { ReplaySettings, Settings } from "../../lib/ipc";
import { findActiveProfile } from "../../lib/settings";
import { useSettingsStore, type SaveMode } from "../../store/settingsStore";

export function ReplaySection({ settings }: { settings: Settings }) {
  const update = useSettingsStore((s) => s.update);
  const r = settings.replay;
  const set = (p: Partial<ReplaySettings>, mode: SaveMode = "now") =>
    update((s) => ({ ...s, replay: { ...s.replay, ...p } }), mode);

  // Rough size estimate from the video + audio bitrate (CQP has no fixed bitrate).
  const enc = settings.encoder;
  const lanes = findActiveProfile(settings)?.audio.lanes.length ?? 1;
  const videoKbps = enc.rateControl === "vbr" ? enc.maxBitrateKbps : enc.bitrateKbps;
  const kbps = enc.rateControl === "cqp" ? null : videoKbps + enc.audioBitrateKbps * lanes;
  const estimateMb = kbps === null ? null : (kbps * r.lengthSecs) / 8 / 1024;

  return (
    <Card title="Replay buffer">
      <Field label="Length" hint="How far back a clip can reach. Profiles can override this.">
        <Slider
          value={r.lengthSecs}
          min={15}
          max={600}
          step={15}
          format={formatSecs}
          onChange={(lengthSecs) => set({ lengthSecs }, "debounced")}
          className="w-56"
          readoutWidth="w-20"
        />
      </Field>
      <Field label="Storage" hint={r.storage === "ram" ? "Fastest, no disk writes. Uses memory." : "Uses a temporary file: low memory, constant disk writes."}>
        <Segmented
          value={r.storage}
          onChange={(storage) => set({ storage })}
          options={[
            { value: "ram", label: "Memory" },
            { value: "disk", label: "Disk" },
          ]}
        />
      </Field>
      {r.storage === "ram" && (
        <Field label="Memory cap" hint="When full, the oldest seconds are dropped first.">
          <NumberInput value={r.ramCapMb} min={64} max={65536} step={64} suffix="MB" onChange={(ramCapMb) => set({ ramCapMb }, "debounced")} />
        </Field>
      )}
      {estimateMb !== null && (
        <p className="text-xs text-zinc-500">
          At the current bitrate, {formatSecs(r.lengthSecs)} needs about {Math.ceil(estimateMb)} MB.
        </p>
      )}
      {r.storage === "ram" && estimateMb !== null && estimateMb > r.ramCapMb && (
        <div className="mt-2">
          <Callout tone="warn">
            The memory cap is smaller than the buffer length needs: clips will be shorter than {formatSecs(r.lengthSecs)}.
          </Callout>
        </div>
      )}
    </Card>
  );
}
