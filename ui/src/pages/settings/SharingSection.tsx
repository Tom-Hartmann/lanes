import { Card, Field, Select, TextInput } from "../../components/ui";
import type { Settings } from "../../lib/ipc";
import { useSettingsStore } from "../../store/settingsStore";
import { isDiscordWebhook } from "../clips/SharePanel";

export function SharingSection({ settings }: { settings: Settings }) {
  const update = useSettingsStore((s) => s.update);
  const sh = settings.share;
  const set = (p: Partial<Settings["share"]>, mode: "now" | "debounced" = "now") =>
    update((s) => ({ ...s, share: { ...s.share, ...p } }), mode);
  const hook = sh.discordWebhook ?? "";
  const bad = hook.trim() !== "" && !isDiscordWebhook(hook);

  return (
    <Card title="Sharing">
      <Field label="Default size limit" hint="Share copies are re-encoded only if they are bigger than this.">
        <Select
          value={String(sh.maxMb)}
          onChange={(v) => set({ maxMb: Number(v) })}
          options={[
            { value: "10", label: "10 MB (Discord)" },
            { value: "50", label: "50 MB" },
            { value: "100", label: "100 MB" },
            { value: "500", label: "500 MB (Nitro)" },
            { value: "0", label: "No limit" },
          ]}
        />
      </Field>
      <Field
        label="Discord webhook"
        hint="Discord: Server settings → Integrations → Webhooks → New webhook → Copy URL. Clips are posted to that channel."
      >
        <TextInput
          className="w-72"
          mono
          placeholder="https://discord.com/api/webhooks/…"
          value={hook}
          onChange={(v) => set({ discordWebhook: v.trim() === "" ? null : v.trim() }, "debounced")}
        />
      </Field>
      {bad && <p className="-mt-1 text-xs text-amber-400">That does not look like a Discord webhook URL.</p>}
      <Field label="Message" hint="Posted with the clip. Placeholders: {game} {preset} {file}">
        <TextInput className="w-72" value={sh.discordMessage} onChange={(discordMessage) => set({ discordMessage }, "debounced")} />
      </Field>
    </Card>
  );
}
