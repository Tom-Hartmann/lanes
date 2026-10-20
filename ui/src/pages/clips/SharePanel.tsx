import { convertFileSrc } from "@tauri-apps/api/core";
import { useEffect, useRef, useState } from "react";
import { Button, Select, Spinner } from "../../components/ui";
import { formatBytes } from "../../lib/format";
import { api, errorMessage, type ClipInfo, type ShareResult } from "../../lib/ipc";
import { notify } from "../../store/noticeStore";
import { useSettingsStore } from "../../store/settingsStore";
import { useStatusStore } from "../../store/statusStore";

const SIZE_OPTIONS = [
  { value: "10", label: "10 MB (Discord)" },
  { value: "50", label: "50 MB (Nitro Basic / boosted server)" },
  { value: "100", label: "100 MB (boosted server)" },
  { value: "500", label: "500 MB (Nitro)" },
  { value: "0", label: "No limit" },
];

export const isDiscordWebhook = (url: string) =>
  /^https:\/\/(ptb\.|canary\.)?(discord|discordapp)\.com\/api\/webhooks\/\d+\/[\w-]+/.test(url.trim());

/**
 * Drag handle: a native drag starts once the pointer moves a few pixels with
 * the button held, so a plain click does nothing.
 */
function DragHandle({ path }: { path: string }) {
  const start = useRef<{ x: number; y: number } | null>(null);
  const dragging = useRef(false);
  return (
    <div
      role="button"
      tabIndex={0}
      title="Drag into Discord, Explorer, a browser upload…"
      onPointerDown={(e) => {
        if (e.button === 0) start.current = { x: e.clientX, y: e.clientY };
      }}
      onPointerUp={() => (start.current = null)}
      onPointerLeave={() => {
        // Leaving the handle with the button held also starts the drag.
        if (start.current && !dragging.current) begin();
      }}
      onPointerMove={(e) => {
        const s = start.current;
        if (!s || dragging.current) return;
        if (Math.abs(e.clientX - s.x) + Math.abs(e.clientY - s.y) > 4) begin();
      }}
      className="flex cursor-grab items-center gap-2 rounded-md border border-dashed border-sky-700/70 bg-sky-500/10 px-3 py-1.5 text-sm text-sky-200 select-none hover:bg-sky-500/20 active:cursor-grabbing"
    >
      <span aria-hidden>⠿</span> Drag me out
    </div>
  );

  function begin() {
    dragging.current = true;
    start.current = null;
    api
      .dragClip(path)
      .catch((err: unknown) => notify({ kind: "error", title: "Drag failed", body: errorMessage(err) }))
      .finally(() => (dragging.current = false));
  }
}

/** Uploads a file to a Discord webhook from the webview (no extra backend code). */
async function sendToDiscord(webhook: string, path: string, message: string, onProgress: (p: number) => void) {
  const blob = await (await fetch(convertFileSrc(path))).blob();
  const name = path.split(/[\\/]/).pop() ?? "clip.mp4";
  const form = new FormData();
  form.append("payload_json", JSON.stringify({ content: message.slice(0, 1900) }));
  form.append("files[0]", new File([blob], name, { type: "video/mp4" }));
  await new Promise<void>((resolve, reject) => {
    const xhr = new XMLHttpRequest();
    xhr.open("POST", webhook.trim());
    xhr.upload.onprogress = (e) => e.lengthComputable && onProgress(e.loaded / e.total);
    xhr.onload = () => {
      if (xhr.status >= 200 && xhr.status < 300) resolve();
      else if (xhr.status === 413) reject(new Error("Discord rejected the file as too large for this server. Pick a smaller size limit."));
      else reject(new Error(`Discord answered ${xhr.status}: ${xhr.responseText.slice(0, 200)}`));
    };
    xhr.onerror = () => reject(new Error("network error while uploading to Discord"));
    xhr.send(form);
  });
}

export function SharePanel({ clip }: { clip: ClipInfo }) {
  const settings = useSettingsStore((s) => s.settings);
  const game = useStatusStore((s) => s.status?.game?.name);
  const [tracks, setTracks] = useState<string[] | null>(null);
  const [selected, setSelected] = useState<boolean[]>([]);
  const [maxMb, setMaxMb] = useState(String(settings?.share.maxMb ?? 10));
  const [busy, setBusy] = useState<"export" | "send" | null>(null);
  const [progress, setProgress] = useState(0);
  const [result, setResult] = useState<ShareResult | null>(null);

  useEffect(() => {
    let alive = true;
    setResult(null);
    api.clipTracks(clip.path).then(
      (t) => {
        if (!alive) return;
        setTracks(t);
        setSelected(t.map(() => true));
      },
      () => alive && setTracks([]),
    );
    return () => {
      alive = false;
    };
  }, [clip.path]);

  const webhook = settings?.share.discordWebhook ?? "";
  const canSend = isDiscordWebhook(webhook);

  const exportCopy = async () => {
    setBusy("export");
    try {
      const all = selected.every(Boolean);
      const idx = selected.flatMap((on, i) => (on ? [i] : []));
      const r = await api.shareClip(clip.path, all ? null : idx, Number(maxMb));
      setResult(r);
    } catch (err) {
      notify({ kind: "error", title: "Share copy failed", body: errorMessage(err) });
    } finally {
      setBusy(null);
    }
  };

  const send = async () => {
    if (!result) return;
    setBusy("send");
    setProgress(0);
    try {
      const file = result.path.split(/[\\/]/).pop() ?? "";
      const message = (settings?.share.discordMessage ?? "")
        .replaceAll("{game}", game ?? "Lanes")
        .replaceAll("{preset}", clip.kind === "clip" ? "clip" : "recording")
        .replaceAll("{file}", file);
      await sendToDiscord(webhook, result.path, message, setProgress);
      notify({ kind: "success", title: "Sent to Discord" });
    } catch (err) {
      notify({ kind: "error", title: "Sending to Discord failed", body: errorMessage(err) });
    } finally {
      setBusy(null);
    }
  };

  return (
    <div className="flex flex-col gap-3 border-t border-zinc-800 px-4 py-3">
      <div className="flex flex-wrap items-center gap-3">
        <span className="text-sm font-medium text-zinc-300">Share</span>
        {tracks === null ? (
          <Spinner />
        ) : tracks.length > 0 ? (
          <div className="flex flex-wrap items-center gap-3 text-sm text-zinc-300">
            <span className="text-xs text-zinc-500">Mix into one track:</span>
            {tracks.map((t, i) => (
              <label key={i} className="flex items-center gap-1.5">
                <input
                  type="checkbox"
                  className="accent-sky-500"
                  checked={selected[i] ?? true}
                  onChange={(e) => setSelected((s) => s.map((v, j) => (j === i ? e.target.checked : v)))}
                />
                {t}
              </label>
            ))}
          </div>
        ) : (
          <span className="text-xs text-zinc-500">No audio</span>
        )}
        <div className="ml-auto flex items-center gap-2">
          <Select value={maxMb} onChange={setMaxMb} options={SIZE_OPTIONS} />
          <Button size="sm" onClick={() => void exportCopy()} disabled={busy !== null}>
            {busy === "export" && <Spinner />} Create share copy
          </Button>
        </div>
      </div>
      <p className="text-xs text-zinc-500">
        Discord and most players only play the first audio track, so the share copy mixes the lanes you pick into one.
        It is only re-encoded (on the GPU) when it has to fit the size limit.
      </p>
      {result && (
        <div className="flex flex-wrap items-center gap-2 rounded-md bg-zinc-900/80 px-3 py-2">
          <span className="min-w-0 flex-1 truncate text-xs text-zinc-400" title={result.path}>
            {result.path.split(/[\\/]/).pop()} · {formatBytes(result.sizeBytes)}
            {result.reencoded ? " · re-encoded" : ""}
          </span>
          <DragHandle path={result.path} />
          <Button
            size="sm"
            variant="ghost"
            onClick={() =>
              void api.copyClipFile(result.path).then(
                () => notify({ kind: "success", title: "Copied — press Ctrl+V in Discord to upload" }),
                (err: unknown) => notify({ kind: "error", title: "Copy failed", body: errorMessage(err) }),
              )
            }
          >
            Copy file
          </Button>
          <Button size="sm" variant="ghost" onClick={() => void api.revealClip(result.path)}>
            Show in folder
          </Button>
          <Button
            size="sm"
            onClick={() => void send()}
            disabled={!canSend || busy !== null}
            title={canSend ? "Upload to your Discord channel" : "Add a Discord webhook in Settings → Sharing"}
          >
            {busy === "send" ? `Sending ${Math.round(progress * 100)} %` : "Send to Discord"}
          </Button>
        </div>
      )}
    </div>
  );
}
