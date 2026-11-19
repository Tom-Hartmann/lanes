// Browser-only harness. Not imported by the application or production bundle.
import React from "react";
import { createRoot } from "react-dom/client";
import { mockIPC } from "@tauri-apps/api/mocks";
import { emit } from "@tauri-apps/api/event";
import { ClipViewer } from "../src/pages/clips/ClipViewer";
import "../src/index.css";

const state = { calls: [] as { cmd: string; args: any }[], opened: "", cancel: null as null | (() => void) };
(window as any).__editorTest = state;
mockIPC(async (cmd, args: any) => {
  state.calls.push({ cmd, args });
  if (cmd === "prepare_clip_playback") return new URLSearchParams(location.search).get("media");
  if (cmd === "clip_tracks" || cmd === "clip_audio_preview") return [];
  if (cmd === "clip_frame_duration") return 1/30;
  if (cmd === "clip_waveform") return Array.from({ length: 1200 }, (_, i) => Math.abs(Math.sin(i / 20)) * .7);
  if (cmd === "render_clip_edit") {
    await emit("clip_edit_progress", { jobId: args.jobId, fraction: 0.4 });
    if (new URLSearchParams(location.search).has("cancel")) {
      return new Promise((_, reject) => { state.cancel = () => reject(new Error("Export cancelled")); });
    }
    return args.preview ? new URLSearchParams(location.search).get("media") : "edited.mp4";
  }
  if (cmd === "cancel_clip_edit") state.cancel?.();
}, { shouldMockEvents: true });
(window as any).__TAURI_INTERNALS__.convertFileSrc = (path: string) => path;

createRoot(document.getElementById("root")!).render(<ClipViewer clip={{
  path: "test.mp4", fileName: "Test recording", sizeBytes: 1024, createdMs: 0,
  durationSecs: 10, kind: "clip", game: "Test", markers: [], mix: null, mixBaked: null,
}} onClose={() => {}} onChanged={() => {}} onOpenPath={(path) => { state.opened = path; }} />);
