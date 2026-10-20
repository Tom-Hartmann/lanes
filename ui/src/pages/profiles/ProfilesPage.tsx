import { IconPlus } from "../../components/icons";
import { Button, Card, Field, PageHeader, Toggle } from "../../components/ui";
import type { GameDetectionSettings } from "../../lib/ipc";
import { DEFAULT_VIDEO, defaultAudio, newId, uniqueName } from "../../lib/settings";
import { useSettingsStore } from "../../store/settingsStore";
import { GamesSection } from "./GamesSection";
import { ProfileCard } from "./ProfileCard";

export function ProfilesPage() {
  const settings = useSettingsStore((s) => s.settings);
  const update = useSettingsStore((s) => s.update);
  if (!settings) return <div className="p-5 text-sm text-zinc-500">Loading settings…</div>;

  const det = settings.gameDetection;
  const setDet = (p: Partial<GameDetectionSettings>) =>
    update((s) => ({ ...s, gameDetection: { ...s.gameDetection, ...p } }));

  const create = () =>
    update((s) => ({
      ...s,
      profiles: [
        ...s.profiles,
        {
          id: newId("profile"),
          name: uniqueName("New profile", s.profiles.map((p) => p.name)),
          video: { ...DEFAULT_VIDEO, monitors: [] },
          audio: defaultAudio(),
          replayLengthSecs: null,
        },
      ],
    }));

  return (
    <div className="flex flex-col gap-4 p-5">
      <PageHeader
        title="Profiles"
        subtitle="A profile holds capture and audio lane settings. Map games to profiles to switch automatically."
        actions={
          <Button size="sm" onClick={create}>
            <IconPlus size={14} /> Profile
          </Button>
        }
      />
      <div className="flex flex-col gap-2">
        {settings.profiles.map((p) => (
          <ProfileCard key={p.id} profile={p} settings={settings} />
        ))}
      </div>
      <Card title="Game detection">
        <Field label="Detect games" hint="Watches the foreground window; costs almost nothing.">
          <Toggle checked={det.enabled} onChange={(enabled) => setDet({ enabled })} />
        </Field>
        <Field label="Treat unknown fullscreen programs as games" hint="Catches games that are not in the list below." disabled={!det.enabled}>
          <Toggle disabled={!det.enabled} checked={det.fullscreenHeuristic} onChange={(fullscreenHeuristic) => setDet({ fullscreenHeuristic })} />
        </Field>
        <Field label="Switch profile automatically" hint="Activates the game's profile when it starts." disabled={!det.enabled}>
          <Toggle disabled={!det.enabled} checked={det.autoSwitchProfile} onChange={(autoSwitchProfile) => setDet({ autoSwitchProfile })} />
        </Field>
      </Card>
      <GamesSection settings={settings} />
    </div>
  );
}
