import { useMemo, useState } from "react";
import { Button, Card, cx, Segmented, Select, TextInput } from "../../components/ui";
import type { AudioApp, AudioDevices, SourceKind, SourceKindType } from "../../lib/ipc";
import { addSource, MAX_LANES } from "./audioOps";
import { excludeOptions, uniqueApps } from "./SourceIcon";
import type { EditAudio } from "./LaneCard";

interface Props {
  devices: AudioDevices | null;
  apps: AudioApp[] | null;
  perAppSupported: boolean;
  laneCount: number;
  edit: EditAudio;
  onDone: () => void;
}

const DEFAULT_NAME: Record<SourceKindType, string> = {
  systemOutput: "Desktop audio",
  app: "App",
  game: "Game audio",
  input: "Microphone",
};

export function AddSource({ devices, apps, perAppSupported, laneCount, edit, onDone }: Props) {
  const [type, setType] = useState<SourceKindType>("systemOutput");
  const [device, setDevice] = useState("");
  const [exe, setExe] = useState("");
  const [exclude, setExclude] = useState("");
  const [name, setName] = useState("");
  const [ownLane, setOwnLane] = useState(true);

  // Unique apps, active ones first.
  const appList = useMemo(() => uniqueApps(apps), [apps]);

  const pickedApp = appList.find((a) => a.exe.toLowerCase() === exe.trim().toLowerCase());
  const deviceList = (type === "input" ? devices?.inputs : devices?.outputs) ?? [];

  const kind: SourceKind | null =
    type === "app"
      ? exe.trim()
        ? { type: "app", exe: exe.trim() }
        : null
      : type === "game"
        ? { type: "game" }
        : type === "input"
          ? { type, device: device === "" ? null : device }
          : { type, device: device === "" ? null : device, exclude: exclude === "" ? null : exclude };

  const defaultName =
    type === "app"
      ? (pickedApp?.name ?? exe.trim().replace(/\.exe$/i, "")) || DEFAULT_NAME.app
      : device && type !== "game"
        ? (deviceList.find((d) => d.id === device)?.name ?? DEFAULT_NAME[type])
        : DEFAULT_NAME[type];

  const add = () => {
    if (!kind) return;
    edit((a) => addSource(a, name.trim() || defaultName, kind, ownLane).layout);
    onDone();
  };

  return (
    <Card title="Add source" bodyClassName="space-y-3 p-3">
      <Segmented
        value={type}
        onChange={(t) => {
          setType(t);
          setDevice("");
          setExclude("");
        }}
        options={[
          { value: "systemOutput", label: "System output" },
          { value: "app", label: "App", disabled: !perAppSupported },
          { value: "game", label: "Game", disabled: !perAppSupported },
          { value: "input", label: "Input / mic" },
        ]}
      />

      {type === "game" && (
        <p className="text-xs text-zinc-500">
          Only the detected game's sound: music, Discord and other apps stay out. Switches to whichever game is running.
        </p>
      )}

      {type === "systemOutput" && perAppSupported && (
        <label className="flex items-center gap-2 text-sm">
          <span className="shrink-0 text-zinc-400">Leave out</span>
          <Select value={exclude} onChange={setExclude} className="min-w-0 flex-1" options={excludeOptions(apps, null)} />
        </label>
      )}

      {(type === "systemOutput" || type === "input") && (
        <Select
          value={device}
          onChange={setDevice}
          className="w-full"
          options={[
            { value: "", label: "Default device (follows Windows)" },
            ...deviceList.map((d) => ({ value: d.id, label: d.name + (d.isDefault ? " (current default)" : "") })),
          ]}
        />
      )}

      {type === "app" && (
        <div className="space-y-2">
          <TextInput mono value={exe} onChange={setExe} placeholder="Type an exe name, e.g. Discord.exe" className="w-full" />
          <ul className="max-h-44 divide-y divide-zinc-800 overflow-y-auto rounded-md border border-zinc-800">
            {apps === null && <li className="px-3 py-1.5 text-xs text-zinc-500">Loading apps…</li>}
            {apps !== null && appList.length === 0 && (
              <li className="px-3 py-1.5 text-xs text-zinc-500">No apps with audio sessions right now. Type the exe name.</li>
            )}
            {appList.map((a) => {
              const on = pickedApp?.exe === a.exe;
              return (
                <li key={a.exe.toLowerCase()}>
                  <button
                    type="button"
                    onClick={() => setExe(a.exe)}
                    className={cx(
                      "flex w-full items-center gap-2 px-3 py-1 text-left text-sm",
                      on ? "bg-sky-500/10" : "hover:bg-zinc-800/60",
                    )}
                  >
                    <span
                      className={cx("h-1.5 w-1.5 shrink-0 rounded-full", a.active ? "bg-emerald-400" : "bg-zinc-600")}
                      title={a.active ? "Playing audio now" : "Has an audio session, currently silent"}
                    />
                    <span className="min-w-0 flex-1 truncate text-zinc-200">{a.name}</span>
                    <span className="shrink-0 font-mono text-xs text-zinc-500">{a.exe}</span>
                  </button>
                </li>
              );
            })}
          </ul>
          <p className="text-xs text-zinc-500">The app is captured whenever it runs, including its child processes.</p>
        </div>
      )}

      <div className="flex flex-wrap items-center gap-2">
        <TextInput value={name} onChange={setName} placeholder={defaultName} className="min-w-40 flex-1" />
        <label
          className={cx("flex items-center gap-1.5 text-xs text-zinc-400", laneCount >= MAX_LANES && "opacity-40")}
          title={laneCount >= MAX_LANES ? `At most ${MAX_LANES} lanes` : undefined}
        >
          <input
            type="checkbox"
            checked={ownLane && laneCount < MAX_LANES}
            disabled={laneCount >= MAX_LANES}
            onChange={(e) => setOwnLane(e.target.checked)}
            className="accent-sky-500"
          />
          Give it its own lane
        </label>
      </div>
      <div className="flex justify-end gap-2">
        <Button size="sm" variant="ghost" onClick={onDone}>
          Cancel
        </Button>
        <Button size="sm" variant="primary" disabled={!kind} onClick={add}>
          Add source
        </Button>
      </div>
    </Card>
  );
}
