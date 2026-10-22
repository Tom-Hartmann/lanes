import { useState } from "react";
import { Elapsed } from "../../components/Elapsed";
import { Badge, Button, Card, cx } from "../../components/ui";
import { formatBytes } from "../../lib/format";
import { api, saveClipAction, type ClipPreset } from "../../lib/ipc";
import { useHotkeyStore } from "../../store/hotkeyStore";
import { attempt } from "../../store/noticeStore";
import { useSettingsStore } from "../../store/settingsStore";
import { useStatusStore } from "../../store/statusStore";

/** Wraps a command so its button shows a busy state and errors become toasts. */
function useCommand() {
  const [busy, setBusy] = useState<string | null>(null);
  const run = (key: string, title: string, fn: () => Promise<unknown>) => {
    setBusy(key);
    void attempt(title, fn).finally(() => setBusy(null));
  };
  return { busy, run };
}

export function RecordingCard() {
  const rec = useStatusStore((s) => s.status?.recording);
  const { busy, run } = useCommand();
  const bindings = useHotkeyStore((s) => s.bindings);
  const combo = (action: string) => bindings.find((b) => b.action === action)?.combo;

  const recording = rec?.state === "recording";
  const paused = rec?.state === "recording" && rec.paused;

  return (
    <Card title="Recording">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div className="flex items-center gap-3">
          <span
            className={cx(
              "h-3 w-3 rounded-full",
              !recording ? "bg-zinc-700" : paused ? "bg-amber-400" : "animate-pulse bg-red-500",
            )}
          />
          <div>
            <div className="text-xl font-semibold text-zinc-100">
              {!rec ? "…" : !recording ? "Idle" : paused ? "Paused" : "Recording"}
            </div>
            {rec?.state === "recording" && (
              <Elapsed sinceMs={rec.sinceMs} paused={rec.paused} className="font-mono text-sm text-zinc-400 tabular-nums" />
            )}
          </div>
        </div>
        <div className="flex gap-2">
          {recording ? (
            <>
              <Button
                disabled={busy !== null}
                onClick={() => run("pause", "Could not pause", api.togglePause)}
                title={combo("toggle_pause") ?? undefined}
              >
                {paused ? "Resume" : "Pause"}
              </Button>
              <Button
                variant="danger"
                disabled={busy !== null}
                onClick={() => run("stop", "Could not stop recording", api.stopRecording)}
                title={combo("toggle_recording") ?? undefined}
              >
                Stop
              </Button>
            </>
          ) : (
            <Button
              variant="record"
              disabled={busy !== null || !rec}
              onClick={() => run("start", "Could not start recording", api.startRecording)}
              title={combo("toggle_recording") ?? undefined}
            >
              <span className="h-2 w-2 rounded-full bg-white" />
              Record
            </Button>
          )}
        </div>
      </div>
      {rec?.state === "recording" && rec.files.length > 0 && (
        <ul className="mt-3 space-y-0.5 text-xs text-zinc-500">
          {rec.files.map((f) => (
            <li key={f} className="truncate font-mono select-text" title={f}>
              {f}
            </li>
          ))}
        </ul>
      )}
    </Card>
  );
}

const NO_PRESETS: ClipPreset[] = [];

export function ReplayCard() {
  const replay = useStatusStore((s) => s.status?.replay);
  const presets = useSettingsStore((s) => s.settings?.clipPresets) ?? NO_PRESETS;
  const replaySettings = useSettingsStore((s) => s.settings?.replay);
  const bindings = useHotkeyStore((s) => s.bindings);
  const { busy, run } = useCommand();

  const active = replay?.active ?? false;
  const fill = replay && replay.capacitySecs > 0 ? Math.min(1, replay.bufferedSecs / replay.capacitySecs) : 0;
  const capBytes = replaySettings?.storage === "ram" ? replaySettings.ramCapMb * 1024 * 1024 : null;
  const bookmarkCombo = bindings.find((b) => b.action === "bookmark")?.combo;

  return (
    <Card
      title="Replay buffer"
      actions={
        <Button
          size="sm"
          variant={active ? "default" : "primary"}
          disabled={busy !== null || !replay}
          onClick={() =>
            active
              ? run("replay", "Could not stop the replay buffer", api.stopReplay)
              : run("replay", "Could not start the replay buffer", api.startReplay)
          }
        >
          {active ? "Turn off" : "Turn on"}
        </Button>
      }
    >
      <div className="flex items-baseline justify-between gap-2 text-sm">
        <span className={active ? "text-sky-300" : "text-zinc-500"}>{active ? "Buffering" : "Off"}</span>
        {replay && (
          <span className="font-mono text-xs text-zinc-400 tabular-nums">
            {Math.floor(replay.bufferedSecs)} / {Math.round(replay.capacitySecs)} s · {formatBytes(replay.bufferedBytes)}
            {capBytes !== null && ` of ${formatBytes(capBytes, 0)}`}
          </span>
        )}
      </div>
      <div className="mt-2 h-1.5 overflow-hidden rounded-full bg-zinc-800">
        <div
          className={cx("h-full rounded-full transition-[width] duration-700", active ? "bg-sky-500" : "bg-zinc-600")}
          style={{ width: `${fill * 100}%` }}
        />
      </div>
      <div className="mt-3 flex flex-wrap gap-2">
        {presets.map((p) => {
          const combo = bindings.find((b) => b.action === saveClipAction(p.id))?.combo;
          return (
            <Button
              key={p.id}
              size="sm"
              disabled={!active || busy !== null}
              title={active ? `Save the last ${p.lengthSecs} s` : "Turn on the replay buffer to save clips"}
              onClick={() => run(`clip:${p.id}`, `Could not save "${p.name}"`, () => api.saveClip(p.id))}
            >
              Save {p.name}
              <span className="text-zinc-500">{p.lengthSecs} s</span>
              {combo && <span className="font-mono text-[10px] text-zinc-500">{combo}</span>}
            </Button>
          );
        })}
        {presets.length === 0 && <span className="text-xs text-zinc-500">No clip presets. Add one in Settings.</span>}
        <Button
          size="sm"
          variant="ghost"
          className="ml-auto"
          title="Mark this moment: clips and recordings that contain it show a marker on the timeline"
          onClick={() => run("bookmark", "Could not set a bookmark", api.bookmark)}
        >
          ★ Bookmark
          {bookmarkCombo && <span className="font-mono text-[10px] text-zinc-500">{bookmarkCombo}</span>}
        </Button>
      </div>
    </Card>
  );
}

export function SessionCard() {
  const status = useStatusStore((s) => s.status);
  const profiles = useSettingsStore((s) => s.settings?.profiles);
  const games = useSettingsStore((s) => s.settings?.games);
  const update = useSettingsStore((s) => s.update);
  const profile = profiles?.find((p) => p.id === status?.activeProfile);
  const game = status?.game;
  // Only heuristic detections can be ignored; listed games are disabled in the list.
  const guessed = game && !games?.some((g) => g.exe.toLowerCase() === game.exe.toLowerCase());
  const ignore = (exe: string) =>
    update((s) => ({
      ...s,
      gameDetection: {
        ...s.gameDetection,
        ignored: s.gameDetection.ignored.some((x) => x.toLowerCase() === exe.toLowerCase())
          ? s.gameDetection.ignored
          : [...s.gameDetection.ignored, exe],
      },
    }));

  return (
    <Card title="Session">
      <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1.5 text-sm">
        <dt className="text-zinc-500">Game</dt>
        <dd className="min-w-0 truncate text-zinc-200">
          {status?.game ? (
            <>
              {status.game.name} <span className="font-mono text-xs text-zinc-500">{status.game.exe}</span>
              {status.game.fullscreen && (
                <span className="ml-2">
                  <Badge>Fullscreen</Badge>
                </span>
              )}
              {guessed && (
                <button
                  type="button"
                  onClick={() => ignore(status.game!.exe)}
                  title="Never treat this program as a game (undo under Profiles → Game detection)"
                  className="ml-2 text-xs text-zinc-500 underline-offset-2 hover:text-zinc-200 hover:underline"
                >
                  Not a game
                </button>
              )}
            </>
          ) : (
            <span className="text-zinc-500">No game detected</span>
          )}
        </dd>
        <dt className="text-zinc-500">Profile</dt>
        <dd className="truncate text-zinc-200">{profile?.name ?? status?.activeProfile ?? "…"}</dd>
        <dt className="text-zinc-500">Capturing</dt>
        <dd className="min-w-0 truncate text-zinc-200" title={status?.targets.join(", ")}>
          {status && status.targets.length > 0 ? status.targets.join(" · ") : <span className="text-zinc-500">Nothing</span>}
        </dd>
        <dt className="text-zinc-500">Encoder</dt>
        <dd className="truncate text-zinc-200">{status?.encoder ?? <span className="text-zinc-500">Idle</span>}</dd>
      </dl>
    </Card>
  );
}
