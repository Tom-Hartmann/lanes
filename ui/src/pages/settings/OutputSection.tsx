import { Callout, Card, Field, NumberInput, Segmented, TextInput, Toggle } from "../../components/ui";
import { expandTemplate, unknownPlaceholders, TEMPLATE_PLACEHOLDERS } from "../../lib/format";
import type { OutputSettings, Settings } from "../../lib/ipc";
import { findActiveProfile } from "../../lib/settings";
import { useSettingsStore, type SaveMode } from "../../store/settingsStore";
import { useStatusStore } from "../../store/statusStore";

/** Live preview of a filename template with sample values. */
export function TemplatePreview({ template, settings, preset }: { template: string; settings: Settings; preset?: string }) {
  const game = useStatusStore((s) => s.status?.game?.name) ?? "Counter-Strike 2";
  const profile = findActiveProfile(settings)?.name ?? "Default";
  const unknown = unknownPlaceholders(template);
  return (
    <div className="text-xs">
      <div className="text-zinc-500">
        Preview:{" "}
        <span className="font-mono text-zinc-300 select-text">
          {expandTemplate(template, { game, profile, preset: preset ?? settings.clipPresets[0]?.name ?? "Clip" })}
          {settings.output.remuxToMp4 ? ".mp4" : ".mkv"}
        </span>
      </div>
      {unknown.length > 0 && <div className="text-amber-400">Unknown placeholder: {unknown.join(" ")}</div>}
    </div>
  );
}

export function OutputSection({ settings }: { settings: Settings }) {
  const update = useSettingsStore((s) => s.update);
  const out = settings.output;
  const set = (p: Partial<OutputSettings>, mode: SaveMode = "now") =>
    update((s) => ({ ...s, output: { ...s.output, ...p } }), mode);

  return (
    <Card title="Output">
      <Field label="Folder" hint="Recordings and clips are saved here.">
        <TextInput mono value={out.folder} onChange={(folder) => set({ folder }, "debounced")} className="w-80 max-w-full" />
      </Field>
      <div className="py-2">
        <div className="flex flex-wrap items-center justify-between gap-2">
          <div>
            <div className="text-sm text-zinc-200">File name</div>
            <div className="text-xs text-zinc-500">
              Placeholders: {TEMPLATE_PLACEHOLDERS.map((p) => `{${p}}`).join(" ")}
            </div>
          </div>
          <TextInput mono value={out.filenameTemplate} onChange={(filenameTemplate) => set({ filenameTemplate }, "debounced")} className="w-80 max-w-full" />
        </div>
        <div className="mt-1.5">
          <TemplatePreview template={out.filenameTemplate} settings={settings} />
        </div>
      </div>
      <Field label="Remux to MP4" hint="Records to crash-safe MKV and converts to MP4 when the file is finished.">
        <Toggle checked={out.remuxToMp4} onChange={(remuxToMp4) => set({ remuxToMp4 })} />
      </Field>
      <Field label="Disk space guard" hint="Watches free space on the output drive.">
        <Toggle checked={out.diskGuardEnabled} onChange={(diskGuardEnabled) => set({ diskGuardEnabled })} />
      </Field>
      {out.diskGuardEnabled && (
        <div className="ml-4 border-l border-zinc-800 pl-4">
          <Field label="Minimum free space">
            <NumberInput value={out.diskGuardMinFreeGb} min={1} max={10000} suffix="GB" onChange={(diskGuardMinFreeGb) => set({ diskGuardMinFreeGb }, "debounced")} />
          </Field>
          <Field label="When below">
            <Segmented
              value={out.diskGuardAction}
              onChange={(diskGuardAction) => set({ diskGuardAction })}
              options={[
                { value: "warn", label: "Warn" },
                { value: "stop", label: "Stop recording" },
              ]}
            />
          </Field>
          {out.diskGuardAction === "stop" && (
            <Callout>The recording is finalized cleanly before the disk fills up; the replay buffer keeps running.</Callout>
          )}
        </div>
      )}
    </Card>
  );
}
