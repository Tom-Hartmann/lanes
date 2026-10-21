import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { IconFolder, IconRefresh, IconTrash } from "../../components/icons";
import { Button, ConfirmButton, EmptyState, PageHeader, Segmented, Spinner, TextInput } from "../../components/ui";
import { formatBytes } from "../../lib/format";
import { api, type ClipInfo } from "../../lib/ipc";
import { attempt, notify, reportError } from "../../store/noticeStore";
import { useStatusStore } from "../../store/statusStore";
import { ClipCard } from "./ClipCard";
import { ClipViewer } from "./ClipViewer";

type Filter = "all" | "clip" | "recording";

export function ClipsPage() {
  const [clips, setClips] = useState<ClipInfo[] | null>(null);
  const [loading, setLoading] = useState(false);
  const [filter, setFilter] = useState<Filter>("all");
  const [query, setQuery] = useState("");
  const [open, setOpen] = useState<ClipInfo | null>(null);
  const clipSeq = useStatusStore((s) => s.clipSeq);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [deleting, setDeleting] = useState<{ done: number; total: number } | null>(null);
  /** Last clicked card, the anchor for Shift-click ranges. */
  const anchor = useRef<string | null>(null);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const list = await api.listClips();
      setClips([...list].sort((a, b) => b.createdMs - a.createdMs));
    } catch (err) {
      reportError("Could not list clips", err);
      setClips((c) => c ?? []);
    } finally {
      setLoading(false);
    }
  }, []);

  // On page open and whenever a clip is saved.
  useEffect(() => void load(), [load, clipSeq]);

  const shown = useMemo(() => {
    const q = query.trim().toLowerCase();
    return (clips ?? []).filter(
      (c) => (filter === "all" || c.kind === filter) && (q === "" || c.fileName.toLowerCase().includes(q)),
    );
  }, [clips, filter, query]);

  const totalBytes = shown.reduce((n, c) => n + c.sizeBytes, 0);
  const closeViewer = useCallback(() => setOpen(null), []);

  // Only what is shown can be selected; files deleted elsewhere drop out.
  const picked = useMemo(() => shown.filter((c) => selected.has(c.path)), [shown, selected]);
  const pickedBytes = picked.reduce((n, c) => n + c.sizeBytes, 0);
  const clearSelection = useCallback(() => setSelected(new Set()), []);

  const toggle = (path: string, range: boolean) => {
    setSelected((prev) => {
      const next = new Set(prev);
      const from = range && anchor.current ? shown.findIndex((c) => c.path === anchor.current) : -1;
      const to = shown.findIndex((c) => c.path === path);
      if (from >= 0 && to >= 0) {
        // Shift-click: select everything between the anchor and this card.
        for (const c of shown.slice(Math.min(from, to), Math.max(from, to) + 1)) next.add(c.path);
      } else if (next.has(path)) next.delete(path);
      else next.add(path);
      return next;
    });
    anchor.current = path;
  };

  const deleteSelected = async () => {
    const targets = picked;
    setDeleting({ done: 0, total: targets.length });
    let failed = 0;
    for (const [i, c] of targets.entries()) {
      try {
        await api.deleteClip(c.path);
      } catch (err) {
        failed += 1;
        if (failed === 1) reportError(`Could not delete ${c.fileName}`, err);
      }
      setDeleting({ done: i + 1, total: targets.length });
    }
    setDeleting(null);
    clearSelection();
    const ok = targets.length - failed;
    if (ok > 0) notify({ kind: "success", title: `Moved ${ok} file${ok === 1 ? "" : "s"} to the recycle bin` });
    void load();
  };

  // Esc clears the selection (the viewer handles Esc itself while open).
  useEffect(() => {
    if (selected.size === 0 || open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") clearSelection();
      else if (e.key === "a" && e.ctrlKey && !(e.target instanceof HTMLInputElement)) {
        e.preventDefault();
        setSelected(new Set(shown.map((c) => c.path)));
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [selected.size, open, shown, clearSelection]);

  return (
    <div className="flex flex-col gap-4 p-5">
      <PageHeader
        title="Clips"
        subtitle={clips ? `${shown.length} file${shown.length === 1 ? "" : "s"} · ${formatBytes(totalBytes)}` : undefined}
        actions={
          <>
            <Button size="sm" variant="ghost" disabled={loading} onClick={() => void load()}>
              {loading ? <Spinner /> : <IconRefresh size={14} />} Refresh
            </Button>
            <Button size="sm" onClick={() => void attempt("Could not open the output folder", api.openOutputFolder)}>
              <IconFolder size={14} /> Open folder
            </Button>
          </>
        }
      />
      <div className="flex flex-wrap items-center gap-2">
        <Segmented<Filter>
          value={filter}
          onChange={setFilter}
          options={[
            { value: "all", label: "All" },
            { value: "clip", label: "Clips" },
            { value: "recording", label: "Recordings" },
          ]}
        />
        <TextInput value={query} onChange={setQuery} placeholder="Search file names…" className="w-60 max-w-full" />
      </div>

      {picked.length > 0 && (
        <div className="sticky top-0 z-10 flex flex-wrap items-center gap-2 rounded-lg border border-sky-500/40 bg-zinc-900/95 px-3 py-2 text-sm backdrop-blur">
          <span className="text-zinc-200">
            {picked.length} selected <span className="text-zinc-500">· {formatBytes(pickedBytes)}</span>
          </span>
          <Button size="sm" variant="ghost" disabled={picked.length === shown.length} onClick={() => setSelected(new Set(shown.map((c) => c.path)))}>
            Select all ({shown.length})
          </Button>
          <Button size="sm" variant="ghost" onClick={clearSelection}>
            Clear
          </Button>
          <span className="flex-1" />
          {deleting ? (
            <span className="flex items-center gap-2 text-zinc-400">
              <Spinner /> Deleting {deleting.done}/{deleting.total}…
            </span>
          ) : (
            <ConfirmButton
              title="Move the selected files to the recycle bin"
              confirmLabel={`Delete ${picked.length}? Click again`}
              onConfirm={() => void deleteSelected()}
            >
              <IconTrash size={14} /> Delete {picked.length}
            </ConfirmButton>
          )}
        </div>
      )}

      {clips === null ? (
        <div className="flex items-center gap-2 text-sm text-zinc-500">
          <Spinner /> Loading clips…
        </div>
      ) : shown.length === 0 ? (
        <EmptyState>
          {clips.length === 0
            ? "No clips yet. Turn on the replay buffer and press your save-clip hotkey in game."
            : "Nothing matches this filter."}
        </EmptyState>
      ) : (
        <div className="grid grid-cols-[repeat(auto-fill,minmax(12rem,1fr))] gap-3">
          {shown.map((c) => (
            <ClipCard
              key={c.path}
              clip={c}
              onOpen={() => setOpen(c)}
              selected={selected.has(c.path)}
              selecting={picked.length > 0}
              onToggle={(range) => toggle(c.path, range)}
            />
          ))}
        </div>
      )}

      {open && <ClipViewer clip={open} onClose={closeViewer} onChanged={() => void load()} />}
    </div>
  );
}
