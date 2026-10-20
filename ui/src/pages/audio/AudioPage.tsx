import { useCallback, useEffect, useState } from "react";
import { IconPlus, IconRefresh } from "../../components/icons";
import { Button, Callout, EmptyState, PageHeader, Segmented, Spinner } from "../../components/ui";
import { api, type AudioApp, type AudioDevices, type AudioLayout } from "../../lib/ipc";
import { withAudio } from "../../lib/settings";
import { useAppInfoStore } from "../../store/appInfoStore";
import { notify, reportError } from "../../store/noticeStore";
import { useActiveProfile, useSettingsStore, type SaveMode } from "../../store/settingsStore";
import { useIsRecording } from "../../store/statusStore";
import { AddSource } from "./AddSource";
import { addLane, MAX_LANES, voiceSplit } from "./audioOps";
import { startSourceDrag } from "./dnd";
import { LaneCard } from "./LaneCard";
import { MatrixView } from "./MatrixView";
import { SourceCard } from "./SourceCard";
import { SourceIcon } from "./SourceIcon";

type View = "lanes" | "matrix";

function useAudioInventory() {
  const [devices, setDevices] = useState<AudioDevices | null>(null);
  const [apps, setApps] = useState<AudioApp[] | null>(null);
  const [loading, setLoading] = useState(false);
  const refresh = useCallback(async () => {
    setLoading(true);
    const [d, a] = await Promise.allSettled([api.listAudioDevices(), api.listAudioApps()]);
    if (d.status === "fulfilled") setDevices(d.value);
    else reportError("Could not list audio devices", d.reason);
    if (a.status === "fulfilled") setApps(a.value);
    else setApps([]);
    setLoading(false);
  }, []);
  // Fetched once per page visit; the Refresh button updates it.
  useEffect(() => void refresh(), [refresh]);
  return { devices, apps, loading, refresh };
}

export function AudioPage() {
  const profile = useActiveProfile();
  const updateActiveProfile = useSettingsStore((s) => s.updateActiveProfile);
  const perAppSupported = useAppInfoStore((s) => s.info?.perAppAudioSupported ?? true);
  const recording = useIsRecording();
  const { devices, apps, loading, refresh } = useAudioInventory();
  const [view, setView] = useState<View>("lanes");
  const [adding, setAdding] = useState(false);

  if (!profile) return <div className="p-5 text-sm text-zinc-500">Loading settings…</div>;
  const layout = profile.audio;
  const edit = (fn: (a: AudioLayout) => AudioLayout, mode: SaveMode = "now") => updateActiveProfile(withAudio(fn), mode);
  const laneNames = new Map(layout.lanes.map((l) => [l.id, l.name]));

  const splitVoices = () => {
    const r = voiceSplit(layout, apps ?? [], perAppSupported);
    if (r.lanesAdded === 0 && r.sourcesAdded === 0) {
      notify({ kind: "info", title: "Every mic and voice app already has its own lane." });
      return;
    }
    edit(() => r.layout);
    notify({
      kind: "success",
      title: `Voice splitting: ${r.lanesAdded} lane${r.lanesAdded === 1 ? "" : "s"} added`,
      body: r.sourcesAdded > 0 ? `${r.sourcesAdded} source${r.sourcesAdded === 1 ? "" : "s"} added too.` : undefined,
    });
  };

  return (
    <div className="flex flex-col gap-4 p-5">
      <PageHeader
        title="Audio Lanes"
        subtitle={
          <>
            Each lane becomes its own audio track in the file of <span className="text-zinc-300">{profile.name}</span>.
            Drag sources onto lanes; a source can feed several lanes.
          </>
        }
        actions={
          <>
            <Button
              size="sm"
              onClick={splitVoices}
              title="Give every mic and voice app (Discord, TeamSpeak, Mumble) its own lane. Existing assignments are kept: remove voices from a mixed lane yourself if you want them only on their own track."
            >
              Split voices
            </Button>
            <Button size="sm" variant="ghost" disabled={loading} onClick={() => void refresh()} title="Refresh devices and apps">
              {loading ? <Spinner /> : <IconRefresh size={14} />}
            </Button>
            <Segmented<View>
              value={view}
              onChange={setView}
              options={[
                { value: "lanes", label: "Lanes" },
                { value: "matrix", label: "Matrix" },
              ]}
            />
          </>
        }
      />

      <Callout tone={recording ? "warn" : "info"}>
        {recording && <strong className="font-semibold">Recording in progress. </strong>}
        Adding or removing lanes applies to the next recording. Volumes, mutes and assignments apply live.
      </Callout>
      {!perAppSupported && (
        <Callout tone="warn">
          Per-app audio capture needs Windows 10 version 2004 (build 19041) or newer. App sources are disabled on this
          system; use system output or input sources instead.
        </Callout>
      )}

      <div className="grid items-start gap-4 @4xl:grid-cols-2">
        {/* Lanes (or matrix) */}
        <section className="flex min-w-0 flex-col gap-3">
          <div className="flex items-center justify-between">
            <h2 className="text-sm font-medium text-zinc-300">
              Lanes <span className="text-zinc-600">{layout.lanes.length}</span>
            </h2>
            <Button
              size="sm"
              disabled={layout.lanes.length >= MAX_LANES}
              onClick={() => edit((a) => addLane(a).layout)}
              title={layout.lanes.length >= MAX_LANES ? `At most ${MAX_LANES} lanes` : undefined}
            >
              <IconPlus size={14} /> Lane
            </Button>
          </div>

          {view === "lanes" && layout.sources.length > 0 && (
            <div className="flex flex-wrap items-center gap-1.5 rounded-lg border border-dashed border-zinc-800 px-2.5 py-2">
              <span className="mr-1 text-[11px] text-zinc-500">Drag:</span>
              {layout.sources.map((s) => (
                <span
                  key={s.id}
                  draggable
                  onDragStart={(e) => startSourceDrag(e, { sourceId: s.id })}
                  className="inline-flex cursor-grab items-center gap-1.5 rounded-full border border-zinc-700 bg-zinc-800 px-2 py-0.5 text-xs text-zinc-200 hover:border-zinc-500 active:cursor-grabbing"
                >
                  <SourceIcon kind={s.kind} size={12} />
                  <span className="max-w-28 truncate">{s.name}</span>
                </span>
              ))}
            </div>
          )}

          {view === "lanes" ? (
            layout.lanes.length === 0 ? (
              <EmptyState>No lanes yet. Add one, then drag sources onto it.</EmptyState>
            ) : (
              layout.lanes.map((l, i) => <LaneCard key={l.id} lane={l} index={i} layout={layout} edit={edit} />)
            )
          ) : (
            <MatrixView layout={layout} edit={edit} />
          )}
        </section>

        {/* Sources */}
        <section className="flex min-w-0 flex-col gap-3">
          <div className="flex items-center justify-between">
            <h2 className="text-sm font-medium text-zinc-300">
              Sources <span className="text-zinc-600">{layout.sources.length}</span>
            </h2>
            {!adding && (
              <Button size="sm" onClick={() => setAdding(true)}>
                <IconPlus size={14} /> Source
              </Button>
            )}
          </div>
          {adding && (
            <AddSource
              devices={devices}
              apps={apps}
              perAppSupported={perAppSupported}
              laneCount={layout.lanes.length}
              edit={edit}
              onDone={() => setAdding(false)}
            />
          )}
          {layout.sources.length === 0 && !adding && (
            <EmptyState>No sources. Add system audio, an app or a microphone.</EmptyState>
          )}
          {layout.sources.map((s) => (
            <SourceCard
              key={s.id}
              source={s}
              laneNames={laneNames}
              devices={devices}
              apps={apps}
              perAppSupported={perAppSupported}
              edit={edit}
            />
          ))}
        </section>
      </div>
    </div>
  );
}
