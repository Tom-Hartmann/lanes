export type Segment = [number, number];
export interface Crop { left: number; right: number; top: number; bottom: number }
export interface ClipEdit { segments: Segment[]; crop: Crop }
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

export interface EditDraft { segments: Segment[] | null; crop: Crop }
export interface EditHistory { past: EditDraft[]; present: EditDraft; future: EditDraft[] }
export type EditAction = { type: "change"; value: EditDraft } | { type: "undo" | "redo" };
export const initialHistory = (): EditHistory => ({ past: [], present: { segments: null, crop: { ...NO_CROP } }, future: [] });
export function editHistory(state: EditHistory, action: EditAction): EditHistory {
  if (action.type === "change") {
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
