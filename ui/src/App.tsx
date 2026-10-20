import { useEffect, useState, type ComponentType } from "react";
import { Elapsed } from "./components/Elapsed";
import {
  IconDashboard,
  IconFilm,
  IconKeyboard,
  IconLanes,
  IconMonitor,
  IconSettings,
  IconUsers,
} from "./components/icons";
import { Toasts } from "./components/Toasts";
import { cx } from "./components/ui";
import { formatSecs } from "./lib/format";
import { api, events } from "./lib/ipc";
import { connectMeters } from "./lib/meters";
import { AudioPage } from "./pages/audio/AudioPage";
import { ClipsPage } from "./pages/clips/ClipsPage";
import { Dashboard } from "./pages/dashboard/Dashboard";
import { Hotkeys } from "./pages/Hotkeys";
import { ProfilesPage } from "./pages/profiles/ProfilesPage";
import { SettingsPage } from "./pages/settings/SettingsPage";
import { SourcesPage } from "./pages/sources/SourcesPage";
import { useAppInfoStore } from "./store/appInfoStore";
import { useHotkeyStore } from "./store/hotkeyStore";
import { useLogStore } from "./store/logStore";
import { notify } from "./store/noticeStore";
import { useSettingsStore } from "./store/settingsStore";
import { useStatusStore } from "./store/statusStore";

const PAGES = [
  { id: "dashboard", label: "Dashboard", icon: IconDashboard, component: Dashboard },
  { id: "sources", label: "Sources", icon: IconMonitor, component: SourcesPage },
  { id: "audio", label: "Audio Lanes", icon: IconLanes, component: AudioPage },
  { id: "clips", label: "Clips", icon: IconFilm, component: ClipsPage },
  { id: "hotkeys", label: "Hotkeys", icon: IconKeyboard, component: Hotkeys },
  { id: "profiles", label: "Profiles", icon: IconUsers, component: ProfilesPage },
  { id: "settings", label: "Settings", icon: IconSettings, component: SettingsPage },
] as const satisfies readonly { id: string; label: string; icon: ComponentType<{ size?: number }>; component: ComponentType }[];

type PageId = (typeof PAGES)[number]["id"];

/** Event subscriptions that live for the whole app session. */
function useBackendConnections() {
  const connectLogs = useLogStore((s) => s.connect);
  const connectStatus = useStatusStore((s) => s.connect);
  const connectSettings = useSettingsStore((s) => s.connect);
  const loadSettings = useSettingsStore((s) => s.load);
  const connectHotkeys = useHotkeyStore((s) => s.connect);
  const loadHotkeys = useHotkeyStore((s) => s.load);
  const loadInfo = useAppInfoStore((s) => s.load);

  useEffect(() => connectLogs(), [connectLogs]);
  useEffect(() => connectStatus(), [connectStatus]);
  useEffect(() => connectSettings(), [connectSettings]);
  useEffect(() => connectHotkeys(), [connectHotkeys]);
  useEffect(() => connectMeters(), []);
  useEffect(() => {
    loadSettings();
    loadHotkeys().catch(() => {});
    loadInfo().catch(() => {});
  }, [loadSettings, loadHotkeys, loadInfo]);

  // In-app notices for saved clips and engine errors.
  useEffect(() => {
    const subs = [
      events.onClipSaved((c) => {
        const presets = useSettingsStore.getState().settings?.clipPresets ?? [];
        const preset = presets.find((p) => p.id === c.preset)?.name ?? c.preset;
        notify({
          kind: "success",
          title: `Clip saved · ${preset} · ${formatSecs(c.durationSecs)}`,
          body: c.path,
          action: { label: "Show in folder", run: () => void api.revealClip(c.path).catch(() => {}) },
        });
      }),
      events.onError((e) => {
        if (!e.recoverable) notify({ kind: "error", title: `${e.source} error`, body: e.message }, 8000);
      }),
    ];
    return () => subs.forEach((p) => void p.then((f) => f()));
  }, []);
}

function CaptureIndicator() {
  const status = useStatusStore((s) => s.status);
  if (!status) return null;
  const rec = status.recording;
  return (
    <div className="flex flex-col gap-1 px-3 text-xs">
      {rec.state === "recording" && (
        <div className="flex items-center gap-2 text-red-400">
          <span className={cx("h-2 w-2 rounded-full bg-red-500", !rec.paused && "animate-pulse")} />
          <span className="font-medium">{rec.paused ? "Paused" : "REC"}</span>
          <Elapsed sinceMs={rec.sinceMs} paused={rec.paused} className="ml-auto font-mono tabular-nums" />
        </div>
      )}
      {status.replay.active && (
        <div className="flex items-center gap-2 text-sky-400">
          <span className="h-2 w-2 rounded-full bg-sky-500" />
          <span className="font-medium">Replay</span>
          <span className="ml-auto font-mono text-zinc-500 tabular-nums">{Math.floor(status.replay.bufferedSecs)} s</span>
        </div>
      )}
    </div>
  );
}

export default function App() {
  const [page, setPage] = useState<PageId>("dashboard");
  const info = useAppInfoStore((s) => s.info);
  const loadError = useSettingsStore((s) => s.loadError);
  const saveError = useSettingsStore((s) => s.saveError);
  const clearSaveError = useSettingsStore((s) => s.clearSaveError);
  useBackendConnections();

  const Page = PAGES.find((p) => p.id === page)!.component;

  return (
    <div className="flex h-full text-zinc-200">
      <nav className="flex w-44 shrink-0 flex-col gap-0.5 border-r border-zinc-800 bg-zinc-900/40 p-2">
        <div className="mb-3 px-3 pt-2 text-lg font-semibold tracking-tight text-zinc-100">Lanes</div>
        {PAGES.map((p) => (
          <button
            key={p.id}
            onClick={() => setPage(p.id)}
            className={cx(
              "flex items-center gap-2.5 rounded-md px-3 py-1.5 text-left text-sm",
              page === p.id ? "bg-zinc-800 text-zinc-100" : "text-zinc-400 hover:bg-zinc-800/50 hover:text-zinc-200",
            )}
          >
            <p.icon size={15} />
            {p.label}
          </button>
        ))}
        <div className="mt-auto flex flex-col gap-3 pb-1">
          <CaptureIndicator />
          {info && (
            <div className="flex items-center justify-between px-3 text-xs text-zinc-500">
              <span>v{info.version}</span>
              <span
                title={
                  info.elevated
                    ? "Running as administrator: hotkeys and audio work with elevated games."
                    : "Running as a standard user. Games that run as administrator cannot be reached."
                }
                className={cx(
                  "rounded px-1.5 py-0.5 font-medium",
                  info.elevated ? "bg-amber-500/15 text-amber-400" : "bg-zinc-800 text-zinc-400",
                )}
              >
                {info.elevated ? "Admin" : "Standard"}
              </span>
            </div>
          )}
        </div>
      </nav>
      <main className="@container min-w-0 flex-1 overflow-y-auto">
        {(loadError || saveError) && (
          <div className="sticky top-0 z-20 flex items-center gap-3 border-b border-red-900/60 bg-red-950/90 px-6 py-2 text-xs text-red-200 backdrop-blur">
            <span className="min-w-0 flex-1 break-words select-text">
              {loadError ? `Could not load settings: ${loadError}` : `Settings were not saved: ${saveError}`}
            </span>
            {saveError && (
              <button onClick={clearSaveError} className="shrink-0 text-red-300 hover:text-red-100">
                Dismiss
              </button>
            )}
          </div>
        )}
        <Page />
      </main>
      <Toasts />
    </div>
  );
}

