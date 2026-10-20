import { useState } from "react";
import { IconChevron, IconPlus } from "../../components/icons";
import { Badge, Button, Card, ConfirmButton, cx, Field, Select, Slider, TextInput, Toggle } from "../../components/ui";
import { formatSecs } from "../../lib/format";
import { saveClipAction, type ClipPreset, type ClipQuality, type Settings } from "../../lib/ipc";
import { findActiveProfile, newId, uniqueName } from "../../lib/settings";
import { useHotkeyStore } from "../../store/hotkeyStore";
import { useSettingsStore, type SaveMode } from "../../store/settingsStore";
import { TemplatePreview } from "./OutputSection";

const QUALITY: { value: ClipQuality; label: string }[] = [
  { value: "original", label: "Original (instant copy)" },
  { value: "high", label: "High (re-encode)" },
  { value: "medium", label: "Medium (re-encode)" },
  { value: "low", label: "Low (re-encode, small)" },
];

function PresetEditor({ preset, settings, onRemove }: { preset: ClipPreset; settings: Settings; onRemove: () => void }) {
  const update = useSettingsStore((s) => s.update);
  const combo = useHotkeyStore((s) => s.bindings.find((b) => b.action === saveClipAction(preset.id))?.combo);
  const [open, setOpen] = useState(false);
  const lanes = findActiveProfile(settings)?.audio.lanes ?? [];
  const replayLen = settings.replay.lengthSecs;

  const set = (p: Partial<ClipPreset>, mode: SaveMode = "now") =>
    update((s) => ({ ...s, clipPresets: s.clipPresets.map((c) => (c.id === preset.id ? { ...c, ...p } : c)) }), mode);

  const toggleLane = (id: string, on: boolean) => {
    const current = preset.lanes ?? lanes.map((l) => l.id);
    const next = on ? lanes.map((l) => l.id).filter((x) => x === id || current.includes(x)) : current.filter((x) => x !== id);
    set({ lanes: next });
  };

  return (
    <div className="rounded-md border border-zinc-800 bg-zinc-950/30">
      <div className="flex items-center gap-2 px-3 py-2">
        <button onClick={() => setOpen((o) => !o)} className="text-zinc-500 hover:text-zinc-200" aria-label="Expand">
          <IconChevron open={open} size={14} />
        </button>
        <TextInput
          value={preset.name}
          onChange={(name) => set({ name }, "debounced")}
          className="min-w-0 flex-1 border-transparent bg-transparent px-1 font-medium hover:border-zinc-700"
        />
        <span className="shrink-0 font-mono text-xs text-zinc-400">{formatSecs(preset.lengthSecs)}</span>
        {combo ? <Badge>{combo}</Badge> : <Badge title="Set it on the Hotkeys page">No hotkey</Badge>}
        <ConfirmButton onConfirm={onRemove} confirmLabel="Delete?">
          Delete
        </ConfirmButton>
      </div>
      {open && (
        <div className="border-t border-zinc-800 px-3 pb-2">
          <Field label="Length" hint={preset.lengthSecs > replayLen ? `Longer than the replay buffer (${formatSecs(replayLen)}): clips are cut to the buffer.` : undefined}>
            <Slider
              value={preset.lengthSecs}
              min={5}
              max={600}
              step={5}
              format={formatSecs}
              onChange={(lengthSecs) => set({ lengthSecs }, "debounced")}
              className="w-56"
              readoutWidth="w-20"
            />
          </Field>
          <Field label="Quality" hint="Original copies the buffered video instantly; the others re-encode in the background.">
            <Select value={preset.quality} options={QUALITY} onChange={(quality) => set({ quality })} />
          </Field>
          <Field label="Audio lanes" hint="Lanes of the active profile included in the clip.">
            <span className="text-xs text-zinc-400">All lanes</span>
            <Toggle checked={preset.lanes === null} onChange={(all) => set({ lanes: all ? null : lanes.map((l) => l.id) })} />
          </Field>
          {preset.lanes !== null && (
            <div className="flex flex-wrap gap-2 pb-2">
              {lanes.map((l) => {
                const on = preset.lanes?.includes(l.id) ?? true;
                return (
                  <label
                    key={l.id}
                    className={cx(
                      "flex cursor-pointer items-center gap-1.5 rounded-full border px-2.5 py-0.5 text-xs",
                      on ? "border-sky-700 bg-sky-500/10 text-sky-200" : "border-zinc-700 text-zinc-400",
                    )}
                  >
                    <input type="checkbox" checked={on} onChange={(e) => toggleLane(l.id, e.target.checked)} className="accent-sky-500" />
                    {l.name}
                  </label>
                );
              })}
              {preset.lanes.length === 0 && <span className="text-xs text-amber-400">No lanes: the clip has no audio.</span>}
            </div>
          )}
          <Field label="Folder override" hint="Empty uses the output folder.">
            <TextInput
              mono
              value={preset.folder ?? ""}
              placeholder={settings.output.folder}
              onChange={(v) => set({ folder: v.trim() === "" ? null : v }, "debounced")}
              className="w-72 max-w-full"
            />
          </Field>
          <Field label="File name override" hint="Empty uses the output file name.">
            <TextInput
              mono
              value={preset.filenameTemplate ?? ""}
              placeholder={settings.output.filenameTemplate}
              onChange={(v) => set({ filenameTemplate: v.trim() === "" ? null : v }, "debounced")}
              className="w-72 max-w-full"
            />
          </Field>
          {preset.filenameTemplate && <TemplatePreview template={preset.filenameTemplate} settings={settings} preset={preset.name} />}
        </div>
      )}
    </div>
  );
}

export function PresetsSection({ settings }: { settings: Settings }) {
  const update = useSettingsStore((s) => s.update);
  const add = () =>
    update((s) => ({
      ...s,
      clipPresets: [
        ...s.clipPresets,
        {
          id: newId("preset"),
          name: uniqueName("Clip", s.clipPresets.map((p) => p.name)),
          lengthSecs: Math.min(30, s.replay.lengthSecs),
          lanes: null,
          quality: "original",
          folder: null,
          filenameTemplate: null,
        },
      ],
    }));

  return (
    <Card
      title="Clip presets"
      actions={
        <Button size="sm" onClick={add}>
          <IconPlus size={14} /> Preset
        </Button>
      }
    >
      <p className="mb-3 text-xs text-zinc-500">
        Each preset saves the last N seconds of the replay buffer and gets its own hotkey (set it on the Hotkeys page).
      </p>
      <div className="space-y-2">
        {settings.clipPresets.map((p) => (
          <PresetEditor
            key={p.id}
            preset={p}
            settings={settings}
            onRemove={() => update((s) => ({ ...s, clipPresets: s.clipPresets.filter((c) => c.id !== p.id) }))}
          />
        ))}
        {settings.clipPresets.length === 0 && <p className="text-sm text-zinc-500">No presets: clips cannot be saved.</p>}
      </div>
    </Card>
  );
}
