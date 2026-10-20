import { useEffect, useState, type ReactNode } from "react";
import { LaneMeter } from "../../components/Meter";
import { Badge, Button, Card, cx } from "../../components/ui";
import { formatBytes, formatRate } from "../../lib/format";
import { useActiveProfile } from "../../store/settingsStore";
import { useStatusStore } from "../../store/statusStore";

export function MetersCard() {
  const lanes = useActiveProfile()?.audio.lanes ?? [];
  return (
    <Card title="Audio lanes">
      {lanes.length === 0 ? (
        <p className="text-sm text-zinc-500">No lanes. Add some on the Audio Lanes page.</p>
      ) : (
        <div className="space-y-2">
          {lanes.map((l, i) => (
            <div key={l.id} className="grid grid-cols-[6.5rem_1fr] items-start gap-3">
              <div className="flex min-w-0 items-center gap-1.5 text-xs">
                <span className="text-zinc-600 tabular-nums">{i + 1}</span>
                <span className={cx("truncate", l.muted ? "text-zinc-600 line-through" : "text-zinc-300")} title={l.name}>
                  {l.name}
                </span>
              </div>
              <LaneMeter id={l.id} scale={i === lanes.length - 1} className="pt-0.5" />
            </div>
          ))}
        </div>
      )}
    </Card>
  );
}

function Stat({ label, value, sub, tone }: { label: string; value: ReactNode; sub?: ReactNode; tone?: "warn" | "bad" }) {
  return (
    <div className="min-w-0 rounded-md bg-zinc-950/40 px-3 py-2">
      <div className="text-[11px] tracking-wide text-zinc-500 uppercase">{label}</div>
      <div
        className={cx(
          "truncate font-mono text-sm tabular-nums",
          tone === "bad" ? "text-red-400" : tone === "warn" ? "text-amber-400" : "text-zinc-100",
        )}
      >
        {value}
      </div>
      {sub && <div className="truncate text-[11px] text-zinc-500">{sub}</div>}
    </div>
  );
}

/** Renders the 1 Hz stats; it is the only component subscribed to them. */
export function StatsCard() {
  const stats = useStatusStore((s) => s.stats);
  const statsAt = useStatusStore((s) => s.statsAtMs);
  const [now, setNow] = useState(() => Date.now());

  // Detect when stats stop arriving (capture ended) with a slow check.
  useEffect(() => {
    const t = setInterval(() => setNow(Date.now()), 3000);
    return () => clearInterval(t);
  }, []);

  const fresh = stats !== null && now - statsAt < 4000;

  return (
    <Card title="Performance" actions={!fresh && <Badge>Idle</Badge>}>
      {!stats ? (
        <p className="text-sm text-zinc-500">Live stats appear while Lanes is capturing.</p>
      ) : (
        <div className={cx("grid grid-cols-2 gap-2 @lg:grid-cols-3 @4xl:grid-cols-5", !fresh && "opacity-50")}>
          <Stat
            label="FPS"
            value={`${stats.fps.toFixed(1)} / ${stats.targetFps}`}
            tone={stats.fps < stats.targetFps * 0.9 ? "bad" : stats.fps < stats.targetFps * 0.98 ? "warn" : undefined}
          />
          <Stat
            label="Frames"
            value={`${stats.droppedFrames} dropped`}
            sub={`${stats.duplicatedFrames} duplicated`}
            tone={stats.droppedFrames > 0 ? "warn" : undefined}
          />
          <Stat
            label="Encode"
            value={`${stats.encodeMsAvg.toFixed(1)} ms`}
            sub={`${Math.round(stats.encoderLoad * 100)}% load · queue ${stats.encoderQueue}`}
            tone={stats.encoderLoad > 1 ? "bad" : stats.encoderLoad > 0.8 ? "warn" : undefined}
          />
          <Stat label="Replay buffer" value={formatBytes(stats.bufferBytes)} sub={`${Math.floor(stats.bufferSecs)} s`} />
          <Stat label="Disk write" value={formatRate(stats.diskWriteBps)} sub={`${formatBytes(stats.freeDiskBytes, 0)} free`} />
          <Stat
            label="CPU"
            value={`${stats.cpuPercent.toFixed(1)} %`}
            sub={`${formatBytes(stats.memoryBytes, 0)} memory`}
            tone={stats.cpuPercent > 10 ? "warn" : undefined}
          />
        </div>
      )}
    </Card>
  );
}

export function ErrorsCard() {
  const errors = useStatusStore((s) => s.errors);
  const clear = useStatusStore((s) => s.clearErrors);
  const dismiss = useStatusStore((s) => s.dismissError);
  if (errors.length === 0) return null;
  return (
    <Card
      title={`Recent errors (${errors.length})`}
      actions={
        <Button size="sm" variant="ghost" onClick={clear}>
          Clear
        </Button>
      }
      bodyClassName="p-0"
    >
      <ul className="max-h-48 divide-y divide-zinc-800 overflow-y-auto">
        {errors.map((e) => (
          <li key={e.id} className="flex items-start gap-3 px-4 py-2 text-xs">
            <span className="shrink-0 font-mono text-zinc-600">
              {new Date(e.atMs).toLocaleTimeString(undefined, { hour12: false })}
            </span>
            <Badge tone={e.recoverable ? "amber" : "red"}>{e.source}</Badge>
            <span className="min-w-0 flex-1 break-words text-zinc-300 select-text">{e.message}</span>
            <button onClick={() => dismiss(e.id)} className="shrink-0 text-zinc-600 hover:text-zinc-300">
              ×
            </button>
          </li>
        ))}
      </ul>
    </Card>
  );
}
