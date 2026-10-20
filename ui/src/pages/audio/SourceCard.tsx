import { useState } from "react";
import { IconChevron, IconGrip, IconX } from "../../components/icons";
import { SourceMeter } from "../../components/Meter";
import { ConfirmButton, cx, DbSlider, Select, Slider, TextInput, Toggle, ToggleChip } from "../../components/ui";
import type { AudioApp, AudioDevices, AudioSource, SourceKind } from "../../lib/ipc";
import type { SaveMode } from "../../store/settingsStore";
import { patchSource, removeSource } from "./audioOps";
import { startSourceDrag } from "./dnd";
import type { EditAudio } from "./LaneCard";
import { appActivity, describeKind, KIND_LABEL, SourceIcon } from "./SourceIcon";

interface Props {
  source: AudioSource;
  laneNames: Map<string, string>;
  devices: AudioDevices | null;
  apps: AudioApp[] | null;
  perAppSupported: boolean;
  edit: EditAudio;
}

const ACTIVITY = {
  active: { dot: "bg-emerald-400", text: "Playing audio" },
  running: { dot: "bg-zinc-500", text: "Running, silent" },
  absent: { dot: "bg-zinc-700", text: "Not running" },
} as const;

function DevicePicker({ kind, devices, onChange }: { kind: SourceKind; devices: AudioDevices | null; onChange: (k: SourceKind) => void }) {
  if (kind.type === "app") return null;
  const list = (kind.type === "input" ? devices?.inputs : devices?.outputs) ?? [];
  const value = kind.device ?? "";
  const known = value === "" || list.some((d) => d.id === value);
  return (
    <Select
      value={value}
      className="min-w-0 flex-1"
      onChange={(device) => onChange({ type: kind.type, device: device === "" ? null : device })}
      options={[
        { value: "", label: "Default device (follows Windows)" },
        ...list.map((d) => ({ value: d.id, label: d.name + (d.isDefault ? " (current default)" : "") })),
        ...(known ? [] : [{ value, label: "Unavailable device" }]),
      ]}
    />
  );
}

export function SourceCard({ source: s, laneNames, devices, apps, perAppSupported, edit }: Props) {
  const [open, setOpen] = useState(false);
  const patch = (p: Partial<AudioSource>, mode?: SaveMode) => edit((a) => patchSource(a, s.id, p), mode);
  const activity = appActivity(s, apps);
  const unsupported = s.kind.type === "app" && !perAppSupported;

  return (
    <div className={cx("rounded-lg border border-zinc-800 bg-zinc-900/60 p-3", unsupported && "opacity-60")}>
      <div className="flex items-center gap-1.5">
        <span
          draggable
          onDragStart={(e) => startSourceDrag(e, { sourceId: s.id })}
          title="Drag onto a lane"
          className="flex shrink-0 cursor-grab items-center gap-1 rounded-md border border-zinc-700 bg-zinc-800 px-1.5 py-1 text-zinc-300 active:cursor-grabbing"
        >
          <IconGrip size={12} className="text-zinc-500" />
          <SourceIcon kind={s.kind} />
        </span>
        <TextInput
          value={s.name}
          onChange={(name) => patch({ name }, "debounced")}
          className="min-w-0 flex-1 border-transparent bg-transparent px-1 font-medium hover:border-zinc-700"
        />
        <ToggleChip active={s.muted} tone="red" title="Mute source" onClick={() => patch({ muted: !s.muted })}>
          M
        </ToggleChip>
        <ToggleChip active={s.mono} title="Downmix to mono (recommended for mics)" onClick={() => patch({ mono: !s.mono })}>
          Mono
        </ToggleChip>
        <button
          title="Processing and device"
          onClick={() => setOpen((o) => !o)}
          className={cx("rounded p-1 hover:bg-zinc-800", open ? "text-zinc-200" : "text-zinc-500")}
        >
          <IconChevron open={open} size={14} />
        </button>
        <ConfirmButton title="Remove source" confirmLabel="Remove?" onConfirm={() => edit((a) => removeSource(a, s.id))}>
          <IconX size={14} />
        </ConfirmButton>
      </div>

      <div className="mt-1 flex items-center gap-2 pl-1 text-xs text-zinc-500">
        <span>{KIND_LABEL[s.kind.type]}</span>
        <span className="text-zinc-700">·</span>
        <span className={cx("min-w-0 truncate", s.kind.type === "app" && "font-mono")}>{describeKind(s.kind, devices)}</span>
        {activity && (
          <span className="flex shrink-0 items-center gap-1" title={ACTIVITY[activity].text}>
            <span className={cx("h-1.5 w-1.5 rounded-full", ACTIVITY[activity].dot)} />
            {activity === "active" ? "active" : activity === "absent" ? "not running" : "silent"}
          </span>
        )}
        {s.noiseGate && <span className="shrink-0 text-zinc-600">gate</span>}
        {s.noiseSuppression && <span className="shrink-0 text-zinc-600">NS</span>}
      </div>

      <div className="mt-2 flex items-center gap-3">
        <DbSlider value={s.volumeDb} onChange={(volumeDb) => patch({ volumeDb }, "debounced")} className="w-48 max-w-[50%]" />
        <SourceMeter id={s.id} className="flex-1" />
      </div>

      <div className="mt-1.5 truncate text-xs">
        {s.lanes.length === 0 ? (
          <span className="text-amber-400/80">Not on any lane: drag it onto one to record it.</span>
        ) : (
          <span className="text-zinc-500">
            → {s.lanes.map((id) => laneNames.get(id) ?? "?").join(", ")}
          </span>
        )}
      </div>

      {open && (
        <div className="mt-3 space-y-2 border-t border-zinc-800 pt-3 text-sm">
          {s.kind.type === "app" ? (
            <div className="flex items-center gap-2">
              <span className="w-28 shrink-0 text-zinc-400">Executable</span>
              <TextInput
                mono
                value={s.kind.exe}
                onChange={(exe) => patch({ kind: { type: "app", exe } }, "debounced")}
                className="min-w-0 flex-1"
              />
            </div>
          ) : (
            <div className="flex items-center gap-2">
              <span className="w-28 shrink-0 text-zinc-400">Device</span>
              <DevicePicker kind={s.kind} devices={devices} onChange={(kind) => patch({ kind })} />
            </div>
          )}
          <div className="flex items-center gap-2">
            <span className="w-28 shrink-0 text-zinc-400">Noise gate</span>
            <Toggle
              checked={s.noiseGate !== null}
              onChange={(on) => patch({ noiseGate: on ? { thresholdDb: -50 } : null })}
            />
            {s.noiseGate && (
              <Slider
                className="min-w-0 flex-1"
                value={s.noiseGate.thresholdDb}
                min={-90}
                max={-10}
                step={1}
                format={(v) => `${v} dB`}
                onChange={(thresholdDb) => patch({ noiseGate: { thresholdDb } }, "debounced")}
              />
            )}
          </div>
          <div className="flex items-center gap-2">
            <span className="w-28 shrink-0 text-zinc-400">Noise suppression</span>
            <Toggle checked={s.noiseSuppression} onChange={(noiseSuppression) => patch({ noiseSuppression })} />
            <span className="text-xs text-zinc-500">Removes steady background noise (fans, hum).</span>
          </div>
        </div>
      )}
    </div>
  );
}
