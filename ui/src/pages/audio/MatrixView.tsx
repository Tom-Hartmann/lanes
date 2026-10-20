import { EmptyState } from "../../components/ui";
import type { AudioLayout } from "../../lib/ipc";
import { setAssigned } from "./audioOps";
import type { EditAudio } from "./LaneCard";
import { SourceIcon } from "./SourceIcon";

/** Sources × lanes assignment table. */
export function MatrixView({ layout, edit }: { layout: AudioLayout; edit: EditAudio }) {
  if (layout.sources.length === 0 || layout.lanes.length === 0)
    return <EmptyState>Add at least one lane and one source to assign them.</EmptyState>;

  return (
    <div className="overflow-x-auto rounded-lg border border-zinc-800 bg-zinc-900/60">
      <table className="w-full text-sm">
        <thead>
          <tr className="border-b border-zinc-800">
            <th className="sticky left-0 bg-zinc-900 px-3 py-2 text-left text-xs font-medium text-zinc-500">Source ╲ Lane</th>
            {layout.lanes.map((l, i) => (
              <th key={l.id} className="px-2 py-2 text-center text-xs font-medium text-zinc-300">
                <div className="font-mono text-[10px] text-zinc-600">{i + 1}</div>
                <div className="mx-auto max-w-24 truncate" title={l.name}>
                  {l.name}
                </div>
              </th>
            ))}
          </tr>
        </thead>
        <tbody className="divide-y divide-zinc-800/70">
          {layout.sources.map((s) => (
            <tr key={s.id} className="hover:bg-zinc-800/30">
              <td className="sticky left-0 bg-zinc-900 px-3 py-1.5">
                <div className="flex max-w-48 items-center gap-2 text-zinc-200">
                  <span className="text-zinc-500">
                    <SourceIcon kind={s.kind} />
                  </span>
                  <span className="truncate">{s.name}</span>
                </div>
              </td>
              {layout.lanes.map((l) => {
                const on = s.lanes.includes(l.id);
                return (
                  <td key={l.id} className="px-2 py-1.5 text-center">
                    <input
                      type="checkbox"
                      checked={on}
                      aria-label={`${s.name} → ${l.name}`}
                      onChange={(e) => edit((a) => setAssigned(a, s.id, l.id, e.target.checked))}
                      className="h-4 w-4 cursor-pointer accent-sky-500"
                    />
                  </td>
                );
              })}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
