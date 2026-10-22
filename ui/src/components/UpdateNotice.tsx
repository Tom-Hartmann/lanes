import { useEffect, useState } from "react";
import { api } from "../lib/ipc";
import { useSettingsStore } from "../store/settingsStore";

const LATEST = "https://api.github.com/repos/Tom-Hartmann/lanes/releases/latest";
const RECHECK_MS = 6 * 3600 * 1000;

/** "0.1.10" > "0.1.9"; ignores a leading "v" and anything after "-". */
export function isNewer(latest: string, current: string): boolean {
  const parts = (v: string) => v.replace(/^v/, "").split("-")[0].split(".").map((x) => Number(x) || 0);
  const [a, b] = [parts(latest), parts(current)];
  for (let i = 0; i < Math.max(a.length, b.length); i++) {
    if ((a[i] ?? 0) !== (b[i] ?? 0)) return (a[i] ?? 0) > (b[i] ?? 0);
  }
  return false;
}

/** Sidebar link shown when GitHub has a newer release (Settings → Check for updates). */
export function UpdateNotice({ current }: { current: string }) {
  const enabled = useSettingsStore((s) => s.settings?.general.checkUpdates ?? false);
  const [latest, setLatest] = useState<string | null>(null);

  useEffect(() => {
    if (!enabled) return;
    let alive = true;
    const check = () =>
      fetch(LATEST, { headers: { Accept: "application/vnd.github+json" } })
        .then((r) => (r.ok ? (r.json() as Promise<{ tag_name?: string }>) : null))
        .then((r) => alive && r?.tag_name && setLatest(r.tag_name))
        // Offline or rate-limited: just try again later.
        .catch(() => {});
    void check();
    const t = window.setInterval(() => void check(), RECHECK_MS);
    return () => {
      alive = false;
      window.clearInterval(t);
    };
  }, [enabled]);

  if (!enabled || !latest || !isNewer(latest, current)) return null;
  return (
    <button
      type="button"
      onClick={() => void api.openReleasesPage()}
      title="Open the release page to download the installer"
      className="mx-1 rounded-md border border-sky-600/50 bg-sky-500/10 px-2 py-1.5 text-left text-xs text-sky-200 hover:bg-sky-500/20"
    >
      Update available: {latest}
      <span className="block text-sky-400/80">Download from GitHub →</span>
    </button>
  );
}
