import React from "react";
import { createRoot } from "react-dom/client";
import { mockIPC } from "@tauri-apps/api/mocks";
import { emit } from "@tauri-apps/api/event";
import { StudioPanel } from "../src/pages/clips/StudioPanel";
import { ClipDetails } from "../src/pages/clips/ClipDetails";
import type { ClipInfo } from "../src/lib/ipc";
import "../src/index.css";
const calls: { cmd: string; args: any }[] = [];
const testState = { calls, failNext: false, slowNext: false, active: 0 };
(window as any).__studioTest = testState;
const cancellations = new Map<string, () => void>();
mockIPC(async (cmd, args: any) => {
  calls.push({ cmd, args });
  if (cmd === "studio_export" || cmd === "caption_clip") {
    if (testState.failNext) { testState.failNext = false; throw new Error("Fixture export failure"); }
    const delay = testState.slowNext ? 3000 : 250;
    testState.slowNext = false;
    testState.active++;
    await emit("clip_edit_progress", { jobId: args.jobId, fraction: .5 });
    try { await new Promise<void>((resolve, reject) => {
      const timer = setTimeout(resolve, delay);
      cancellations.set(args.jobId, () => { clearTimeout(timer); reject(new Error("Export cancelled")); });
    }); } finally { cancellations.delete(args.jobId); testState.active--; }
    return cmd === "caption_clip" ? "captions.srt" : "compilation.mp4";
  }
  if (cmd === "cleanup_library") return [];
  if (cmd === "cancel_clip_edit") cancellations.get(args.jobId)?.();
  return null;
}, { shouldMockEvents: true });
const clips: ClipInfo[] = ["first.mp4", "second.mp4"].map((path) => ({ path, fileName: path, sizeBytes: 1000000, createdMs: 0, durationSecs: 2, kind: "clip", game: "Test", markers: [], mix: null, mixBaked: null }));
function Harness() {
  const [selected, setSelected] = React.useState(clips);
  return <div className="space-y-4 bg-zinc-900 p-5"><button onClick={() => setSelected([clips[0]])}>Select first only</button><StudioPanel selected={selected} onChanged={() => {}} /><ClipDetails clip={clips[0]} playhead={1.25} onSeek={() => {}} onChanged={() => {}} /></div>;
}
createRoot(document.getElementById("root")!).render(<Harness />);
