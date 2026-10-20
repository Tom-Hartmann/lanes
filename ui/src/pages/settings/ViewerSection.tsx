import { useEffect, useState } from "react";
import { Button, Card, Field, NumberInput, Select } from "../../components/ui";
import type { Settings } from "../../lib/ipc";
import { conflicts, FIXED_KEYS, keyLabel, normalizeKey, resolveViewerKeys, VIEWER_ACTIONS } from "../../lib/viewerKeys";
import { useSettingsStore } from "../../store/settingsStore";

const MODIFIERS = new Set(["Shift", "Control", "Alt", "Meta", "CapsLock", "AltGraph"]);

/** One key slot: click, then press a key. Esc cancels, Backspace/Delete clears. */
function KeySlot({ value, onChange, bad }: { value: string | undefined; onChange: (key: string | null) => void; bad: boolean }) {
  const [capturing, setCapturing] = useState(false);
  useEffect(() => {
    if (!capturing) return;
    const onKey = (e: KeyboardEvent) => {
      if (MODIFIERS.has(e.key)) return;
      e.preventDefault();
      e.stopPropagation();
      setCapturing(false);
      if (e.key === "Escape") return;
      if (e.key === "Backspace" || e.key === "Delete") onChange(null);
      else if (!FIXED_KEYS.has(e.key)) onChange(normalizeKey(e.key));
    };
    const cancel = () => setCapturing(false);
    window.addEventListener("keydown", onKey, true);
    window.addEventListener("blur", cancel);
    return () => {
      window.removeEventListener("keydown", onKey, true);
      window.removeEventListener("blur", cancel);
    };
  }, [capturing, onChange]);
  return (
    <button
      type="button"
      onClick={() => setCapturing((c) => !c)}
      title="Click, then press a key (Backspace clears, Esc cancels)"
      className={`min-w-20 rounded-md border px-2 py-1 font-mono text-xs ${
        capturing
          ? "border-sky-500 bg-sky-500/10 text-sky-300"
          : bad
            ? "border-red-500/70 text-red-300"
            : "border-zinc-700 bg-zinc-900 text-zinc-200 hover:border-zinc-500"
      }`}
    >
      {capturing ? "press a key…" : value ? keyLabel(value) : "—"}
    </button>
  );
}

export function ViewerSection({ settings }: { settings: Settings }) {
  const update = useSettingsStore((s) => s.update);
  const v = settings.viewer;
  const keys = resolveViewerKeys(v);
  const clash = conflicts(keys);
  const setViewer = (p: Partial<Settings["viewer"]>, mode: "now" | "debounced" = "now") =>
    update((s) => ({ ...s, viewer: { ...s.viewer, ...p } }), mode);

  const setKey = (id: string, slot: number, key: string | null) => {
    const list = [...keys[id]];
    if (key === null) list.splice(slot, 1);
    else list[slot] = key;
    const deduped = list.filter((k, i) => k && list.indexOf(k) === i).slice(0, 2);
    const defaults = VIEWER_ACTIONS.find((a) => a.id === id)?.defaults ?? [];
    const next = { ...v.keys };
    // Store only real changes, so later default changes still apply.
    if (JSON.stringify(deduped) === JSON.stringify(defaults)) delete next[id];
    else next[id] = deduped;
    setViewer({ keys: next });
  };

  return (
    <Card title="Clip viewer">
      <Field label="Short step" hint="← / → seek this far (Shift: long step).">
        <NumberInput
          value={v.shortSeekSecs}
          integer={false}
          step={0.5}
          min={0.1}
          max={60}
          suffix="s"
          onChange={(shortSeekSecs) => setViewer({ shortSeekSecs }, "debounced")}
        />
      </Field>
      <Field label="Long step" hint="J / L seek this far (Shift: double).">
        <NumberInput
          value={v.longSeekSecs}
          integer={false}
          step={1}
          min={0.5}
          max={300}
          suffix="s"
          onChange={(longSeekSecs) => setViewer({ longSeekSecs }, "debounced")}
        />
      </Field>
      <Field label="Slow-motion speed" hint="What the slow-motion key switches to.">
        <Select
          value={String(v.slowMotionSpeed)}
          onChange={(x) => setViewer({ slowMotionSpeed: Number(x) })}
          options={["0.1", "0.25", "0.5", "0.75"].map((x) => ({ value: x, label: `${x}×` }))}
        />
      </Field>

      <div className="mt-2 flex items-center justify-between">
        <h3 className="text-xs font-medium tracking-wide text-zinc-500 uppercase">Keyboard shortcuts</h3>
        <Button size="sm" variant="ghost" onClick={() => setViewer({ keys: {} })} disabled={Object.keys(v.keys).length === 0}>
          Reset all
        </Button>
      </div>
      <table className="w-full text-sm">
        <tbody className="divide-y divide-zinc-800/70">
          {VIEWER_ACTIONS.map((a) => (
            <tr key={a.id}>
              <td className="py-1.5 pr-2 text-zinc-300">{a.label}</td>
              <td className="py-1.5">
                <div className="flex items-center justify-end gap-1.5">
                  {[0, 1].map((slot) => (
                    <KeySlot key={slot} value={keys[a.id][slot]} bad={clash.has(a.id)} onChange={(k) => setKey(a.id, slot, k)} />
                  ))}
                  <button
                    type="button"
                    onClick={() => {
                      const next = { ...v.keys };
                      delete next[a.id];
                      setViewer({ keys: next });
                    }}
                    disabled={!(a.id in v.keys)}
                    className="w-10 text-xs text-zinc-500 hover:text-zinc-200 disabled:invisible"
                  >
                    Reset
                  </button>
                </div>
              </td>
            </tr>
          ))}
        </tbody>
      </table>
      {clash.size > 0 && <p className="mt-2 text-xs text-red-400">Red keys are used by more than one action; only the first action gets them.</p>}
      <p className="mt-2 text-xs text-zinc-500">0–9 jump to 0–90 % and Esc closes the viewer; these are fixed.</p>
    </Card>
  );
}
