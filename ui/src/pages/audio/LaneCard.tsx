import { IconDown, IconUp, IconX } from "../../components/icons";
import { LaneMeter } from "../../components/Meter";
import { useStatusStore } from "../../store/statusStore";
import { ConfirmButton, cx, DbSlider, TextInput, ToggleChip } from "../../components/ui";
import type { AudioLayout, Lane } from "../../lib/ipc";
import type { SaveMode } from "../../store/settingsStore";
import { moveAssignment, moveLane, patchLane, removeLane, setAssigned } from "./audioOps";
import { startSourceDrag, useSourceDrop } from "./dnd";
import { SourceIcon } from "./SourceIcon";

export type EditAudio = (fn: (a: AudioLayout) => AudioLayout, mode?: SaveMode) => void;

interface Props {
  lane: Lane;
  index: number;
  layout: AudioLayout;
  edit: EditAudio;
}

export function LaneCard({ lane, index, layout, edit }: Props) {
  const feeding = layout.sources.filter((s) => s.lanes.includes(lane.id));
  const sessionLanes = useStatusStore((s) => s.status?.sessionLanes);
  const live = !sessionLanes || sessionLanes.length === 0 || sessionLanes.includes(lane.id);
  const drop = useSourceDrop((d, copy) =>
    edit((a) =>
      d.fromLane && !copy ? moveAssignment(a, d.sourceId, d.fromLane, lane.id) : setAssigned(a, d.sourceId, lane.id, true),
    ),
  );
  const patch = (p: Partial<Lane>, mode?: SaveMode) => edit((a) => patchLane(a, lane.id, p), mode);

  return (
    <div
      {...drop.props}
      className={cx(
        "rounded-lg border p-3 transition-colors",
        drop.over ? "border-sky-500 bg-sky-500/10" : "border-zinc-800 bg-zinc-900/60",
      )}
    >
      <div className="flex items-center gap-1.5">
        <span className="w-6 shrink-0 text-center font-mono text-xs text-zinc-500" title={`Audio track ${index + 1}`}>
          {index + 1}
        </span>
        <TextInput
          value={lane.name}
          onChange={(name) => patch({ name }, "debounced")}
          className="min-w-0 flex-1 border-transparent bg-transparent px-1 font-medium hover:border-zinc-700"
        />
        <div className="flex shrink-0 items-center gap-1">
          <button
            className="rounded p-0.5 text-zinc-500 hover:bg-zinc-800 hover:text-zinc-200 disabled:opacity-30"
            disabled={index === 0}
            title="Move up"
            onClick={() => edit((a) => moveLane(a, lane.id, -1))}
          >
            <IconUp size={14} />
          </button>
          <button
            className="rounded p-0.5 text-zinc-500 hover:bg-zinc-800 hover:text-zinc-200 disabled:opacity-30"
            disabled={index === layout.lanes.length - 1}
            title="Move down"
            onClick={() => edit((a) => moveLane(a, lane.id, 1))}
          >
            <IconDown size={14} />
          </button>
          <ToggleChip active={lane.muted} tone="red" title="Mute lane" onClick={() => patch({ muted: !lane.muted })}>
            M
          </ToggleChip>
          <ToggleChip active={lane.mono} title="Downmix lane to mono" onClick={() => patch({ mono: !lane.mono })}>
            Mono
          </ToggleChip>
          <ConfirmButton
            title="Delete lane"
            confirmLabel="Delete?"
            onConfirm={() => edit((a) => removeLane(a, lane.id))}
          >
            <IconX size={14} />
          </ConfirmButton>
        </div>
      </div>

      <div className="mt-2 flex items-center gap-3 pl-7">
        <DbSlider value={lane.volumeDb} onChange={(volumeDb) => patch({ volumeDb }, "debounced")} className="w-48 max-w-[50%]" />
        {live ? (
          <LaneMeter id={lane.id} className="flex-1" />
        ) : (
          <span className="flex-1 text-xs text-amber-400/90" title="Lanes added while capturing are recorded after a restart">
            Not in the running capture yet
          </span>
        )}
      </div>

      <div className="mt-2 flex min-h-8 flex-wrap items-center gap-1.5 pl-7">
        {feeding.length === 0 && (
          <span className={cx("text-xs", drop.over ? "text-sky-300" : "text-zinc-600")}>
            Drop sources here. This lane records silence until something feeds it.
          </span>
        )}
        {feeding.map((s) => (
          <span
            key={s.id}
            draggable
            onDragStart={(e) => startSourceDrag(e, { sourceId: s.id, fromLane: lane.id })}
            title="Drag to another lane to move it, hold Ctrl to copy"
            className={cx(
              "inline-flex cursor-grab items-center gap-1.5 rounded-full border py-0.5 pr-1 pl-2 text-xs active:cursor-grabbing",
              s.muted ? "border-zinc-800 text-zinc-500" : "border-zinc-700 bg-zinc-800/70 text-zinc-200",
            )}
          >
            <SourceIcon kind={s.kind} size={12} />
            <span className="max-w-32 truncate">{s.name}</span>
            <button
              className="rounded-full p-0.5 text-zinc-500 hover:bg-zinc-700 hover:text-zinc-100"
              title={`Remove ${s.name} from this lane`}
              onClick={() => edit((a) => setAssigned(a, s.id, lane.id, false))}
            >
              <IconX size={11} />
            </button>
          </span>
        ))}
      </div>
    </div>
  );
}
