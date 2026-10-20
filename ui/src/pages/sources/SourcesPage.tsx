import { useState } from "react";
import { Callout, Card, Field, NumberInput, PageHeader, Segmented, Select, Toggle } from "../../components/ui";
import type { CaptureApi, CaptureMode, VideoSettings } from "../../lib/ipc";
import { withVideo } from "../../lib/settings";
import { useActiveProfile, useSettingsStore, type SaveMode } from "../../store/settingsStore";
import { useIsRecording } from "../../store/statusStore";
import { MonitorPicker } from "./MonitorPicker";
import { WindowPicker } from "./WindowPicker";

const FPS_PRESETS = [24, 30, 48, 60, 90, 120, 144, 165, 240];
const HEIGHT_PRESETS = [2160, 1440, 1080, 900, 720];

const MODE_HINT: Record<CaptureMode, string> = {
  followGame: "Captures the game Lanes detects (see Profiles → Games) and follows it when it changes.",
  monitors: "Captures one or more monitors, whatever is on them.",
  window: "Captures a single program's window, even when it is behind other windows.",
};

const API_HINT: Record<CaptureApi, string> = {
  auto: "Lanes picks the best method per target.",
  wgc: "Windows Graphics Capture: works with windows and monitors, yellow border may show on older Windows.",
  duplication: "Desktop Duplication: lowest overhead for full monitors, cannot capture single windows.",
};

export function SourcesPage() {
  const profile = useActiveProfile();
  const updateActiveProfile = useSettingsStore((s) => s.updateActiveProfile);
  const recording = useIsRecording();
  const [customFps, setCustomFps] = useState(false);
  const [customHeight, setCustomHeight] = useState(false);

  if (!profile) return <div className="p-5 text-sm text-zinc-500">Loading settings…</div>;
  const video = profile.video;
  const edit = (fn: (v: VideoSettings) => VideoSettings, mode: SaveMode = "now") =>
    updateActiveProfile(withVideo(fn), mode);

  const heightMode =
    customHeight || (video.outputHeight !== null && !HEIGHT_PRESETS.includes(video.outputHeight))
      ? "custom"
      : video.outputHeight === null
        ? "native"
        : String(video.outputHeight);
  const fpsMode = customFps || !FPS_PRESETS.includes(video.fps) ? "custom" : String(video.fps);

  return (
    <div className="flex flex-col gap-4 p-5">
      <PageHeader
        title="Sources & Monitors"
        subtitle={
          <>
            What Lanes captures for the profile <span className="text-zinc-300">{profile.name}</span>.
          </>
        }
      />
      {recording && <Callout tone="warn">A recording is running. Capture changes apply to the next recording.</Callout>}

      <Card title="Capture">
        <Field label="Capture" hint={MODE_HINT[video.mode]}>
          <Segmented
            value={video.mode}
            onChange={(mode) => edit((v) => ({ ...v, mode }))}
            options={[
              { value: "followGame", label: "Follow game" },
              { value: "monitors", label: "Monitors" },
              { value: "window", label: "Window" },
            ]}
          />
        </Field>
        <Field label="Capture method" hint={API_HINT[video.api]}>
          <Segmented
            value={video.api}
            onChange={(api) => edit((v) => ({ ...v, api }))}
            options={[
              { value: "auto", label: "Auto" },
              { value: "wgc", label: "WGC" },
              { value: "duplication", label: "Duplication", disabled: video.mode === "window" },
            ]}
          />
        </Field>
        <Field label="Capture cursor">
          <Toggle checked={video.captureCursor} onChange={(captureCursor) => edit((v) => ({ ...v, captureCursor }))} />
        </Field>
      </Card>

      {video.mode === "monitors" && <MonitorPicker video={video} onChange={(fn) => edit(fn)} />}
      {video.mode === "window" && (
        <WindowPicker exe={video.windowExe} onChange={(windowExe) => edit((v) => ({ ...v, windowExe }), "debounced")} />
      )}

      <Card title="Output">
        <Field label="Frame rate" hint="Higher frame rates cost encoder time and disk space.">
          <Select
            value={fpsMode}
            onChange={(v) => {
              setCustomFps(v === "custom");
              if (v !== "custom") edit((x) => ({ ...x, fps: Number(v) }));
            }}
            options={[
              ...FPS_PRESETS.map((f) => ({ value: String(f), label: `${f} fps` })),
              { value: "custom", label: "Custom" },
            ]}
          />
          {fpsMode === "custom" && (
            <NumberInput value={video.fps} min={1} max={360} suffix="fps" onChange={(fps) => edit((x) => ({ ...x, fps }), "debounced")} />
          )}
        </Field>
        <Field label="Resolution" hint="Downscaling happens on the GPU; width follows the aspect ratio.">
          <Select
            value={heightMode}
            onChange={(v) => {
              setCustomHeight(v === "custom");
              edit((x) => ({
                ...x,
                outputHeight: v === "native" ? null : v === "custom" ? (x.outputHeight ?? 1080) : Number(v),
              }));
            }}
            options={[
              { value: "native", label: "Native" },
              ...HEIGHT_PRESETS.map((h) => ({ value: String(h), label: `${h}p` })),
              { value: "custom", label: "Custom height" },
            ]}
          />
          {heightMode === "custom" && video.outputHeight !== null && (
            <NumberInput
              value={video.outputHeight}
              min={240}
              max={4320}
              step={2}
              suffix="px"
              onChange={(h) => edit((x) => ({ ...x, outputHeight: h - (h % 2) }), "debounced")}
            />
          )}
        </Field>
      </Card>
    </div>
  );
}
