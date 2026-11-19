import { useEffect, useState } from "react";
import { Button, ConfirmButton } from "../../components/ui";
import { api, type ClipInfo, type ExportPreset } from "../../lib/ipc";
import { formatBytes } from "../../lib/format";
import { attempt } from "../../store/noticeStore";
import { enqueue, useExportQueue, cancelJob, retryJob } from "../../store/exportQueue";

const STANDARD: ExportPreset = { format: "mp4", maxMb: 0, width: 1280, height: 720, fps: 30, crf: 20, crossfade: 0 };
export function StudioPanel({ selected, onChanged }: { selected: ClipInfo[]; onChanged: () => void }) {
  const [order, setOrder] = useState<ClipInfo[]>([]);
  const [installing, setInstalling] = useState(false);
  const [input, setInput] = useState("");
  const [preset, setPreset] = useState<ExportPreset>(STANDARD);
  const [saved, setSaved] = useState<Record<string, ExportPreset>>(() => {
    try {
      const data = JSON.parse(localStorage.getItem("lanes.exportPresets.v1") ?? "{}");
      if (!data || typeof data !== "object" || Array.isArray(data)) return {};
      return Object.fromEntries(Object.entries(data).filter(([n,p]) => n.length <= 60 && p && typeof p === "object" && ["width","height","fps","crf","crossfade"].every((k) => Number.isFinite((p as Record<string, unknown>)[k]))).slice(0, 32)) as Record<string, ExportPreset>;
    } catch { return {}; }
  });
  const [name, setName] = useState("");
  const [days, setDays] = useState(30);
  const [limit, setLimit] = useState(100);
  const [candidates, setCandidates] = useState<ClipInfo[] | null>(null);
  const [auto, setAuto] = useState(false);
  const jobs = useExportQueue((s) => s.jobs);
  const storageError = useExportQueue((s) => s.storageError);
  const selectionKey = selected.map((c) => c.path).join("\n");
  useEffect(() => { setOrder(selected); }, [selectionKey]);
  useEffect(() => {
    if (!auto) return;
    const timer = setInterval(() => void attempt("Automatic cleanup skipped", async () => { await api.cleanupLibrary(days, limit, true); onChanged(); }), 10 * 60 * 1000);
    return () => clearInterval(timer);
  }, [auto, days, limit, onChanged]);
  const field = "rounded border border-zinc-600 bg-zinc-950 p-2 text-zinc-100";
  const outputSeconds = Math.max(0, order.reduce((n,c) => n + (c.durationSecs ?? 0), 0) - preset.crossfade * Math.max(0, order.length - 1));
  const estimate = (bitsPerPixel: number) => Math.min(preset.maxMb > 0 ? preset.maxMb * 1_000_000 : Infinity,
    outputSeconds * (preset.width * preset.height * preset.fps * bitsPerPixel * 2 ** ((23 - preset.crf) / 6) + 160_000) / 8);
  return <details className="rounded-lg border border-zinc-700 p-3 text-sm text-zinc-300">
    <summary className="cursor-pointer font-medium">Export studio, import & storage {jobs.some((j) => j.state === "running") && "· Exporting"}</summary>
    <div className="mt-4 space-y-5">
      <div className="flex flex-wrap gap-2">
        <label>Import video by full file path <input aria-label="Import video path" className={`${field} ml-2 w-64 max-w-full`} value={input} onChange={(e) => setInput(e.target.value)} placeholder="C:\Videos\clip.mp4" /></label>
        <Button size="sm" disabled={!input.trim()} onClick={() => void attempt("Could not queue import", async () => enqueue([input.trim().replace(/^"|"$/g, "")], preset, "Import video", "import"))}>Import copy</Button>
      </div>
      <p className="text-xs">Imports and exports create video copies in your output folder. The original is untouched. Studio audio uses each clip’s first track; save your lane mix first if needed.</p>
      <div className="flex flex-wrap gap-3">
        <label>Format <select className={field} value={preset.format} onChange={(e) => setPreset({ ...preset, format: e.target.value as "mp4" | "mkv", maxMb: 0 })}><option value="mp4">MP4</option><option value="mkv">MKV</option></select></label>
        <label>Size limit <select aria-label="Export size limit" className={field} disabled={preset.format !== "mp4"} value={preset.maxMb} onChange={(e) => setPreset({ ...preset, maxMb: Number(e.target.value) })}>{[0,10,25,50,100,500].map((n) => <option key={n} value={n}>{n ? `${n} MB` : "No limit"}</option>)}</select></label>
        <label>Resolution <select aria-label="Export resolution" className={field} value={`${preset.width}x${preset.height}`} onChange={(e) => { const [width, height] = e.target.value.split("x").map(Number); setPreset({ ...preset, width, height }); }}>
          {[[1280,720],[1920,1080],[3840,2160],[1080,1920],[1080,1080]].map(([w,h]) => <option key={w+"x"+h} value={w+"x"+h}>{w} × {h}</option>)}
        </select></label>
        <label>FPS <select aria-label="Export FPS" className={field} value={preset.fps} onChange={(e) => setPreset({ ...preset, fps: Number(e.target.value) })}>{[24,25,30,50,60].map((n) => <option key={n}>{n}</option>)}</select></label>
        <label>Quality <select className={field} value={preset.crf} onChange={(e) => setPreset({ ...preset, crf: Number(e.target.value) })}><option value={18}>High</option><option value={20}>Balanced</option><option value={26}>Smaller</option></select></label>
        <label>Crossfade + dissolve <select aria-label="Crossfade duration" className={field} value={preset.crossfade} onChange={(e) => setPreset({ ...preset, crossfade: Number(e.target.value) })}>{[0,.1,.25,.5,1,2].map((n) => <option key={n} value={n}>{n} s</option>)}</select></label>
      </div>
      <div className="flex flex-wrap gap-2">
        <input aria-label="Preset name" value={name} maxLength={60} onChange={(e) => setName(e.target.value)} placeholder="Preset name" className={field} />
        <Button size="sm" disabled={!name.trim()} onClick={() => void attempt("Could not save preset", async () => { const next = { ...saved, [name.trim()]: preset }; localStorage.setItem("lanes.exportPresets.v1", JSON.stringify(next)); setSaved(next); })}>Save preset</Button>
        {Object.entries(saved).map(([n,p]) => <Button key={n} size="sm" onClick={() => setPreset({ ...STANDARD, ...p })}>{n}</Button>)}
      </div>
      <p>Select clips in the library, then arrange their compilation order here.</p>
      <ol className="max-h-52 space-y-2 overflow-y-auto">{order.map((c,i) => <li key={c.path} className="flex gap-2"><span className="min-w-0 flex-1 truncate">{i+1}. {c.fileName}</span><Button size="sm" disabled={!i} onClick={() => { const next = [...order]; [next[i-1],next[i]] = [next[i],next[i-1]]; setOrder(next); }}>Earlier</Button></li>)}</ol>
      <p className="text-xs">Rough output estimate: {formatBytes(estimate(.04))}–{formatBytes(estimate(.2))}. Content affects size; this range is an estimate. {preset.maxMb > 0 ? `The ${preset.maxMb} MB maximum is enforced before saving.` : "Choose a size limit for a guaranteed maximum."}</p>
      <div className="flex flex-wrap gap-2">
        <Button size="sm" disabled={!order.length || order.length > 16} onClick={() => void attempt("Could not queue compilation", async () => enqueue(order.map((c) => c.path), preset, `${order.length} clip compilation`))}>Queue compilation</Button>
        <Button size="sm" disabled={!order.length || order.length > 16} onClick={() => void attempt("Could not queue exports", async () => { if (useExportQueue.getState().jobs.filter((j) => ["queued","running"].includes(j.state)).length + order.length > 16) throw new Error("Not enough space in the queue for this batch"); for (const c of order) enqueue([c.path], preset, c.fileName); })}>Queue separate exports</Button>
        <Button size="sm" disabled={order.length !== 1} onClick={() => void attempt("Could not queue recovery", async () => enqueue([order[0].path], preset, "Recover recording", "recover"))}>Recover selected recording</Button>
      </div>
      <p className="text-xs">Recovery salvages readable packets into a new MKV. Missing or never-written footage cannot be reconstructed. Select a stopped recording, including an interrupted MKV.</p>
      <section className="space-y-2">
        <h3>Automatic captions</h3>
        <p className="text-xs">Generate an editable SRT beside the selected clip, using its first audio track. Speech stays on this computer. Use standard-user mode for speech setup and transcription. Review automatic captions before sharing. Load the SRT in your video player or upload it alongside the video.</p>
        <div className="flex flex-wrap gap-2">
          <Button size="sm" disabled={installing} onClick={() => { setInstalling(true); void attempt("Speech setup failed", api.installSpeech).finally(() => setInstalling(false)); }}>{installing ? "Downloading speech engine?" : "Install local speech engine (~85 MB)"}</Button>
          <Button size="sm" disabled={order.length !== 1 || installing} onClick={() => void attempt("Could not queue captions", async () => enqueue([order[0].path], preset, "Generate captions", "captions"))}>Queue captions</Button>
        </div>
      </section>
      <section aria-label="Export queue" className="space-y-2">
        <h3>Export queue</h3>{storageError && <p role="alert">Queue history could not be saved on this device.</p>}
        <p className="text-xs">Queued jobs run in order while this window remains open. After restarting, interrupted jobs require an explicit retry.</p>
        {jobs.slice(-20).map((j) => <div key={j.id} className="flex flex-wrap items-center gap-2 rounded bg-zinc-950 p-2"><span className="flex-1">{j.label} · {j.state} {j.state === "running" && j.progress !== null ? `${Math.round(j.progress*100)}%` : ""}</span>
          {j.error && <span className="w-full text-xs text-amber-300">{j.error}</span>}
          {["queued","running"].includes(j.state) && <Button size="sm" disabled={j.state === "running" && j.progress === null} onClick={() => void attempt("Could not cancel", () => cancelJob(j))}>Cancel</Button>}
          {["failed","cancelled"].includes(j.state) && <Button size="sm" onClick={() => void attempt("Could not retry", async () => retryJob(j))}>Retry</Button>}
          {j.result && <Button size="sm" onClick={() => void attempt("Could not show file", () => api.revealClip(j.result!))}>Show file</Button>}
        </div>)}
      </section>
      <section className="space-y-2 border-t border-zinc-700 pt-3">
        <h3>Storage cleanup</h3><p className="text-xs">Recycle oldest files until the library is under your limit. Favorites and files younger than the age limit are protected. Recording and replay must be stopped.</p>
        <label>Older than <input aria-label="Cleanup age days" className={`${field} w-24`} type="number" min={7} max={3650} value={days} onChange={(e) => { setDays(Number(e.target.value)); setCandidates(null); setAuto(false); }} /> days</label>{" "}
        <label>Keep below <input aria-label="Library size limit GB" className={`${field} w-24`} type="number" min={1} max={100000} value={limit} onChange={(e) => { setLimit(Number(e.target.value)); setCandidates(null); setAuto(false); }} /> GB</label>
        <Button size="sm" onClick={() => void attempt("Could not preview cleanup", async () => setCandidates(await api.cleanupLibrary(days, limit, false)))}>Preview cleanup</Button>
        {candidates && <div><p>{candidates.length} files · {formatBytes(candidates.reduce((n,c) => n+c.sizeBytes,0))}</p><ul className="max-h-32 overflow-y-auto">{candidates.map((c) => <li key={c.path}>{c.fileName}</li>)}</ul>
          <ConfirmButton disabled={!candidates.length} onConfirm={() => void attempt("Cleanup failed", async () => { await api.cleanupLibrary(days, limit, true); setCandidates(null); onChanged(); })}>Recycle eligible files</ConfirmButton>
          <label className="ml-3"><input type="checkbox" checked={auto} onChange={(e) => setAuto(e.target.checked)} /> Repeat this cleanup every 10 minutes while this library page is open</label>
        </div>}
      </section>
    </div>
  </details>;
}
