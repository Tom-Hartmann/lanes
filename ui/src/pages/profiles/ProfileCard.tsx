import { useState } from "react";
import { Badge, Button, ConfirmButton, cx, Slider, Spinner, TextInput, Toggle } from "../../components/ui";
import { formatSecs } from "../../lib/format";
import { api, type Profile, type Settings } from "../../lib/ipc";
import { cloneProfile, uniqueName } from "../../lib/settings";
import { attempt } from "../../store/noticeStore";
import { useSettingsStore } from "../../store/settingsStore";
import { useStatusStore } from "../../store/statusStore";

/** Removes a profile and every reference to it. */
function withoutProfile(s: Settings, id: string): Settings {
  return {
    ...s,
    profiles: s.profiles.filter((p) => p.id !== id),
    games: s.games.map((g) => (g.profile === id ? { ...g, profile: null } : g)),
    general: s.general.autostartProfile === id ? { ...s.general, autostartProfile: null } : s.general,
  };
}

export function ProfileCard({ profile, settings }: { profile: Profile; settings: Settings }) {
  const update = useSettingsStore((s) => s.update);
  const updateProfile = useSettingsStore((s) => s.updateProfile);
  const flush = useSettingsStore((s) => s.flush);
  const load = useSettingsStore((s) => s.load);
  const refreshStatus = useStatusStore((s) => s.refresh);
  const [activating, setActivating] = useState(false);
  const active = settings.activeProfile === profile.id;
  const games = settings.games.filter((g) => g.profile === profile.id);

  const activate = async () => {
    setActivating(true);
    await flush();
    await attempt("Could not switch profile", () => api.activateProfile(profile.id));
    await Promise.allSettled([load(), refreshStatus()]);
    setActivating(false);
  };

  const duplicate = () =>
    update((s) => ({
      ...s,
      profiles: [...s.profiles, cloneProfile(profile, uniqueName(`${profile.name} copy`, s.profiles.map((p) => p.name)))],
    }));

  const override = profile.replayLengthSecs;

  return (
    <div className={cx("rounded-lg border p-3", active ? "border-sky-700/70 bg-sky-500/5" : "border-zinc-800 bg-zinc-900/60")}>
      <div className="flex flex-wrap items-center gap-2">
        <TextInput
          value={profile.name}
          onChange={(name) => updateProfile(profile.id, (p) => ({ ...p, name }), "debounced")}
          className="min-w-32 flex-1 border-transparent bg-transparent px-1 font-medium hover:border-zinc-700"
        />
        {active ? (
          <Badge tone="sky">Active</Badge>
        ) : (
          <Button size="sm" variant="primary" disabled={activating} onClick={() => void activate()}>
            {activating && <Spinner />} Activate
          </Button>
        )}
        <Button size="sm" variant="ghost" onClick={duplicate}>
          Duplicate
        </Button>
        <ConfirmButton
          disabled={active || settings.profiles.length <= 1}
          title={active ? "Activate another profile first" : undefined}
          confirmLabel="Delete?"
          onConfirm={() => update((s) => withoutProfile(s, profile.id))}
        >
          Delete
        </ConfirmButton>
      </div>
      <div className="mt-1 px-1 text-xs text-zinc-500">
        {profile.audio.lanes.length} lanes · {profile.audio.sources.length} sources · {profile.video.fps} fps
        {profile.video.outputHeight ? ` · ${profile.video.outputHeight}p` : " · native"}
        {games.length > 0 && ` · used by ${games.map((g) => g.name || g.exe).join(", ")}`}
      </div>
      <div className="mt-2 flex flex-wrap items-center gap-3 px-1">
        <span className="text-sm text-zinc-300">Own replay length</span>
        <Toggle
          checked={override !== null}
          onChange={(on) => updateProfile(profile.id, (p) => ({ ...p, replayLengthSecs: on ? settings.replay.lengthSecs : null }))}
        />
        {override !== null ? (
          <Slider
            value={override}
            min={15}
            max={600}
            step={15}
            format={formatSecs}
            onChange={(v) => updateProfile(profile.id, (p) => ({ ...p, replayLengthSecs: v }), "debounced")}
            className="w-56 max-w-full"
            readoutWidth="w-20"
          />
        ) : (
          <span className="text-xs text-zinc-500">Uses the global {formatSecs(settings.replay.lengthSecs)}</span>
        )}
      </div>
    </div>
  );
}
