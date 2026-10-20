import { useEffect, useRef, useState } from "react";
import { IconPlay } from "../../components/icons";
import { Badge } from "../../components/ui";
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

export function ClipCard({ clip, onOpen }: { clip: ClipInfo; onOpen: () => void }) {
  const { ref, src } = useLazyThumbnail(clip.path);
  return (
    <div
      ref={ref}
      className="group overflow-hidden rounded-lg border border-zinc-800 bg-zinc-900/60 transition-colors hover:border-zinc-600"
    >
      <button type="button" onClick={onOpen} className="relative block aspect-video w-full bg-zinc-950">
        {src ? (
          <img src={src} alt="" draggable={false} className="h-full w-full object-cover" loading="lazy" />
        ) : (
          <span className="absolute inset-0 animate-pulse bg-zinc-900" />
        )}
        <span className="absolute inset-0 flex items-center justify-center bg-black/0 opacity-0 transition group-hover:bg-black/30 group-hover:opacity-100">
          <span className="rounded-full bg-black/60 p-2.5 text-white">
            <IconPlay size={18} />
          </span>
        </span>
        <span className="absolute top-1.5 left-1.5">
          <Badge tone={clip.kind === "clip" ? "sky" : "neutral"}>{clip.kind === "clip" ? "Clip" : "Recording"}</Badge>
        </span>
        {clip.durationSecs !== null && (
          <span className="absolute right-1.5 bottom-1.5 rounded bg-black/70 px-1 font-mono text-[11px] text-zinc-100">
            {formatDuration(clip.durationSecs)}
          </span>
        )}
      </button>
      <div className="px-2.5 py-1.5">
        <div className="truncate text-sm text-zinc-200" title={clip.fileName}>
          {clip.fileName}
        </div>
        <div className="flex justify-between gap-2 text-[11px] text-zinc-500">
          <span className="truncate">{formatDate(clip.createdMs)}</span>
          <span className="shrink-0">{formatBytes(clip.sizeBytes)}</span>
        </div>
      </div>
    </div>
  );
}
