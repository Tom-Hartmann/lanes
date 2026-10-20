import { useEffect, useState } from "react";
import { comboFromEvent } from "../lib/keys";

interface Props {
  value: string | null;
  onChange: (combo: string) => void;
}

/** A button that records the next key combination pressed. Esc cancels. */
export function ComboCapture({ value, onChange }: Props) {
  const [capturing, setCapturing] = useState(false);

  useEffect(() => {
    if (!capturing) return;
    const onKey = (e: KeyboardEvent) => {
      e.preventDefault();
      if (e.code === "Escape" && !e.ctrlKey && !e.altKey && !e.shiftKey) {
        setCapturing(false);
        return;
      }
      const combo = comboFromEvent(e);
      if (combo) {
        setCapturing(false);
        onChange(combo);
      }
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
      className={`min-w-44 rounded-md border px-3 py-1.5 text-left font-mono text-sm transition-colors ${
        capturing
          ? "border-sky-500 bg-sky-500/10 text-sky-300"
          : "border-zinc-700 bg-zinc-900 text-zinc-100 hover:border-zinc-500"
      }`}
    >
      {capturing ? "Press keys… (Esc cancels)" : (value ?? "Not set")}
    </button>
  );
}
