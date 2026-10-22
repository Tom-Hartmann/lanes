import { convertFileSrc } from "@tauri-apps/api/core";
import { useEffect, useMemo, useRef, useState, type RefObject } from "react";
import { Button, DbSlider, Spinner, ToggleChip } from "../../components/ui";
import { api, errorMessage, type ClipInfo, type MixSpec } from "../../lib/ipc";

// The <video> only plays a file's first audio track, so the viewer plays
// every lane as its own (cached, extracted) audio file through a WebAudio gain
// node, kept in sync with the video. While the mix is ready the viewer mutes
// the video itself (it must stay out of WebAudio: a suspended AudioContext
// would stall the video's clock) and passes its volume in as `master`.

export interface LaneMix {
  gainDb: number;
  muted: boolean;
  solo: boolean;
}

/** What copies use: included tracks and their gains (file track indexes). */
export type MixChoice = MixSpec;

/** Title of the all-lanes track Lanes writes first (core `MIX_TRACK_TITLE`). */
export const MIX_TRACK_TITLE = "Mix (all lanes)";

const MIN_DB = -60;
const dbToGain = (db: number) => (db <= MIN_DB ? 0 : 10 ** (db / 20));
const audible = (lanes: LaneMix[], i: number) =>
  lanes.some((l) => l.solo) ? lanes[i].solo && !lanes[i].muted : !lanes[i].muted && lanes[i].gainDb > MIN_DB;

/** Mixes saved in this session, newer than the clip list the viewer was opened from. */
const savedMixes = new Map<string, LaneMix[] | null>();

function initialMix(clip: ClipInfo, count: number): LaneMix[] {
  const saved = savedMixes.has(clip.path) ? savedMixes.get(clip.path) : clip.mix;
  const fresh = () => ({ gainDb: 0, muted: false, solo: false });
  if (!Array.isArray(saved)) return Array.from({ length: count }, fresh);
  return Array.from({ length: count }, (_, i) => {
    const x = saved[i] as Partial<LaneMix> | undefined;
    return {
      gainDb: typeof x?.gainDb === "number" ? x.gainDb : 0,
      muted: x?.muted === true,
      solo: x?.solo === true,
    };
  });
}

/** Compares mixes; "every lane at 0 dB" and null are the same (the recorded mix). */
function mixKey(m: unknown): string {
  if (!Array.isArray(m)) return "default";
  const lanes = m.map((x) => {
    const l = x as Partial<LaneMix> | null;
    return { gainDb: typeof l?.gainDb === "number" ? l.gainDb : 0, muted: l?.muted === true, solo: l?.solo === true };
  });
  return lanes.some((l) => l.gainDb !== 0 || l.muted || l.solo) ? JSON.stringify(lanes) : "default";
}

/** What is stored: the lanes, or null for the default (all at 0 dB). */
const mixPayload = (lanes: LaneMix[]) => (lanes.some((l) => l.gainDb !== 0 || l.muted || l.solo) ? lanes : null);

let ctx: AudioContext | null = null;
const audioContext = () => (ctx ??= new AudioContext());
/** Browsers start an AudioContext suspended until a user gesture; call from one. */
export const resumeAudio = () => void ctx?.resume();
// createMediaElementSource works once per element.
const sources = new WeakMap<HTMLMediaElement, MediaElementAudioSourceNode>();
function sourceFor(el: HTMLMediaElement) {
  let s = sources.get(el);
  if (!s) {
    s = audioContext().createMediaElementSource(el);
    sources.set(el, s);
  }
  return s;
}

export function useLaneMixer(video: RefObject<HTMLVideoElement | null>, src: string | null, clip: ClipInfo, master: number) {
  const path = clip.path;
  /** Lane titles (the mixed track, if any, is not a lane). */
  const [titles, setTitles] = useState<string[] | null>(null);
  /** 1 when track 0 is the mixed track: lane i is track i + skip. */
  const [skip, setSkip] = useState(0);
  const [firstTrack, setFirstTrack] = useState<string | null>(null);
  const [lanes, setLanes] = useState<LaneMix[]>([]);
  const [ready, setReady] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const gains = useRef<GainNode[]>([]);
  const loaded = useRef(false);
  /** Last mix written for this file, so opening a clip does not rewrite it. */
  const lastSaved = useRef("");

  useEffect(() => {
    let alive = true;
    loaded.current = false;
    setTitles(null);
    setError(null);
    api.clipTracks(path).then(
      (t) => {
        if (!alive) return;
        const sk = t.length > 1 && t[0] === MIX_TRACK_TITLE ? 1 : 0;
        const initial = initialMix(clip, t.length - sk);
        lastSaved.current = JSON.stringify(mixPayload(initial));
        setSkip(sk);
        setFirstTrack(t[0] ?? null);
        setTitles(t.slice(sk));
        setLanes(initial);
        loaded.current = true;
      },
      () => alive && setTitles([]),
    );
    return () => {
      alive = false;
    };
    // Only reload for another file (clip.mix is read once, when it opens).
  }, [path]);

  // Remember the mix with the clip (debounced), so it is there next time.
  useEffect(() => {
    if (!loaded.current) return;
    const payload = mixPayload(lanes);
    const json = JSON.stringify(payload);
    if (json === lastSaved.current) return;
    const t = window.setTimeout(() => {
      lastSaved.current = json;
      savedMixes.set(path, payload);
      api.setClipMix(path, payload).catch(() => {});
    }, 400);
    return () => window.clearTimeout(t);
  }, [lanes, path]);

  const trackCount = titles?.length ?? 0;
  useEffect(() => {
    const v = video.current;
    if (!v || !src || trackCount === 0) return;
    let alive = true;
    const audios: HTMLAudioElement[] = [];
    const cleanup: (() => void)[] = [];

    api.clipAudioPreview(path).then(
      (files) => {
        if (!alive) return;
        const ac = audioContext();
        gains.current = files.slice(skip).map((f) => {
          const a = new Audio();
          a.crossOrigin = "anonymous";
          a.preload = "auto";
          a.preservesPitch = true;
          a.src = convertFileSrc(f);
          audios.push(a);
          const g = ac.createGain();
          sourceFor(a).connect(g).connect(ac.destination);
          return g;
        });

        const sync = (force: boolean) => {
          for (const a of audios) {
            a.playbackRate = v.playbackRate;
            // Re-seeking audio glitches, so only correct real drift.
            if (force || Math.abs(a.currentTime - v.currentTime) > 0.15) a.currentTime = v.currentTime;
          }
        };
        const play = () => {
          void ac.resume();
          sync(true);
          for (const a of audios) void a.play().catch(() => {});
        };
        const pause = () => {
          for (const a of audios) a.pause();
          sync(true);
        };
        const handlers: [string, () => void][] = [
          ["play", play],
          ["playing", play],
          ["pause", pause],
          ["waiting", pause],
          ["seeked", () => sync(true)],
          ["ratechange", () => sync(false)],
          ["timeupdate", () => !v.paused && sync(false)],
        ];
        for (const [ev, fn] of handlers) {
          v.addEventListener(ev, fn);
          cleanup.push(() => v.removeEventListener(ev, fn));
        }
        if (!v.paused) play();
        setReady(true);
      },
      (err: unknown) => alive && setError(errorMessage(err)),
    );

    return () => {
      alive = false;
      for (const c of cleanup) c();
      for (const a of audios) {
        a.pause();
        a.removeAttribute("src");
        a.load();
      }
      gains.current = [];
      setReady(false);
    };
  }, [video, src, path, trackCount, skip]);

  // Apply the lane gains (and the master).
  useEffect(() => {
    gains.current.forEach((g, i) => {
      if (lanes[i]) g.gain.value = audible(lanes, i) ? dbToGain(lanes[i].gainDb) * master : 0;
    });
  }, [lanes, ready, master]);

  const choice: MixChoice = useMemo(() => {
    const on = lanes.map((_, i) => audible(lanes, i));
    // Indexes are file tracks: shift past the mixed track.
    return {
      tracks: on.every(Boolean) ? null : on.flatMap((x, i) => (x ? [i + skip] : [])),
      gainsDb: [...Array<number>(skip).fill(0), ...lanes.map((l) => l.gainDb)],
    };
  }, [lanes, skip]);

  const payload = useMemo(() => mixPayload(lanes), [lanes]);
  // What a saved copy must render into its mixed track: what you hear when it
  // differs from the file's mixed track, or the lanes of a file without one
  // (players and Discord would only play its first lane).
  const bakeReason: "mix" | "track" | null =
    titles === null || titles.length === 0
      ? null
      : mixKey(payload) !== mixKey(clip.mixBaked)
        ? "mix"
        : titles.length > 1 && skip === 0
          ? "track"
          : null;
  const bake: MixSpec | null = bakeReason ? choice : null;

  const setLane = (i: number, p: Partial<LaneMix>) => setLanes((l) => l.map((x, j) => (j === i ? { ...x, ...p } : x)));
  const reset = () => setLanes((l) => l.map(() => ({ gainDb: 0, muted: false, solo: false })));
  const changed = lanes.some((l) => l.gainDb !== 0 || l.muted || l.solo);

  return { titles, lanes, setLane, reset, changed, ready, error, choice, payload, bake, bakeReason, hasMix: skip > 0, firstTrack };
}

export function LaneMixerPanel({ mixer }: { mixer: ReturnType<typeof useLaneMixer> }) {
  const { titles, lanes, setLane, reset, changed, ready, error } = mixer;
  if (titles !== null && titles.length === 0) return null;
  return (
    <div className="space-y-2 border-b border-zinc-800 px-4 py-3">
      <div className="flex items-center gap-2 text-sm text-zinc-300">
        Audio mix
        {!ready && !error && <Spinner />}
        <span className="text-xs text-zinc-500">
          {error
            ? `Lane preview unavailable: ${error}`
            : "Adjust each lane after the fact. \"Save copy\" and share copies use this mix."}
        </span>
        <Button size="sm" variant="ghost" className="ml-auto" disabled={!changed} onClick={reset}>
          Reset
        </Button>
      </div>
      <div className="grid gap-x-6 gap-y-1.5 sm:grid-cols-2">
        {(titles ?? []).map((t, i) =>
          lanes[i] ? (
            <div key={i} className="flex items-center gap-2">
              <span className="w-24 shrink-0 truncate text-sm text-zinc-300" title={t}>
                {t}
              </span>
              <ToggleChip active={lanes[i].muted} tone="red" title="Mute lane" onClick={() => setLane(i, { muted: !lanes[i].muted })}>
                M
              </ToggleChip>
              <ToggleChip active={lanes[i].solo} tone="amber" title="Solo: hear only soloed lanes" onClick={() => setLane(i, { solo: !lanes[i].solo })}>
                S
              </ToggleChip>
              <DbSlider value={lanes[i].gainDb} min={MIN_DB} max={12} className="min-w-0 flex-1" onChange={(gainDb) => setLane(i, { gainDb })} />
            </div>
          ) : null,
        )}
      </div>
    </div>
  );
}
