import { useState } from "react";
import { Badge, Button, Callout, Card, Field, Spinner, Toggle } from "../../components/ui";
import { api, errorMessage } from "../../lib/ipc";
import { useAppInfoStore } from "../../store/appInfoStore";
import { useSettingsStore } from "../../store/settingsStore";

export function AdminSection() {
  const info = useAppInfoStore((s) => s.info);
  const setInfo = useAppInfoStore((s) => s.setInfo);
  const loadSettings = useSettingsStore((s) => s.load);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  if (!info) return null;
  const enabled = info.adminTaskInstalled;

  const toggle = async (on: boolean) => {
    setBusy(true);
    setError(null);
    try {
      setInfo(await api.setRunAsAdmin(on));
      await loadSettings();
    } catch (err) {
      setError(errorMessage(err));
    } finally {
      setBusy(false);
    }
  };

  const relaunch = async () => {
    setBusy(true);
    setError(null);
    try {
      await api.relaunchAsAdmin();
    } catch (err) {
      setError(errorMessage(err));
      setBusy(false);
    }
  };

  return (
    <Card
      title="Run as administrator"
      actions={
        <Badge tone={info.elevated ? "amber" : "neutral"}>{info.elevated ? "Running as Admin" : "Running as Standard user"}</Badge>
      }
    >
      <p className="text-xs leading-5 text-zinc-400">
        Windows keeps standard apps away from programs that run as administrator. Some games and anti-cheat launchers do.
        Then Lanes' hotkeys do not fire in that game, its window cannot be captured, and per-app audio of elevated
        processes stays silent. Admin mode starts Lanes elevated through a scheduled task: you confirm one UAC prompt
        when turning it on, and never again after that.
      </p>
      {!info.installedPerMachine ? (
        <div className="mt-3">
          <Callout tone="warn">
            Admin mode needs Lanes installed for all users (in Program Files), so that the elevated program cannot be
            modified without admin rights. Reinstall with the installer to enable it.
          </Callout>
        </div>
      ) : (
        <>
          <Field label="Start Lanes as administrator" hint={busy ? "Waiting for Windows…" : "Applies from the next start."}>
            {busy && <Spinner />}
            <Toggle checked={enabled} disabled={busy} onChange={(on) => void toggle(on)} />
          </Field>
          {enabled && !info.elevated && (
            <Field label="Restart now as administrator" hint="No UAC prompt: uses the scheduled task. Recording stops.">
              <Button size="sm" disabled={busy} onClick={() => void relaunch()}>
                Restart as administrator
              </Button>
            </Field>
          )}
        </>
      )}
      {error && (
        <div className="mt-2">
          <Callout tone="error">{/cancel/i.test(error) ? "The UAC prompt was cancelled. Nothing changed." : error}</Callout>
        </div>
      )}
    </Card>
  );
}
