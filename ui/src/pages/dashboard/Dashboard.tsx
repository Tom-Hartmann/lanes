import { useState } from "react";
import { IconChevron } from "../../components/icons";
import { LogPanel } from "../../components/LogPanel";
import { RecordingCard, ReplayCard, SessionCard } from "./CaptureCards";
import { ErrorsCard, MetersCard, StatsCard } from "./LiveCards";

export function Dashboard() {
  const [logOpen, setLogOpen] = useState(false);
  return (
    <div className="flex flex-col gap-4 p-5">
      <div className="grid gap-4 @3xl:grid-cols-2 @6xl:grid-cols-3">
        <RecordingCard />
        <ReplayCard />
        <SessionCard />
      </div>
      <div className="grid gap-4 @3xl:grid-cols-2">
        <MetersCard />
        <ErrorsCard />
      </div>
      <StatsCard />
      <section>
        <button
          onClick={() => setLogOpen((o) => !o)}
          className="flex items-center gap-1.5 text-sm text-zinc-400 hover:text-zinc-200"
        >
          <IconChevron open={logOpen} size={14} />
          Log
        </button>
        {logOpen && (
          <div className="mt-2 flex h-72">
            <LogPanel />
          </div>
        )}
      </section>
    </div>
  );
}
