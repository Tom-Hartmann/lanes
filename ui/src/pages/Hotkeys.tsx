import { useEffect, useState } from "react";
import { ComboCapture } from "../components/ComboCapture";
import { IconRefresh } from "../components/icons";
import { Button, PageHeader, Select } from "../components/ui";
import { errorMessage, presetIdOfAction, type BindingInfo, type HotkeyBackend } from "../lib/ipc";
import { useHotkeyStore } from "../store/hotkeyStore";

function Status({ b }: { b: BindingInfo }) {
  if (b.error) return <span className="text-red-400">{b.error}</span>;
  if (!b.combo) return <span className="text-zinc-500">Unbound</span>;
  if (b.mode === "registered") return <span className="text-emerald-400">Registered</span>;
  if (b.mode === "hook")
    return (
      <span className="text-amber-400" title="Uses a low-level keyboard hook. The game still receives the key.">
        Keyboard hook
      </span>
    );
  return null;
}

function Row({ b, error, apply }: { b: BindingInfo; error?: string; apply: (combo: string | null, backend: HotkeyBackend) => void }) {
  return (
    <tr>
      <td className="py-2.5 pr-3 text-zinc-200">{b.label}</td>
      <td className="py-2.5 pr-3">
        <ComboCapture value={b.combo} onChange={(c) => apply(c, b.backend)} />
      </td>
      <td className="py-2.5 pr-3">
        <Select<HotkeyBackend>
          value={b.backend}
          onChange={(v) => apply(b.combo, v)}
          options={[
            { value: "auto", label: "Automatic" },
            { value: "hook", label: "Keyboard hook" },
          ]}
        />
      </td>
      <td className="py-2.5 pr-3 text-xs">{error ? <span className="text-red-400">{error}</span> : <Status b={b} />}</td>
      <td className="py-2.5 text-right">
        {b.combo && (
          <button onClick={() => apply(null, b.backend)} className="text-xs text-zinc-500 hover:text-zinc-300">
            Clear
          </button>
        )}
      </td>
    </tr>
  );
}

function Table({ title, rows, errors, apply }: {
  title: string;
  rows: BindingInfo[];
  errors: Record<string, string | undefined>;
  apply: (action: string, combo: string | null, backend: HotkeyBackend) => void;
}) {
  if (rows.length === 0) return null;
  return (
    <section className="mt-5">
      <h2 className="text-xs font-medium tracking-wide text-zinc-500 uppercase">{title}</h2>
      <div className="mt-1 overflow-x-auto">
        <table className="w-full min-w-[34rem] text-sm">
          <thead className="text-left text-[11px] tracking-wide text-zinc-600 uppercase">
            <tr>
              <th className="py-1.5 font-medium">Action</th>
              <th className="py-1.5 font-medium">Combo</th>
              <th className="py-1.5 font-medium">Method</th>
              <th className="py-1.5 font-medium">Status</th>
              <th />
            </tr>
          </thead>
          <tbody className="divide-y divide-zinc-800">
            {rows.map((b) => (
              <Row key={b.action} b={b} error={errors[b.action]} apply={(c, be) => apply(b.action, c, be)} />
            ))}
          </tbody>
        </table>
      </div>
    </section>
  );
}

export function Hotkeys() {
  const bindings = useHotkeyStore((s) => s.bindings);
  const load = useHotkeyStore((s) => s.load);
  const setBinding = useHotkeyStore((s) => s.set);
  const [errors, setErrors] = useState<Record<string, string | undefined>>({});

  // Fresh list on page open; the `hotkeys` event keeps it current afterwards.
  useEffect(() => void load().catch(() => {}), [load]);

  const apply = (action: string, combo: string | null, backend: HotkeyBackend) => {
    setErrors((e) => ({ ...e, [action]: undefined }));
    setBinding(action, combo, backend).catch((err: unknown) =>
      setErrors((e) => ({ ...e, [action]: errorMessage(err) })),
    );
  };

  const general = bindings.filter((b) => presetIdOfAction(b.action) === null);
  const clips = bindings.filter((b) => presetIdOfAction(b.action) !== null);

  return (
    <div className="p-5">
      <PageHeader
        title="Hotkeys"
        subtitle={
          <>
            Hotkeys work while a game has focus. If another app already owns a combo, Lanes switches to a keyboard hook
            for it automatically. Choose “Keyboard hook” when the game should still receive the key. Every clip preset
            gets its own hotkey; add presets in Settings.
          </>
        }
        actions={
          <Button size="sm" variant="ghost" onClick={() => void load().catch(() => {})}>
            <IconRefresh size={14} /> Refresh
          </Button>
        }
      />
      <Table title="Capture" rows={general} errors={errors} apply={apply} />
      <Table title="Save clip" rows={clips} errors={errors} apply={apply} />
      {bindings.length === 0 && <p className="mt-6 text-sm text-zinc-500">Loading hotkeys…</p>}
    </div>
  );
}
