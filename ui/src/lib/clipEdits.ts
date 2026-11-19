export type Segment = [number, number];
export interface Crop { left: number; right: number; top: number; bottom: number }
export interface AudioEffects { crossfade?: number; fadeIn: number; fadeOut: number; normalize: boolean; denoise: boolean }
export const NO_AUDIO: AudioEffects = { crossfade: 0, fadeIn: 0, fadeOut: 0, normalize: false, denoise: false };
export interface ClipEdit { segments: Segment[]; crop: Crop; audio?: AudioEffects }
export const NO_CROP: Crop = { left: 0, right: 0, top: 0, bottom: 0 };
export const MIN_SEGMENT = 0.05;

export function splitAt(segments: Segment[], time: number): Segment[] {
  if (segments.length >= 32) return segments;
  return segments.flatMap(([a, b]): Segment[] =>
    time - a >= MIN_SEGMENT && b - time >= MIN_SEGMENT ? [[a, time], [time, b]] : [[a, b]],
  );
}

export function removeRange(segments: Segment[], [start, end]: Segment): Segment[] {
  return segments.flatMap(([a, b]): Segment[] => {
    if (end <= a || start >= b) return [[a, b]];
    const kept: Segment[] = [];
    if (start - a >= MIN_SEGMENT) kept.push([a, start]);
    if (b - end >= MIN_SEGMENT) kept.push([end, b]);
    return kept;
  });
}

export function intersectRange(segments: Segment[], [start, end]: Segment): Segment[] {
  return segments.map(([a, b]): Segment => [Math.max(a, start), Math.min(b, end)]).filter(([a, b]) => b - a >= MIN_SEGMENT);
}

export function presetCrop(width: number, height: number, aspect: number): Crop {
  if (!(width > 0 && height > 0 && aspect > 0)) return { ...NO_CROP };
  if (width / height > aspect) {
    const edge = (1 - height * aspect / width) / 2;
    return { ...NO_CROP, left: edge, right: edge };
  }
  const edge = (1 - width / aspect / height) / 2;
  return { ...NO_CROP, top: edge, bottom: edge };
}

export const segmentDuration = (segments: Segment[]) => segments.reduce((sum, [a, b]) => sum + b - a, 0);
export const hasCrop = (crop: Crop) => Object.values(crop).some((n) => n > 0);

export interface EditDraft { segments: Segment[] | null; crop: Crop; audio?: AudioEffects }
export interface EditHistory { past: EditDraft[]; present: EditDraft; future: EditDraft[] }
export type EditAction = { type: "change"; value: EditDraft } | { type: "undo" | "redo" };
export const initialHistory = (): EditHistory => ({ past: [], present: { segments: null, crop: { ...NO_CROP } }, future: [] });
export function editHistory(state: EditHistory, action: EditAction): EditHistory {
  if (action.type === "change") {
    const crossfade = action.value.audio?.crossfade ?? 0;
    if (crossfade > 0 && (!action.value.segments || action.value.segments.some(([a,b]) => b-a <= crossfade*2))) {
      action = { ...action, value: { ...action.value, audio: { ...NO_AUDIO, ...action.value.audio, crossfade: 0 } } };
    }
    if (JSON.stringify(action.value) === JSON.stringify(state.present)) return state;
    return { past: [...state.past.slice(-99), state.present], present: action.value, future: [] };
  }
  if (action.type === "undo" && state.past.length) {
    return { past: state.past.slice(0, -1), present: state.past.at(-1)!, future: [state.present, ...state.future] };
  }
  if (action.type === "redo" && state.future.length) {
    return { past: [...state.past, state.present], present: state.future[0], future: state.future.slice(1) };
  }
  return state;
}

export interface SavedDraft { version: 1; duration: number; draft: EditDraft; range: Segment }
export function parseDraft(raw: string, duration: number): SavedDraft | null {
  try {
    const s = JSON.parse(raw) as SavedDraft;
    if (s.version !== 1 || !Number.isFinite(duration) || duration <= 0 || Math.abs(s.duration - duration) > 0.1) return null;
    const validRange = (r: Segment) => Array.isArray(r) && r.length === 2 && r.every(Number.isFinite) && r[0] >= 0 && r[1] <= duration + 0.05 && r[1] - r[0] >= MIN_SEGMENT;
    if (!validRange(s.range)) return null;
    if (s.draft.segments !== null && (!Array.isArray(s.draft.segments) || !s.draft.segments.length || s.draft.segments.length > 32 || !s.draft.segments.every(validRange))) return null;
    const c = s.draft.crop;
    if (![c.left, c.right, c.top, c.bottom].every((x) => Number.isFinite(x) && x >= 0) || c.left + c.right >= 0.95 || c.top + c.bottom >= 0.95) return null;
    const a = s.draft.audio ?? NO_AUDIO;
    const ranges = s.draft.segments ?? [s.range];
    const crossfade = a.crossfade ?? 0;
    if (!Number.isFinite(crossfade) || crossfade < 0 || crossfade > 2 || ranges.some(([start,end]) => end-start <= crossfade*2)) return null;
    const length = segmentDuration(ranges) - crossfade * Math.max(0,ranges.length-1);
    if (![a.fadeIn, a.fadeOut].every((x) => Number.isFinite(x) && x >= 0 && x <= duration) || typeof a.normalize !== "boolean" || typeof a.denoise !== "boolean") return null;
    if (s.draft.audio) s.draft.audio = { ...a, fadeIn: Math.min(a.fadeIn, length), fadeOut: Math.min(a.fadeOut, length) };
    return s;
  } catch { return null; }
}

export function fadeGain(time: number, segments: Segment[], audio: AudioEffects): number {
  let offset = 0;
  const duration = segmentDuration(segments);
  for (const [a, b] of segments) {
    if (time >= a && time <= b) {
      const t = offset + time - a;
      return Math.max(0, Math.min(1, audio.fadeIn > 0 ? t / audio.fadeIn : 1, audio.fadeOut > 0 ? (duration - t) / audio.fadeOut : 1));
    }
    offset += b - a;
  }
  return 1;
}

/** Suggestions only: coarse waveform bins cannot establish speech boundaries. */
export function quietRanges(peaks: number[], duration: number, threshold = 0.01): Segment[] {
  if (!peaks.length || duration <= 0) return [];
  const ranges: Segment[] = [];
  let start: number | null = null;
  for (let i = 0; i <= peaks.length; i++) {
    if (i < peaks.length && peaks[i] < threshold) { start ??= i; continue; }
    if (start !== null && (i - start) / peaks.length * duration >= 0.5) ranges.push([start / peaks.length * duration, i / peaks.length * duration]);
    start = null;
  }
  return ranges;
}
