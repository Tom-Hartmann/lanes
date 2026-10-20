// Native HTML5 drag & drop of sources onto lanes.

import { useRef, useState, type DragEvent } from "react";
import type { Id } from "../../lib/ipc";

export const SOURCE_MIME = "application/x-lanes-source";

export interface SourceDrag {
  sourceId: Id;
  /** Set when dragging a chip out of a lane: the drop moves it (Ctrl copies). */
  fromLane?: Id;
}

export function startSourceDrag(e: DragEvent, data: SourceDrag) {
  e.dataTransfer.setData(SOURCE_MIME, JSON.stringify(data));
  e.dataTransfer.setData("text/plain", data.sourceId);
  e.dataTransfer.effectAllowed = data.fromLane ? "copyMove" : "copy";
}

export function readSourceDrag(e: DragEvent): SourceDrag | null {
  try {
    const raw = e.dataTransfer.getData(SOURCE_MIME);
    if (!raw) return null;
    const v: unknown = JSON.parse(raw);
    if (v && typeof v === "object" && "sourceId" in v && typeof v.sourceId === "string") {
      const fromLane = "fromLane" in v && typeof v.fromLane === "string" ? v.fromLane : undefined;
      return { sourceId: v.sourceId, fromLane };
    }
  } catch {
    /* foreign drag */
  }
  return null;
}

const carriesSource = (e: DragEvent) => e.dataTransfer.types.includes(SOURCE_MIME);

/** Drop-target props plus an `over` flag for highlighting. */
export function useSourceDrop(onDrop: (d: SourceDrag, copy: boolean) => void) {
  const [over, setOver] = useState(false);
  const depth = useRef(0);
  return {
    over,
    props: {
      onDragEnter: (e: DragEvent) => {
        if (!carriesSource(e)) return;
        e.preventDefault();
        depth.current++;
        setOver(true);
      },
      onDragOver: (e: DragEvent) => {
        if (!carriesSource(e)) return;
        e.preventDefault();
        e.dataTransfer.dropEffect = e.dataTransfer.effectAllowed === "copyMove" && !e.ctrlKey ? "move" : "copy";
      },
      onDragLeave: (e: DragEvent) => {
        if (!carriesSource(e)) return;
        depth.current = Math.max(0, depth.current - 1);
        if (depth.current === 0) setOver(false);
      },
      onDrop: (e: DragEvent) => {
        depth.current = 0;
        setOver(false);
        const d = readSourceDrag(e);
        if (!d) return;
        e.preventDefault();
        onDrop(d, e.ctrlKey);
      },
    },
  };
}
