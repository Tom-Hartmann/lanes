import { useEffect, useRef, useState } from "react";
import { IconPlay } from "../../components/icons";
import { Badge, cx } from "../../components/ui";
import { formatBytes, formatDate, formatDuration } from "../../lib/format";
import type { ClipInfo } from "../../lib/ipc";
import { clipThumbnail } from "./thumbnails";

/** Loads the thumbnail only once the card scrolls into view. */
function useLazyThumbnail(path: string) {
  const ref = useRef<HTMLDivElement>(null);
  const [src, setSrc] = useState<string | null>(null);
  const [visible, setVisible] = useState(false);

  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const io = new IntersectionObserver(
      (entries) => {
        if (entries.some((e) => e.isIntersecting)) {
          setVisible(true);
          io.disconnect();
        }
      },
      { rootMargin: "200px" },
    );
    io.observe(el);
    return () => io.disconnect();
  }, []);

  useEffect(() => {
    if (!visible) return;
    let alive = true;
    clipThumbnail(path).then(
      (url) => alive && setSrc(url),
      () => {},
    );
    return () => {
      alive = false;
    };
  }, [visible, path]);

  return { ref, src };
}

interface Props {
  clip: ClipInfo;
  onOpen: () => void;
  selected: boolean;
  /** Something is selected: a click toggles instead of opening. */
  selecting: boolean;
  onToggle: (range: boolean) => void;
}

export function ClipCard({ clip, onOpen, selected, selecting, onToggle }: Props) {
  const { ref, src } = useLazyThumbnail(clip.path);
  return (
    <div
      ref={ref}
      className={cx(
        "group relative overflow-hidden rounded-lg border bg-zinc-900/60 transition-colors",
        selected ? "border-sky-500 ring-1 ring-sky-500" : "border-zinc-800 hover:border-zinc-600",
      )}
    >
      <button
        type="button"
        onClick={(e) => (selecting || e.ctrlKey || e.shiftKey ? onToggle(e.shiftKey) : onOpen())}
        className="relative block aspect-video w-full bg-zinc-950"
      >
        {src ? (
          <img src={src} alt="" draggable={false} className="h-full w-full object-cover" loading="lazy" />
        ) : (
          <span className="absolute inset-0 animate-pulse bg-zinc-900" />
        )}
        {!selecting && (
          <span className="absolute inset-0 flex items-center justify-center bg-black/0 opacity-0 transition group-hover:bg-black/30 group-hover:opacity-100">
            <span className="rounded-full bg-black/60 p-2.5 text-white">
              <IconPlay size={18} />
            </span>
          </span>
        )}
        <span className="absolute top-1.5 left-1.5">
          <Badge tone={clip.kind === "clip" ? "sky" : "neutral"}>{clip.kind === "clip" ? "Clip" : "Recording"}</Badge>
        </span>
        {clip.markers.length > 0 && (
          <span
            className="absolute bottom-1.5 left-1.5 rounded bg-black/70 px-1 text-[11px] text-amber-300"
            title={`${clip.markers.length} bookmark${clip.markers.length === 1 ? "" : "s"}`}
          >
            ★ {clip.markers.length}
          </span>
        )}
        {clip.durationSecs !== null && (
          <span className="absolute right-1.5 bottom-1.5 rounded bg-black/70 px-1 font-mono text-[11px] text-zinc-100">
            {formatDuration(clip.durationSecs)}
          </span>
        )}
      </button>
      <label
        title="Select (Shift-click selects a range)"
        onClick={(e) => {
          e.preventDefault();
          onToggle(e.shiftKey);
        }}
        className={cx(
          "absolute top-1 right-1 flex h-7 w-7 cursor-pointer items-center justify-center rounded-md bg-black/50 transition-opacity",
          selected || selecting ? "opacity-100" : "opacity-0 group-hover:opacity-100",
        )}
      >
        <input type="checkbox" checked={selected} readOnly className="pointer-events-none h-4 w-4 accent-sky-500" />
      </label>
      <div className="px-2.5 py-1.5">
        <div className="truncate text-sm text-zinc-200" title={clip.fileName}>
          {clip.fileName}
        </div>
        <div className="flex justify-between gap-2 text-[11px] text-zinc-500">
          <span className="truncate">
            {clip.game && <span className="text-zinc-400">{clip.game} · </span>}
            {formatDate(clip.createdMs)}
          </span>
          <span className="shrink-0">{formatBytes(clip.sizeBytes)}</span>
        </div>
      </div>
    </div>
  );
}
