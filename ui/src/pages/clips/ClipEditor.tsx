import { useEffect, useState, type RefObject } from "react";
import { Button } from "../../components/ui";
import { formatDuration } from "../../lib/format";
import { NO_CROP, NO_AUDIO, MIN_SEGMENT, presetCrop, removeRange, splitAt, segmentDuration, type Crop, type EditDraft, type Segment } from "../../lib/clipEdits";

interface Props {
  draft: EditDraft;
  segments: Segment[];
  duration: number;
  selection: Segment;
  playhead: number;
  width: number;
  height: number;
  disabled: boolean;
  canUndo: boolean;
  canRedo: boolean;
  onChange: (draft: EditDraft) => void;
  onUndo: () => void;
  onRedo: () => void;
  onSeek: (time: number) => void;
  onPreview: () => void;
  onClearSelection: () => void;
}

export function ClipEditor(p: Props) {
  const { draft, segments, duration, selection, playhead } = p;
  const audio = draft.audio ?? NO_AUDIO;
  const [zoom, setZoom] = useState(1);
  const removed = removeRange(segments, selection);
  const canSplit = segments.length < 32 && segments.some(([a, b]) => playhead - a >= MIN_SEGMENT && b - playhead >= MIN_SEGMENT);
  const changeSegments = (next: Segment[]) => { p.onChange({ ...draft, segments: next }); p.onClearSelection(); };
  const outputW = Math.floor(p.width * (1 - draft.crop.left - draft.crop.right) / 2) * 2;
  const outputH = Math.floor(p.height * (1 - draft.crop.top - draft.crop.bottom) / 2) * 2;
  return (
    <fieldset disabled={p.disabled || duration <= 0} className="space-y-4 border-t border-zinc-800 p-4 disabled:opacity-60">
      <div className="flex flex-wrap items-center gap-2">
        <h3 className="mr-auto text-sm font-medium text-zinc-100">Cut & crop</h3>
        <Button size="sm" onClick={p.onUndo} disabled={!p.canUndo}>Undo</Button>
        <Button size="sm" onClick={p.onRedo} disabled={!p.canRedo}>Redo</Button>
        <Button size="sm" onClick={() => { p.onChange({ segments: null, crop: { ...NO_CROP } }); p.onClearSelection(); }}>Reset edits</Button>
      </div>
      <p className="text-xs leading-5 text-zinc-400">Split at the playhead, then remove the parts you don’t want. Or mark a range with the trim handles above and remove it in one click. Kept segments are joined in order.</p>
      <div className="flex flex-wrap gap-2">
        <Button size="sm" onClick={() => p.onChange({ ...draft, segments: splitAt(segments, playhead) })} disabled={!canSplit}>Split at playhead</Button>
        <Button size="sm" onClick={() => changeSegments(removed)} disabled={!removed.length || removed.length > 32 || JSON.stringify(removed) === JSON.stringify(segments)}>Remove selection</Button>
        <Button size="sm" onClick={() => changeSegments([[...selection]])} disabled={selection[1] - selection[0] < MIN_SEGMENT}>Keep selection only</Button>
        <Button size="sm" onClick={p.onPreview} disabled={!segments.length}>Preview kept segments</Button>
      </div>
      <label className="block text-xs text-zinc-300">Segment timeline zoom <select aria-label="Segment timeline zoom" value={zoom} onChange={(e) => setZoom(Number(e.target.value))} className="rounded bg-zinc-800 p-2">{[1,2,4,8].map((n) => <option key={n} value={n}>{n}x</option>)}</select></label>
      <div className="overflow-x-auto"><div style={{ width: `${zoom * 100}%` }} className="relative h-9 overflow-hidden rounded bg-zinc-950" aria-label="Kept segments on source timeline">
        {segments.map(([a, b], i) => (
          <button key={`${i}-${a}-${b}`} type="button" onClick={() => p.onSeek(a)} title={`Segment ${i + 1}: ${a.toFixed(2)}–${b.toFixed(2)} s`}
            aria-label={`Seek to segment ${i + 1}`} className="absolute inset-y-0 overflow-hidden rounded border border-emerald-400/60 bg-emerald-900 text-xs text-emerald-100 hover:bg-emerald-800 focus-visible:outline-2 focus-visible:outline-white"
            style={{ left: `${a / duration * 100}%`, width: `${(b - a) / duration * 100}%` }}>{i + 1}</button>
        ))}
        <div className="pointer-events-none absolute inset-y-0 w-0.5 bg-white" style={{ left: `${playhead / duration * 100}%` }} />
      </div>
      </div>
      <ol className="grid max-h-44 gap-2 overflow-y-auto sm:grid-cols-2">
        {segments.map(([a, b], i) => <li key={`${i}-${a}-${b}`} className="flex items-center gap-2 rounded border border-zinc-700 px-3 py-2 text-xs">
          <button type="button" className="flex-1 text-left text-zinc-200 hover:text-sky-300" onClick={() => p.onSeek(a)}>
            <span className="mr-2 text-emerald-300">{i + 1}</span>{a.toFixed(2)}–{b.toFixed(2)} s <span className="text-zinc-400">({(b - a).toFixed(2)} s)</span>
          </button>
          <Button size="sm" disabled={i === 0} aria-label={`Move segment ${i + 1} earlier`} onClick={() => { const next = [...segments]; [next[i - 1], next[i]] = [next[i], next[i - 1]]; changeSegments(next); }}>Earlier</Button>
          <Button size="sm" disabled={i === segments.length - 1} aria-label={`Move segment ${i + 1} later`} onClick={() => { const next = [...segments]; [next[i + 1], next[i]] = [next[i], next[i + 1]]; changeSegments(next); }}>Later</Button>
          <Button size="sm" disabled={segments.length === 1} aria-label={`Remove segment ${i + 1}`} onClick={() => changeSegments(segments.filter((_, k) => i !== k))}>Remove</Button>
        </li>)}
      </ol>
      <p className="text-xs text-emerald-300">{segments.length} kept {segments.length === 1 ? "segment" : "segments"} · {formatDuration(segmentDuration(segments))} total · {formatDuration(Math.max(0, duration - segmentDuration(segments)))} removed</p>
      <div className="space-y-3 border-t border-zinc-800 pt-4">
        <h4 className="text-sm text-zinc-200">Audio effects</h4>
        <p className="text-xs text-zinc-400">Applied to every audio lane in the saved copy. Fades are previewed here; preview the exported copy to hear normalization and noise reduction. Fades apply to the beginning and end of the joined result.</p>
        <label className="block text-xs text-zinc-300">Crossfade cuts with video dissolve <select aria-label="Cut crossfade" disabled={segments.length < 2} value={audio.crossfade ?? 0} onChange={(e) => p.onChange({ ...draft, audio: { ...audio, crossfade: Number(e.target.value) } })} className="ml-2 rounded bg-zinc-950 p-2">{[0,.1,.25,.5,1,2].map((n) => <option key={n} value={n} disabled={n > 0 && segments.some(([a,b]) => b-a <= 2*n)}>{n} seconds</option>)}</select></label>
        <p className="text-xs text-zinc-400">Transitions overlap adjacent segments and shorten the result. Use Render preview to see and hear them.</p>
        <div className="flex flex-wrap gap-4">
          {(["fadeIn", "fadeOut"] as const).map((key) => <label key={key} className="text-xs text-zinc-300">{key === "fadeIn" ? "Fade in (seconds)" : "Fade out (seconds)"}
            <input aria-label={key === "fadeIn" ? "Fade in seconds" : "Fade out seconds"} type="number" min={0} max={segmentDuration(draft.segments ?? [selection])} step={0.1} value={Math.min(audio[key], segmentDuration(draft.segments ?? [selection]))}
              onChange={(e) => { const n = e.currentTarget.valueAsNumber; if (Number.isFinite(n)) p.onChange({ ...draft, audio: { ...audio, [key]: Math.max(0, Math.min(n, segmentDuration(draft.segments ?? [selection]))) } }); }} className="ml-2 w-20 rounded bg-zinc-950 p-2" />
          </label>)}
          {(["normalize", "denoise"] as const).map((key) => <label key={key} className="text-xs text-zinc-300"><input type="checkbox" checked={audio[key]} onChange={(e) => p.onChange({ ...draft, audio: { ...audio, [key]: e.target.checked } })} /> {key === "normalize" ? "Normalize loudness (-16 LUFS)" : "Reduce steady background noise"}</label>)}
        </div>
      </div>
      <div className="flex flex-wrap items-center gap-2 border-t border-zinc-800 pt-4">
        <h4 className="mr-auto text-sm text-zinc-200">Crop edges</h4>
        <Button size="sm" onClick={() => p.onChange({ ...draft, crop: { ...NO_CROP } })}>Original</Button>
        {[["16:9", 16 / 9], ["Square", 1], ["Portrait 9:16", 9 / 16]].map(([label, ratio]) =>
          <Button key={label} size="sm" disabled={!p.width} onClick={() => p.onChange({ ...draft, crop: presetCrop(p.width, p.height, Number(ratio)) })}>{label}</Button>,
        )}
      </div>
      <div className="grid grid-cols-2 gap-3 sm:grid-cols-4">
        {(["left", "right", "top", "bottom"] as const).map((side) => {
          const opposite = { left: "right", right: "left", top: "bottom", bottom: "top" } as const;
          const max = Math.max(0, Math.min(90, 94 - draft.crop[opposite[side]] * 100));
          return <label key={side} className="space-y-1 text-xs text-zinc-300">
            <span className="capitalize">{side} (%)</span>
            <input type="number" min={0} max={max} step={0.5} value={Math.round(draft.crop[side] * 10000) / 100}
              onChange={(e) => { const n = e.currentTarget.valueAsNumber; if (Number.isFinite(n)) p.onChange({ ...draft, crop: { ...draft.crop, [side]: Math.min(max, Math.max(0, n)) / 100 } }); }}
              className="w-full rounded border border-zinc-600 bg-zinc-950 px-2 py-2 text-zinc-100 focus-visible:outline-2 focus-visible:outline-sky-400" />
          </label>;
        })}
      </div>
      <p className="text-xs leading-5 text-zinc-400">{p.width > 0 && <>Output: {outputW} × {outputH} pixels. </>}The shaded edges are removed. Cuts and crops save as a new MP4 with precise cuts and separate audio lanes; exporting takes longer than a lossless trim.</p>
    </fieldset>
  );
}

/** Align the crop guide with the actual image, excluding letterbox bars. */
export function CropOverlay({ video, crop, onChange, disabled }: { video: RefObject<HTMLVideoElement | null>; crop: Crop; onChange: (crop: Crop) => void; disabled: boolean }) {
  const [rect, setRect] = useState({ left: 0, top: 0, width: 0, height: 0 });
  useEffect(() => {
    const v = video.current;
    if (!v) return;
    const measure = () => {
      const scale = Math.min(v.clientWidth / v.videoWidth, v.clientHeight / v.videoHeight);
      if (!Number.isFinite(scale)) return;
      const width = v.videoWidth * scale, height = v.videoHeight * scale;
      setRect({ left: (v.clientWidth - width) / 2, top: (v.clientHeight - height) / 2, width, height });
    };
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(v);
    v.addEventListener("loadedmetadata", measure);
    return () => { observer.disconnect(); v.removeEventListener("loadedmetadata", measure); };
  }, [video]);
  return <div className="pointer-events-none absolute overflow-hidden" style={rect}>
    {(["left", "right", "top", "bottom"] as const).map((side) => <button key={side} type="button" disabled={disabled} aria-label={`Drag ${side} crop edge`}
      className="pointer-events-auto absolute z-10 h-6 w-6 touch-none rounded-full border-2 border-white bg-sky-600"
      style={{ left: `${(side === "left" ? crop.left : side === "right" ? 1 - crop.right : (crop.left + 1 - crop.right) / 2) * 100}%`, top: `${(side === "top" ? crop.top : side === "bottom" ? 1 - crop.bottom : (crop.top + 1 - crop.bottom) / 2) * 100}%`, transform: "translate(-50%, -50%)" }}
      onPointerDown={(e) => { e.preventDefault(); e.stopPropagation(); e.currentTarget.setPointerCapture(e.pointerId); }}
      onPointerUp={(e) => {
        if (!e.currentTarget.hasPointerCapture(e.pointerId)) return;
        const r = e.currentTarget.parentElement!.getBoundingClientRect();
        const opposite = { left: "right", right: "left", top: "bottom", bottom: "top" } as const;
        const n = side === "left" ? (e.clientX - r.left) / r.width : side === "right" ? (r.right - e.clientX) / r.width : side === "top" ? (e.clientY - r.top) / r.height : (r.bottom - e.clientY) / r.height;
        onChange({ ...crop, [side]: Math.max(0, Math.min(0.94 - crop[opposite[side]], n)) });
        e.currentTarget.releasePointerCapture(e.pointerId);
      }} />)}
    <div className="absolute border-2 border-sky-300" style={{ left: `${crop.left * 100}%`, right: `${crop.right * 100}%`, top: `${crop.top * 100}%`, bottom: `${crop.bottom * 100}%`, boxShadow: "0 0 0 9999px rgb(0 0 0 / 70%)" }} />
  </div>;
}
