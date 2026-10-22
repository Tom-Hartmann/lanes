import { api } from "../lib/ipc";
import { attempt } from "../store/noticeStore";
import { useIsRecording, useStatusStore } from "../store/statusStore";
import { Button } from "./ui";

/** Lane, video or encoder changes wait for the capture to restart; says so and offers it. */
export function PendingRestart() {
  const pending = useStatusStore((s) => s.status?.pendingRestart ?? false);
  const recording = useIsRecording();
  if (!pending) return null;
  return (
    <div className="sticky top-0 z-10 flex flex-wrap items-center gap-3 border-b border-amber-700/50 bg-amber-950/90 px-6 py-2 text-xs text-amber-100 backdrop-blur">
      <span className="min-w-0 flex-1">
        {recording
          ? "Lane, video or encoder changes apply to the next recording."
          : "Lane, video or encoder changes apply after the capture restarts. Restarting empties the replay buffer."}
      </span>
      {!recording && (
        <Button size="sm" variant="primary" onClick={() => void attempt("Could not restart the capture", api.applyCaptureChanges)}>
          Restart capture now
        </Button>
      )}
    </div>
  );
}
