// Typed bridge to the Rust backend. This file is the contract: every command
// and event the UI uses is declared here and mirrors app/src/*.rs and
// core/src/config.rs (serde camelCase). Keep both sides in sync.

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

// ------------------------------------------------------------------ settings

export type Id = string;
export type EncoderBackend = "auto" | "nvenc" | "amf" | "qsv" | "x264";
export type VideoCodec = "h264" | "hevc" | "av1";
export type RateControl = "cbr" | "vbr" | "cqp";
export type EncoderPreset = "performance" | "balanced" | "quality";
export type AudioCodec = "aac" | "opus";

export interface EncoderSettings {
  backend: EncoderBackend;
  codec: VideoCodec;
  rateControl: RateControl;
  bitrateKbps: number;
  maxBitrateKbps: number;
  /** CQP quality, 0 (best) – 51. */
  quality: number;
  keyframeIntervalSecs: number;
  preset: EncoderPreset;
  audioCodec: AudioCodec;
  audioBitrateKbps: number;
}

export type CaptureMode = "followGame" | "monitors" | "window";
export type MultiMonitorLayout = "separateFiles" | "canvas";
export type CaptureApi = "auto" | "wgc" | "duplication";

export interface MonitorSelection {
  id: string;
  fpsCap: number | null;
  maxHeight: number | null;
}

export interface VideoSettings {
  mode: CaptureMode;
  api: CaptureApi;
  /** Empty = primary monitor. */
  monitors: MonitorSelection[];
  layout: MultiMonitorLayout;
  windowExe: string | null;
  fps: number;
  /** null = native resolution. */
  outputHeight: number | null;
  captureCursor: boolean;
}

export type SourceKind =
  /** `exclude`: leave out one app (process loopback in exclude mode while it runs). */
  | { type: "systemOutput"; device: string | null; exclude: string | null }
  | { type: "app"; exe: string }
  /** The detected game's own audio; follows whichever game is running. */
  | { type: "game" }
  | { type: "input"; device: string | null };

export interface AudioSource {
  id: Id;
  name: string;
  kind: SourceKind;
  volumeDb: number;
  muted: boolean;
  mono: boolean;
  noiseGate: { thresholdDb: number } | null;
  noiseSuppression: boolean;
  /** Lanes this source feeds (many-to-many). */
  lanes: Id[];
}

export interface Lane {
  id: Id;
  name: string;
  volumeDb: number;
  muted: boolean;
  mono: boolean;
}

export interface AudioLayout {
  lanes: Lane[];
  sources: AudioSource[];
}

export type ReplayStorage = "ram" | "disk";
export interface ReplaySettings {
  lengthSecs: number;
  ramCapMb: number;
  storage: ReplayStorage;
}

export type ClipQuality = "original" | "high" | "medium" | "low";
export interface ClipPreset {
  id: Id;
  name: string;
  lengthSecs: number;
  /** null = all lanes. */
  lanes: Id[] | null;
  quality: ClipQuality;
  folder: string | null;
  filenameTemplate: string | null;
}

export type HotkeyBackend = "auto" | "hook";
export type HotkeyMode = "registered" | "hook";

export interface HotkeySetting {
  /** "toggle_recording" | "toggle_pause" | "toggle_replay" | `save_clip:${presetId}` */
  action: string;
  combo: string | null;
  backend: HotkeyBackend;
}

export type OverlayCorner = "topLeft" | "topRight" | "bottomLeft" | "bottomRight";

export interface Profile {
  id: Id;
  name: string;
  video: VideoSettings;
  audio: AudioLayout;
  replayLengthSecs: number | null;
}

export interface GameEntry {
  exe: string;
  name: string;
  profile: Id | null;
  enabled: boolean;
}

export interface Settings {
  version: number;
  general: {
    startWithWindows: boolean;
    startMinimized: boolean;
    autostartReplay: boolean;
    autostartProfile: Id | null;
    autostartDelaySecs: number;
    runAsAdmin: boolean;
    clipSound: boolean;
    /** Output device id; null = Windows default output. */
    clipSoundDevice: string | null;
    /** 0–100 */
    clipSoundVolume: number;
    clipToast: boolean;
    /** Small on-screen notice that never takes focus. */
    clipOverlay: boolean;
    clipOverlayCorner: OverlayCorner;
    /** Placeholders: {preset} {duration} {game} */
    clipOverlayText: string;
  };
  output: {
    folder: string;
    /** Placeholders: {game} {profile} {date} {time} {preset} */
    filenameTemplate: string;
    remuxToMp4: boolean;
    diskGuardEnabled: boolean;
    diskGuardMinFreeGb: number;
    diskGuardAction: "warn" | "stop";
  };
  encoder: EncoderSettings;
  replay: ReplaySettings;
  clipPresets: ClipPreset[];
  hotkeys: HotkeySetting[];
  profiles: Profile[];
  activeProfile: Id;
  games: GameEntry[];
  gameDetection: {
    enabled: boolean;
    fullscreenHeuristic: boolean;
    autoSwitchProfile: boolean;
  };
  viewer: {
    /** Action id -> keys (KeyboardEvent.key); missing actions use the defaults. */
    keys: Record<string, string[]>;
    shortSeekSecs: number;
    longSeekSecs: number;
    /** Speed the slow-motion key switches to. */
    slowMotionSpeed: number;
  };
  share: {
    /** Discord channel webhook URL (Server settings → Integrations → Webhooks). */
    discordWebhook: string | null;
    /** Size limit for share copies in MB; 0 = no limit. */
    maxMb: number;
    /** Message posted with the clip. Placeholders: {game} {preset} {file} */
    discordMessage: string;
  };
}

// -------------------------------------------------------------- runtime info

export interface AppInfo {
  version: string;
  elevated: boolean;
  /** The elevated logon task exists. */
  adminTaskInstalled: boolean;
  osBuild: number;
  /** Per-application audio capture needs Windows 10 2004 (build 19041). */
  perAppAudioSupported: boolean;
  /** The app runs from a protected location (Program Files), required for admin mode. */
  installedPerMachine: boolean;
}

export type RecordingState =
  | { state: "idle" }
  | { state: "recording"; sinceMs: number; paused: boolean; files: string[] };

export interface ReplayState {
  active: boolean;
  bufferedSecs: number;
  bufferedBytes: number;
  capacitySecs: number;
}

export interface DetectedGame {
  exe: string;
  name: string;
  pid: number;
  profileId: Id | null;
  fullscreen: boolean;
}

export interface EngineStatus {
  recording: RecordingState;
  replay: ReplayState;
  activeProfile: Id;
  game: DetectedGame | null;
  /** Human-readable capture targets, e.g. ["DELL U2720Q", "cs2.exe"]. */
  targets: string[];
  /** e.g. "NVENC · H.264", null when idle. */
  encoder: string | null;
}

export interface BindingInfo {
  action: string;
  label: string;
  combo: string | null;
  backend: HotkeyBackend;
  mode: HotkeyMode | null;
  error: string | null;
}

export interface MonitorInfo {
  id: string;
  name: string;
  width: number;
  height: number;
  refreshHz: number;
  x: number;
  y: number;
  primary: boolean;
}

export interface WindowInfo {
  hwnd: number;
  title: string;
  exe: string;
  pid: number;
  fullscreen: boolean;
}

export interface AudioDevice {
  id: string;
  name: string;
  isDefault: boolean;
}

export interface AudioDevices {
  outputs: AudioDevice[];
  inputs: AudioDevice[];
}

export interface AudioApp {
  pid: number;
  exe: string;
  name: string;
  /** Currently has an active audio session. */
  active: boolean;
}

export interface EncoderInfo {
  backend: EncoderBackend;
  name: string;
  codecs: VideoCodec[];
  hardware: boolean;
}

export interface ShareResult {
  path: string;
  sizeBytes: number;
  reencoded: boolean;
}

export interface ClipInfo {
  path: string;
  fileName: string;
  sizeBytes: number;
  createdMs: number;
  durationSecs: number | null;
  kind: "clip" | "recording";
}

// -------------------------------------------------------------------- events

export interface LogEntry {
  seq: number;
  timeMs: number;
  level: "ERROR" | "WARN" | "INFO" | "DEBUG" | "TRACE";
  target: string;
  message: string;
}

export interface ActionEvent {
  action: string;
  trigger: "hotkey" | "tray" | "ui";
  atMs: number;
  detail: string | null;
}

/** ~20 Hz while the window is open. dBFS, -inf encoded as -100. */
export interface MeterFrame {
  lanes: { id: Id; peakDb: number; rmsDb: number }[];
  sources: { id: Id; peakDb: number }[];
}

/** 1 Hz while capturing and the window is open. */
export interface Stats {
  fps: number;
  targetFps: number;
  droppedFrames: number;
  duplicatedFrames: number;
  encodeMsAvg: number;
  /** Encode time / frame budget, 0..1+ */
  encoderLoad: number;
  encoderQueue: number;
  bufferSecs: number;
  bufferBytes: number;
  diskWriteBps: number;
  freeDiskBytes: number;
  cpuPercent: number;
  memoryBytes: number;
}

export interface ClipSaved {
  path: string;
  preset: string;
  durationSecs: number;
}

export interface EngineError {
  source: string;
  message: string;
  recoverable: boolean;
}

// ------------------------------------------------------------------ commands

export const api = {
  getAppInfo: () => invoke<AppInfo>("get_app_info"),
  getLogs: () => invoke<LogEntry[]>("get_logs"),
  /** Tries one clip-saved feedback with the current settings (sound, on-screen notice, Windows notification). */
  testClipFeedback: (kind: "sound" | "overlay" | "toast") => invoke<void>("test_clip_feedback", { kind }),

  getSettings: () => invoke<Settings>("get_settings"),
  /** Validates, persists and applies. Returns the stored (possibly repaired) settings. */
  updateSettings: (settings: Settings) => invoke<Settings>("update_settings", { settings }),
  activateProfile: (id: Id) => invoke<void>("activate_profile", { id }),

  listHotkeys: () => invoke<BindingInfo[]>("list_hotkeys"),
  setHotkey: (action: string, combo: string | null, backend: HotkeyBackend) =>
    invoke<BindingInfo>("set_hotkey", { action, combo, backend }),
  triggerAction: (action: string) => invoke<void>("trigger_action", { action }),

  getStatus: () => invoke<EngineStatus>("get_status"),
  startRecording: () => invoke<void>("start_recording"),
  stopRecording: () => invoke<void>("stop_recording"),
  togglePause: () => invoke<void>("toggle_pause"),
  startReplay: () => invoke<void>("start_replay"),
  stopReplay: () => invoke<void>("stop_replay"),
  saveClip: (presetId: Id) => invoke<void>("save_clip", { presetId }),

  listMonitors: () => invoke<MonitorInfo[]>("list_monitors"),
  /** PNG/BMP data URL, ~320 px wide. Only call while the page is visible. */
  getMonitorThumbnail: (id: string) => invoke<string>("get_monitor_thumbnail", { id }),
  listWindows: () => invoke<WindowInfo[]>("list_windows"),
  listAudioDevices: () => invoke<AudioDevices>("list_audio_devices"),
  listAudioApps: () => invoke<AudioApp[]>("list_audio_apps"),
  listEncoders: () => invoke<EncoderInfo[]>("list_encoders"),

  listClips: () => invoke<ClipInfo[]>("list_clips"),
  /** Image data URL of a frame near the start. */
  getClipThumbnail: (path: string) => invoke<string>("get_clip_thumbnail", { path }),
  /** Returns a path playable in the webview (remuxes MKV to a cached MP4 if needed). */
  prepareClipPlayback: (path: string) => invoke<string>("prepare_clip_playback", { path }),
  /** Moves to the recycle bin. */
  deleteClip: (path: string) => invoke<void>("delete_clip", { path }),
  revealClip: (path: string) => invoke<void>("reveal_clip", { path }),
  openOutputFolder: () => invoke<void>("open_output_folder"),
  remuxClip: (path: string) => invoke<string>("remux_clip", { path }),
  /** Lossless cut snapped to keyframes; returns the new file's path. */
  trimClip: (path: string, startSecs: number, endSecs: number) =>
    invoke<string>("trim_clip", { path, startSecs, endSecs }),

  /** Audio track names (lanes) of a clip. */
  clipTracks: (path: string) => invoke<string[]>("clip_tracks", { path }),
  /** Share copy with one mixed audio track (tracks: indices, null = all); maxMb 0 = no limit. */
  shareClip: (path: string, tracks: number[] | null, gainsDb: number[] | null, maxMb: number) =>
    invoke<ShareResult>("share_clip", { path, tracks, gainsDb, maxMb }),
  /** Each audio track as its own cached file, for the viewer's lane mixer. */
  clipAudioPreview: (path: string) => invoke<string[]>("clip_audio_preview", { path }),
  /** Clip on the clipboard as a file (Ctrl+V in Discord uploads it). */
  copyClipFile: (path: string) => invoke<void>("copy_clip_file", { path }),
  /** Native drag of the file; call while the mouse button is held. */
  dragClip: (path: string) => invoke<boolean>("drag_clip", { path }),

  /** Enables/disables admin mode. Enabling shows one UAC prompt. */
  setRunAsAdmin: (enabled: boolean) => invoke<AppInfo>("set_run_as_admin", { enabled }),
  /** Restarts elevated via the scheduled task (no UAC prompt). */
  relaunchAsAdmin: () => invoke<void>("relaunch_as_admin"),
};

const on =
  <T>(name: string) =>
  (cb: (payload: T) => void): Promise<UnlistenFn> =>
    listen<T>(name, (e) => cb(e.payload));

export const events = {
  onLog: on<LogEntry>("log"),
  onAction: on<ActionEvent>("action"),
  onStatus: on<EngineStatus>("status"),
  onMeters: on<MeterFrame>("meters"),
  onStats: on<Stats>("stats"),
  onClipSaved: on<ClipSaved>("clip_saved"),
  /** A recording finished (after the optional MP4 remux); payload = file paths. */
  onRecordingSaved: on<string[]>("recording_saved"),
  onError: on<EngineError>("engine_error"),
  onSettings: on<Settings>("settings"),
  onHotkeys: on<BindingInfo[]>("hotkeys"),
};

// ------------------------------------------------------------------ helpers
// Additive UI-side helpers. They do not change the wire contract.

/** The fixed hotkey actions; clip presets add `save_clip:<presetId>`. */
export const HOTKEY_ACTIONS = {
  toggleRecording: "toggle_recording",
  togglePause: "toggle_pause",
  toggleReplay: "toggle_replay",
} as const;

export const SAVE_CLIP_PREFIX = "save_clip:";

/** The hotkey action string that saves a clip with the given preset. */
export const saveClipAction = (presetId: Id) => `${SAVE_CLIP_PREFIX}${presetId}`;

/** The preset id of a `save_clip:<id>` action, or null for other actions. */
export const presetIdOfAction = (action: string): Id | null =>
  action.startsWith(SAVE_CLIP_PREFIX) ? action.slice(SAVE_CLIP_PREFIX.length) : null;

export type SourceKindType = SourceKind["type"];
export type GeneralSettings = Settings["general"];
export type OutputSettings = Settings["output"];
export type GameDetectionSettings = Settings["gameDetection"];

/** Turns a rejected invoke (string, Error or object) into a readable message. */
export function errorMessage(err: unknown): string {
  if (typeof err === "string") return err;
  if (err instanceof Error) return err.message;
  if (err && typeof err === "object" && "message" in err) return String((err as { message: unknown }).message);
  try {
    return JSON.stringify(err);
  } catch {
    return String(err);
  }
}
