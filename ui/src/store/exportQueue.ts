import { create } from "zustand";
import { api, events, errorMessage, type ExportPreset } from "../lib/ipc";
import { useStatusStore } from "./statusStore";

export interface ExportJob {
  id: string; paths: string[]; preset: ExportPreset; label: string; kind: "export" | "import" | "recover" | "captions";
  state: "queued" | "running" | "done" | "failed" | "cancelled"; progress: number | null; result?: string; error?: string;
}
const KEY = "lanes.exportQueue.v1";
function restore(): ExportJob[] {
  try {
    const jobs = JSON.parse(localStorage.getItem(KEY) ?? "[]") as ExportJob[];
    if (!Array.isArray(jobs)) return [];
    return jobs.filter((j) => typeof j.id === "string" && Array.isArray(j.paths) && j.paths.every((p) => typeof p === "string") && j.preset && typeof j.label === "string").slice(-50).map((j) =>
      ["queued", "running"].includes(j.state) ? { ...j, state: "failed", error: "App closed before completion. Check the output folder before retrying." } : j);
  } catch { return []; }
}
export const useExportQueue = create<{ jobs: ExportJob[]; storageError: boolean }>(() => ({ jobs: restore(), storageError: false }));
let running = false;
let persisted = "";
function update(id: string, patch: Partial<ExportJob>) {
  useExportQueue.setState((s) => ({ jobs: s.jobs.map((j) => j.id === id ? { ...j, ...patch } : j) }));
}
useExportQueue.subscribe((s) => {
  const value = JSON.stringify(s.jobs.map((j) => j.state === "running" ? { ...j, progress: null } : j));
  if (value === persisted) return;
  try { localStorage.setItem(KEY, value); persisted = value; }
  catch { if (!s.storageError) useExportQueue.setState({ storageError: true }); }
});
export function enqueue(paths: string[], preset: ExportPreset, label: string, kind: ExportJob["kind"] = "export") {
  const jobs = useExportQueue.getState().jobs;
  if (jobs.filter((j) => j.state === "queued" || j.state === "running").length >= 16) throw new Error("Queue is full (16 jobs)");
  useExportQueue.setState({ jobs: [...jobs.slice(-49), { id: crypto.randomUUID(), paths: [...paths], preset: { ...preset }, label, kind, state: "queued", progress: null }] });
  void pump();
}
export async function cancelJob(job: ExportJob) {
  if (job.state === "queued") update(job.id, { state: "cancelled" });
  else if (job.state === "running" && job.progress !== null) await api.cancelClipEdit(job.id);
}
export function retryJob(job: ExportJob) { enqueue(job.paths, job.preset, job.label, job.kind); }
async function pump() {
  if (running) return;
  running = true;
  try {
    for (;;) {
      const job = useExportQueue.getState().jobs.find((j) => j.state === "queued");
      if (!job) break;
      update(job.id, { state: "running", progress: null });
      let off: (() => void) | undefined;
      try {
        off = await events.onClipEditProgress((p) => { if (p.jobId === job.id) update(job.id, { progress: p.fraction }); });
        const result = job.kind === "captions" ? await api.captionClip(job.paths[0], job.id) : await api.studioExport(job.paths, job.preset, job.id, job.kind === "recover", job.kind === "import");
        update(job.id, { state: "done", result, progress: 1 });
        useStatusStore.setState((s) => ({ clipSeq: s.clipSeq + 1 }));
      } catch (e) {
        const error = errorMessage(e);
        update(job.id, { state: error.toLowerCase().includes("cancelled") ? "cancelled" : "failed", error });
      } finally { off?.(); }
    }
  } finally { running = false; }
}
