import { useEffect, useState } from "react";
import { IconPlus, IconX } from "../../components/icons";
import { Badge, Button, Card, Select, Spinner, TextInput, Toggle } from "../../components/ui";
import { api, type GameEntry, type Settings, type WindowInfo } from "../../lib/ipc";
import { reportError } from "../../store/noticeStore";
import { useSettingsStore, type SaveMode } from "../../store/settingsStore";

function RunningWindows({ known, onPick, onClose }: { known: Set<string>; onPick: (w: WindowInfo) => void; onClose: () => void }) {
  const [windows, setWindows] = useState<WindowInfo[] | null>(null);
  useEffect(() => {
    let alive = true;
    api.listWindows().then(
      (list) => {
        if (!alive) return;
        // One row per exe, fullscreen windows first.
        const seen = new Set<string>();
        const sorted = [...list].sort((a, b) => Number(b.fullscreen) - Number(a.fullscreen));
        setWindows(
          sorted.filter((w) => {
            const key = w.exe.toLowerCase();
            if (seen.has(key)) return false;
            seen.add(key);
            return true;
          }),
        );
      },
      (err: unknown) => {
        reportError("Could not list windows", err);
        if (alive) setWindows([]);
      },
    );
    return () => {
      alive = false;
    };
  }, []);
  return (
    <div className="mt-3 rounded-md border border-zinc-800">
      <div className="flex items-center justify-between border-b border-zinc-800 px-3 py-1.5 text-xs text-zinc-400">
        Running programs
        <button onClick={onClose} className="text-zinc-500 hover:text-zinc-200" aria-label="Close">
          <IconX size={14} />
        </button>
      </div>
      <ul className="max-h-56 divide-y divide-zinc-800 overflow-y-auto">
        {windows === null && (
          <li className="flex items-center gap-2 px-3 py-2 text-xs text-zinc-500">
            <Spinner /> Loading…
          </li>
        )}
        {windows?.map((w) => {
          const added = known.has(w.exe.toLowerCase());
          return (
            <li key={w.exe.toLowerCase()} className="flex items-center gap-3 px-3 py-1.5 text-sm">
              <span className="w-40 shrink-0 truncate font-mono text-xs text-zinc-300">{w.exe}</span>
              <span className="min-w-0 flex-1 truncate text-zinc-400">{w.title}</span>
              {w.fullscreen && <Badge>Fullscreen</Badge>}
              <Button size="sm" disabled={added} onClick={() => onPick(w)}>
                {added ? "Added" : "Add"}
              </Button>
            </li>
          );
        })}
      </ul>
    </div>
  );
}

export function GamesSection({ settings }: { settings: Settings }) {
  const update = useSettingsStore((s) => s.update);
  const [picking, setPicking] = useState(false);
  const known = new Set(settings.games.map((g) => g.exe.toLowerCase()));

  const patch = (i: number, p: Partial<GameEntry>, mode: SaveMode = "now") =>
    update((s) => ({ ...s, games: s.games.map((g, j) => (j === i ? { ...g, ...p } : g)) }), mode);
  const add = (g: GameEntry) => update((s) => ({ ...s, games: [...s.games, g] }));
  const remove = (i: number) => update((s) => ({ ...s, games: s.games.filter((_, j) => j !== i) }));

  const profileOptions = [
    { value: "", label: "Active profile" },
    ...settings.profiles.map((p) => ({ value: p.id, label: p.name })),
  ];

  return (
    <Card
      title="Games"
      actions={
        <>
          <Button size="sm" variant="ghost" onClick={() => setPicking((p) => !p)}>
            From running programs
          </Button>
          <Button size="sm" onClick={() => add({ exe: "", name: "", profile: null, enabled: true })}>
            <IconPlus size={14} /> Game
          </Button>
        </>
      }
    >
      <p className="mb-2 text-xs text-zinc-500">
        Lanes recognizes these programs as games. With automatic switching on, the mapped profile is activated when the
        game starts.
      </p>
      {settings.games.length === 0 ? (
        <p className="text-sm text-zinc-500">No games yet.</p>
      ) : (
        <div className="overflow-x-auto">
          <table className="w-full min-w-[34rem] text-sm">
            <thead className="text-left text-[11px] tracking-wide text-zinc-500 uppercase">
              <tr>
                <th className="w-12 pb-1.5 font-medium">On</th>
                <th className="pb-1.5 font-medium">Executable</th>
                <th className="pb-1.5 font-medium">Name</th>
                <th className="pb-1.5 font-medium">Profile</th>
                <th />
              </tr>
            </thead>
            <tbody className="divide-y divide-zinc-800">
              {settings.games.map((g, i) => (
                <tr key={i}>
                  <td className="py-1.5">
                    <Toggle checked={g.enabled} onChange={(enabled) => patch(i, { enabled })} />
                  </td>
                  <td className="py-1.5 pr-2">
                    <TextInput mono value={g.exe} placeholder="game.exe" onChange={(exe) => patch(i, { exe }, "debounced")} className="w-full" />
                  </td>
                  <td className="py-1.5 pr-2">
                    <TextInput value={g.name} placeholder="Display name" onChange={(name) => patch(i, { name }, "debounced")} className="w-full" />
                  </td>
                  <td className="py-1.5 pr-2">
                    <Select value={g.profile ?? ""} options={profileOptions} onChange={(v) => patch(i, { profile: v === "" ? null : v })} />
                  </td>
                  <td className="py-1.5 text-right">
                    <button onClick={() => remove(i)} className="rounded p-1 text-zinc-500 hover:bg-zinc-800 hover:text-zinc-200" aria-label="Remove game">
                      <IconX size={14} />
                    </button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
      {picking && (
        <RunningWindows
          known={known}
          onClose={() => setPicking(false)}
          onPick={(w) => add({ exe: w.exe, name: w.title || w.exe.replace(/\.exe$/i, ""), profile: null, enabled: true })}
        />
      )}
    </Card>
  );
}
