import { useEffect, useState } from "react";
import { Button, Card, Field, NumberInput, Select, Slider, TextInput, Toggle } from "../../components/ui";
import { api, errorMessage, type AudioDevice, type GeneralSettings, type OverlayCorner, type Settings } from "../../lib/ipc";
import { notify } from "../../store/noticeStore";
import { useSettingsStore, type SaveMode } from "../../store/settingsStore";

export function GeneralSection({ settings }: { settings: Settings }) {
  const update = useSettingsStore((s) => s.update);
  const g = settings.general;
  const set = (p: Partial<GeneralSettings>, mode: SaveMode = "now") =>
    update((s) => ({ ...s, general: { ...s.general, ...p } }), mode);
  const off = !g.startWithWindows;
  const [outputs, setOutputs] = useState<AudioDevice[]>([]);
  useEffect(() => {
    api.listAudioDevices().then((d) => setOutputs(d.outputs), () => {});
  }, []);
  const test = (kind: "sound" | "overlay" | "toast") =>
    api.testClipFeedback(kind).catch((e: unknown) => notify({ kind: "error", title: "Test failed", body: errorMessage(e) }));

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
      <Field label="Check for updates" hint="Asks GitHub for the latest release at startup and shows a link when there is a newer one.">
        <Toggle checked={g.checkUpdates} onChange={(checkUpdates) => set({ checkUpdates })} />
      </Field>
      <Field label="Clip saved sound" hint="A short chime. Pick another output than the one Lanes records to keep it out of your clips.">
        <div className="flex items-center gap-2">
          <Button size="sm" variant="ghost" onClick={() => void test("sound")}>
            Test
          </Button>
          <Toggle checked={g.clipSound} onChange={(clipSound) => set({ clipSound })} />
        </div>
      </Field>
      <div className="ml-4 border-l border-zinc-800 pl-4">
        <Field label="Output" disabled={!g.clipSound}>
          <Select
            disabled={!g.clipSound}
            value={g.clipSoundDevice ?? ""}
            onChange={(v) => set({ clipSoundDevice: v === "" ? null : v })}
            options={[
              { value: "", label: "Windows default output" },
              ...outputs.map((d) => ({ value: d.id, label: d.name })),
            ]}
          />
        </Field>
        <Field label="Volume" disabled={!g.clipSound}>
          <Slider
            className="w-48"
            disabled={!g.clipSound}
            value={g.clipSoundVolume}
            min={0}
            max={100}
            format={(v) => `${v} %`}
            onChange={(clipSoundVolume) => set({ clipSoundVolume }, "debounced")}
          />
        </Field>
      </div>
      <Field label="On-screen notice" hint="A small box in a screen corner. Click-through, never takes focus from the game.">
        <div className="flex items-center gap-2">
          <Button size="sm" variant="ghost" onClick={() => void test("overlay")}>
            Test
          </Button>
          <Toggle checked={g.clipOverlay} onChange={(clipOverlay) => set({ clipOverlay })} />
        </div>
      </Field>
      <div className="ml-4 border-l border-zinc-800 pl-4">
        <Field label="Corner" disabled={!g.clipOverlay}>
          <Select<OverlayCorner>
            disabled={!g.clipOverlay}
            value={g.clipOverlayCorner}
            onChange={(clipOverlayCorner) => set({ clipOverlayCorner })}
            options={[
              { value: "topLeft", label: "Top left" },
              { value: "topRight", label: "Top right" },
              { value: "bottomLeft", label: "Bottom left" },
              { value: "bottomRight", label: "Bottom right" },
            ]}
          />
        </Field>
        <Field label="Text" hint="Placeholders: {preset} {duration} {game}" disabled={!g.clipOverlay}>
          <TextInput
            className="w-72"
            disabled={!g.clipOverlay}
            value={g.clipOverlayText}
            onChange={(clipOverlayText) => set({ clipOverlayText }, "debounced")}
          />
        </Field>
      </div>
      <Field label="Windows notification" hint="A regular Windows notification in the Action Center.">
        <div className="flex items-center gap-2">
          <Button size="sm" variant="ghost" onClick={() => void test("toast")}>
            Test
          </Button>
          <Toggle checked={g.clipToast} onChange={(clipToast) => set({ clipToast })} />
        </div>
      </Field>
    </Card>
  );
}
