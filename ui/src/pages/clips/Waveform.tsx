import { useEffect, useState } from "react";
import { api, errorMessage } from "../../lib/ipc";
import { quietRanges, type Segment } from "../../lib/clipEdits";

export function Waveform({ path, duration, playhead, onSeek, onSelect }: { path: string; duration: number; playhead: number; onSeek: (time: number) => void; onSelect: (range: Segment) => void }) {
  const [peaks, setPeaks] = useState<number[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [zoom, setZoom] = useState(1);
  const [suggest, setSuggest] = useState(false);
  useEffect(() => {
    let alive = true;
    api.clipWaveform(path).then((v) => { if (alive) setPeaks(v); }, (e) => { if (alive) setError(errorMessage(e)); });
    return () => { alive = false; };
  }, [path]);
  return <section className="space-y-2 p-4" aria-label="Audio waveform">
    <div className="flex items-center justify-between text-xs text-zinc-300">
      <span>Audio waveform · first audio lane</span>
      <label>Timeline zoom <select aria-label="Timeline zoom" value={zoom} onChange={(e) => setZoom(Number(e.target.value))} className="rounded bg-zinc-800 p-2">
        {[1, 2, 4, 8, 16].map((n) => <option key={n} value={n}>{n}×</option>)}
      </select></label>
    </div>
    {error ? <p role="status" className="text-xs text-amber-300">Waveform unavailable: {error}</p> : peaks === null ? <p className="text-xs text-zinc-400">Analyzing audio…</p> : !peaks.length ? <p className="text-xs text-zinc-400">No audio in this clip.</p> :
      <div className="overflow-x-auto rounded border border-zinc-700">
        <div style={{ width: `${zoom * 100}%` }}>
          <svg viewBox="0 0 1200 80" preserveAspectRatio="none" className="h-20 w-full cursor-crosshair" aria-hidden="true"
            onPointerDown={(e) => { const r = e.currentTarget.getBoundingClientRect(); onSeek(Math.max(0, Math.min(duration, (e.clientX - r.left) / r.width * duration))); }}>
            <path stroke="#38bdf8" strokeWidth="1" d={peaks.map((p, i) => `M${i},${40 - p * 39}v${Math.max(1, p * 78)}`).join(" ")} />
            <path stroke="white" strokeWidth="2" d={`M${playhead / Math.max(duration, 1) * 1200},0v80`} />
          </svg>
          <input aria-label="Seek audio waveform" type="range" min={0} max={duration} step={0.01} value={playhead} onChange={(e) => onSeek(Number(e.target.value))} className="block w-full accent-sky-500" />
        </div>
      </div>}
    {!!peaks?.length && <div className="space-y-2 text-xs text-zinc-300">
      <button type="button" className="rounded border border-zinc-600 px-3 py-2" onClick={() => setSuggest(!suggest)}>Suggest quiet sections</button>
      {suggest && <>
        <p>First-lane sections below -40 dB for at least 0.5 seconds. Select a suggestion, listen, then use Remove selection. Other lanes may still contain sound.</p>
        <div className="flex max-h-32 flex-wrap gap-2 overflow-y-auto">
          {quietRanges(peaks, duration).map((r, i) => <button key={i} type="button" className="rounded bg-zinc-800 px-3 py-2" onClick={() => { onSelect(r); onSeek(r[0]); }}>{r[0].toFixed(2)}–{r[1].toFixed(2)} s</button>)}
          {!quietRanges(peaks, duration).length && <p>No quiet sections found at this threshold.</p>}
        </div>
      </>}
    </div>}
  </section>;
}
