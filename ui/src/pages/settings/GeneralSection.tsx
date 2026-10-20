import { Card, Field, NumberInput, Select, Toggle } from "../../components/ui";
import type { GeneralSettings, Settings } from "../../lib/ipc";
import { useSettingsStore, type SaveMode } from "../../store/settingsStore";

export function GeneralSection({ settings }: { settings: Settings }) {
  const update = useSettingsStore((s) => s.update);
  const g = settings.general;
  const set = (p: Partial<GeneralSettings>, mode: SaveMode = "now") =>
    update((s) => ({ ...s, general: { ...s.general, ...p } }), mode);
  const off = !g.startWithWindows;

  return (
    <Card title="General">
      <Field label="Start with Windows">
        <Toggle checked={g.startWithWindows} onChange={(startWithWindows) => set({ startWithWindows })} />
      </Field>
      <div className="ml-4 border-l border-zinc-800 pl-4">
        <Field label="Start minimized to the tray" disabled={off}>
          <Toggle disabled={off} checked={g.startMinimized} onChange={(startMinimized) => set({ startMinimized })} />
        </Field>
        <Field label="Start the replay buffer" hint="Clips are ready without opening Lanes." disabled={off}>
          <Toggle disabled={off} checked={g.autostartReplay} onChange={(autostartReplay) => set({ autostartReplay })} />
        </Field>
        <Field label="Load profile" disabled={off}>
          <Select
            disabled={off}
            value={g.autostartProfile ?? ""}
            onChange={(v) => set({ autostartProfile: v === "" ? null : v })}
            options={[
              { value: "", label: "Last active profile" },
              ...settings.profiles.map((p) => ({ value: p.id, label: p.name })),
            ]}
          />
        </Field>
        <Field label="Delay" hint="Wait after sign-in so Lanes does not slow down startup." disabled={off}>
          <NumberInput
            disabled={off}
            value={g.autostartDelaySecs}
            min={0}
            max={600}
            suffix="s"
            onChange={(autostartDelaySecs) => set({ autostartDelaySecs }, "debounced")}
          />
        </Field>
      </div>
      <Field label="Clip saved sound" hint="A short sound you hear in game.">
        <Toggle checked={g.clipSound} onChange={(clipSound) => set({ clipSound })} />
      </Field>
      <Field label="Clip saved notification" hint="A Windows notification when a clip is written.">
        <Toggle checked={g.clipToast} onChange={(clipToast) => set({ clipToast })} />
      </Field>
    </Card>
  );
}
