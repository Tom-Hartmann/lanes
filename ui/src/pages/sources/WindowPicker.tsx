import { useCallback, useEffect, useMemo, useState } from "react";
import { IconRefresh } from "../../components/icons";
import { Badge, Button, Card, cx, Spinner, TextInput } from "../../components/ui";
import { api, type WindowInfo } from "../../lib/ipc";
import { reportError } from "../../store/noticeStore";

/** Lists open windows grouped by executable; the selection is stored by exe. */
export function WindowPicker({ exe, onChange }: { exe: string | null; onChange: (exe: string | null) => void }) {
  const [windows, setWindows] = useState<WindowInfo[] | null>(null);
  const [loading, setLoading] = useState(false);

  const refresh = useCallback(async () => {
    setLoading(true);
    try {
      setWindows(await api.listWindows());
    } catch (err) {
      reportError("Could not list windows", err);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => void refresh(), [refresh]);

  const byExe = useMemo(() => {
    const map = new Map<string, WindowInfo[]>();
    for (const w of windows ?? []) {
      const key = w.exe.toLowerCase();
      map.set(key, [...(map.get(key) ?? []), w]);
    }
    return [...map.values()].sort((a, b) => a[0].exe.localeCompare(b[0].exe));
  }, [windows]);

  const current = exe?.toLowerCase();

  return (
    <Card
      title="Window"
      actions={
        <Button size="sm" variant="ghost" disabled={loading} onClick={() => void refresh()}>
          {loading ? <Spinner /> : <IconRefresh size={14} />} Refresh
        </Button>
      }
    >
      <div className="flex flex-wrap items-center gap-2">
        <span className="text-sm text-zinc-400">Executable</span>
        <TextInput
          mono
          value={exe ?? ""}
          placeholder="game.exe"
          onChange={(v) => onChange(v.trim() === "" ? null : v.trim())}
          className="w-56"
        />
        <span className="text-xs text-zinc-500">Lanes captures this program's main window whenever it runs.</span>
      </div>
      <ul className="mt-3 max-h-72 divide-y divide-zinc-800 overflow-y-auto rounded-md border border-zinc-800">
        {windows === null && (
          <li className="flex items-center gap-2 px-3 py-2 text-sm text-zinc-500">
            <Spinner /> Loading…
          </li>
        )}
        {windows !== null && byExe.length === 0 && <li className="px-3 py-2 text-sm text-zinc-500">No windows found.</li>}
        {byExe.map((group) => {
          const w = group[0];
          const on = w.exe.toLowerCase() === current;
          return (
            <li key={w.exe.toLowerCase()}>
              <button
                type="button"
                onClick={() => onChange(w.exe)}
                className={cx(
                  "flex w-full items-center gap-3 px-3 py-1.5 text-left",
                  on ? "bg-sky-500/10" : "hover:bg-zinc-800/60",
                )}
              >
                <span className={cx("h-2 w-2 shrink-0 rounded-full", on ? "bg-sky-400" : "bg-zinc-700")} />
                <span className="w-44 shrink-0 truncate font-mono text-xs text-zinc-300">{w.exe}</span>
                <span className="min-w-0 flex-1 truncate text-sm text-zinc-400" title={group.map((g) => g.title).join("\n")}>
                  {w.title || "(untitled)"}
                  {group.length > 1 && <span className="text-zinc-600"> +{group.length - 1}</span>}
                </span>
                {group.some((g) => g.fullscreen) && <Badge>Fullscreen</Badge>}
              </button>
            </li>
          );
        })}
      </ul>
    </Card>
  );
}
