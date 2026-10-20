import { useCallback, useEffect, useRef, useState } from "react";
import { IconRefresh } from "../../components/icons";
import { Badge, Button, Callout, Card, cx, Segmented, Select, Spinner } from "../../components/ui";
import { api, type MonitorInfo, type MonitorSelection, type VideoSettings } from "../../lib/ipc";
import { reportError } from "../../store/noticeStore";

interface Props {
  video: VideoSettings;
  onChange: (fn: (v: VideoSettings) => VideoSettings) => void;
}

/**
 * Monitor cards with thumbnails. Thumbnails are fetched once when the page
 * opens and on the refresh button only, never on a timer, so the page costs
 * nothing while a game runs.
 */
export function MonitorPicker({ video, onChange }: Props) {
  const [monitors, setMonitors] = useState<MonitorInfo[] | null>(null);
  const [thumbs, setThumbs] = useState<Record<string, string>>({});
  const [loading, setLoading] = useState(false);
  /** Bumped on every refresh and on unmount; a stale run stops fetching. */
  const generation = useRef(0);

  const refresh = useCallback(async () => {
    const gen = ++generation.current;
    const current = () => gen === generation.current;
    setLoading(true);
    try {
      const list = await api.listMonitors();
      if (!current()) return;
      setMonitors(list);
      // One at a time keeps the capture work spread out.
      for (const m of list) {
        if (!current() || document.visibilityState !== "visible") break;
        try {
          const url = await api.getMonitorThumbnail(m.id);
          if (current()) setThumbs((t) => ({ ...t, [m.id]: url }));
        } catch {
          /* no thumbnail for this monitor */
        }
      }
    } catch (err) {
      if (current()) reportError("Could not list monitors", err);
    } finally {
      if (current()) setLoading(false);
    }
  }, []);

  useEffect(() => {
    void refresh();
    return () => {
      generation.current++;
    };
  }, [refresh]);

  const selected = new Map(video.monitors.map((m) => [m.id, m]));
  const primaryId = monitors?.find((m) => m.primary)?.id;
  const isSelected = (m: MonitorInfo) => selected.has(m.id) || (video.monitors.length === 0 && m.id === primaryId);

  const toggle = (m: MonitorInfo) =>
    onChange((v) => {
      // An empty list means "primary"; make that explicit before editing.
      const current = v.monitors.length === 0 && primaryId ? [{ id: primaryId, fpsCap: null, maxHeight: null }] : v.monitors;
      const has = current.some((s) => s.id === m.id);
      const next = has ? current.filter((s) => s.id !== m.id) : [...current, { id: m.id, fpsCap: null, maxHeight: null }];
      return { ...v, monitors: next };
    });

  const patch = (id: string, p: Partial<MonitorSelection>) =>
    onChange((v) => ({ ...v, monitors: v.monitors.map((m) => (m.id === id ? { ...m, ...p } : m)) }));

  const selectedList = video.monitors.filter((s) => monitors?.some((m) => m.id === s.id) ?? true);
  const nameOf = (id: string) => monitors?.find((m) => m.id === id)?.name ?? id;

  return (
    <Card
      title="Monitors"
      actions={
        <Button size="sm" variant="ghost" disabled={loading} onClick={() => void refresh()}>
          {loading ? <Spinner /> : <IconRefresh size={14} />} Refresh
        </Button>
      }
    >
      {!monitors ? (
        <div className="flex items-center gap-2 text-sm text-zinc-500">
          <Spinner /> Looking for monitors…
        </div>
      ) : (
        <div className="grid grid-cols-[repeat(auto-fill,minmax(11rem,1fr))] gap-3">
          {monitors.map((m) => {
            const on = isSelected(m);
            return (
              <button
                key={m.id}
                type="button"
                onClick={() => toggle(m)}
                className={cx(
                  "overflow-hidden rounded-lg border text-left transition-colors",
                  on ? "border-sky-500 bg-sky-500/5" : "border-zinc-800 bg-zinc-950/40 hover:border-zinc-600",
                )}
              >
                <div className="relative aspect-video bg-zinc-950">
                  {thumbs[m.id] ? (
                    <img src={thumbs[m.id]} alt="" className="h-full w-full object-cover" draggable={false} />
                  ) : (
                    <div className="flex h-full items-center justify-center text-xs text-zinc-700">No preview</div>
                  )}
                  <span
                    className={cx(
                      "absolute top-1.5 right-1.5 flex h-4 w-4 items-center justify-center rounded border text-[10px]",
                      on ? "border-sky-400 bg-sky-500 text-white" : "border-zinc-500 bg-zinc-900/80",
                    )}
                  >
                    {on && "✓"}
                  </span>
                </div>
                <div className="px-2.5 py-1.5">
                  <div className="flex items-center gap-1.5">
                    <span className="truncate text-sm text-zinc-200">{m.name}</span>
                    {m.primary && <Badge tone="sky">Primary</Badge>}
                  </div>
                  <div className="font-mono text-[11px] text-zinc-500">
                    {m.width}×{m.height} · {Math.round(m.refreshHz)} Hz
                  </div>
                </div>
              </button>
            );
          })}
        </div>
      )}
      {video.monitors.length === 0 && monitors && (
        <p className="mt-2 text-xs text-zinc-500">No monitor chosen: Lanes captures the primary monitor.</p>
      )}

      {selectedList.length > 1 && (
        <div className="mt-4 flex flex-wrap items-center justify-between gap-2 border-t border-zinc-800 pt-3">
          <div>
            <div className="text-sm text-zinc-200">Multi-monitor layout</div>
            <div className="text-xs text-zinc-500">
              Separate files keep each monitor at its own fps and size. A combined canvas is one video.
            </div>
          </div>
          <Segmented
            value={video.layout}
            onChange={(layout) => onChange((v) => ({ ...v, layout }))}
            options={[
              { value: "separateFiles", label: "Separate files" },
              { value: "canvas", label: "Combined canvas" },
            ]}
          />
        </div>
      )}

      {selectedList.length > 0 && (
        <div className="mt-4 border-t border-zinc-800 pt-3">
          <div className="text-sm text-zinc-200">Per-monitor limits</div>
          <div className="mb-2 text-xs text-zinc-500">
            Record a secondary monitor at 30 fps / 720p to keep encoder and disk load low while the game monitor stays at
            full quality.
          </div>
          <div className="space-y-1.5">
            {selectedList.map((s) => (
              <div key={s.id} className="flex flex-wrap items-center gap-2 text-sm">
                <span className="min-w-32 flex-1 truncate text-zinc-300">{nameOf(s.id)}</span>
                <Select
                  value={s.fpsCap === null ? "" : String(s.fpsCap)}
                  onChange={(v) => patch(s.id, { fpsCap: v === "" ? null : Number(v) })}
                  options={[
                    { value: "", label: `Output fps (${video.fps})` },
                    ...[60, 30, 15]
                      .filter((f) => f < video.fps)
                      .map((f) => ({ value: String(f), label: `Max ${f} fps` })),
                  ]}
                />
                <Select
                  value={s.maxHeight === null ? "" : String(s.maxHeight)}
                  onChange={(v) => patch(s.id, { maxHeight: v === "" ? null : Number(v) })}
                  options={[
                    { value: "", label: "Output size" },
                    ...[1440, 1080, 720, 480].map((h) => ({ value: String(h), label: `Max ${h}p` })),
                  ]}
                />
              </div>
            ))}
          </div>
          {video.layout === "canvas" && selectedList.length > 1 && (
            <div className="mt-2">
              <Callout>With a combined canvas all monitors share the output fps; per-monitor fps caps apply to separate files.</Callout>
            </div>
          )}
        </div>
      )}
    </Card>
  );
}
