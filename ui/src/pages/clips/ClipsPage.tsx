import { useCallback, useEffect, useMemo, useState } from "react";
import { IconFolder, IconRefresh } from "../../components/icons";
import { Button, EmptyState, PageHeader, Segmented, Spinner, TextInput } from "../../components/ui";
import { formatBytes } from "../../lib/format";
import { api, type ClipInfo } from "../../lib/ipc";
import { attempt, reportError } from "../../store/noticeStore";
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
            <ClipCard key={c.path} clip={c} onOpen={() => setOpen(c)} />
          ))}
        </div>
      )}

      {open && <ClipViewer clip={open} onClose={closeViewer} onChanged={() => void load()} />}
    </div>
  );
}
