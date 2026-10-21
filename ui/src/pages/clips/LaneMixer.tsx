import { convertFileSrc } from "@tauri-apps/api/core";
import { useEffect, useMemo, useRef, useState, type RefObject } from "react";
import { Button, DbSlider, Spinner, ToggleChip } from "../../components/ui";
import { api, errorMessage } from "../../lib/ipc";

// The <video> only plays a file's first audio track, so the viewer plays
// every lane as its own (cached, extracted) audio file through a WebAudio gain
// node, kept in sync with the video. The video's own sound is silenced; its
// volume and mute controls act as the master.

export interface LaneMix {
  gainDb: number;
  muted: boolean;
  solo: boolean;
}

/** What the share copy uses: included tracks and their gains. */
export interface MixChoice {
  tracks: number[] | null;
  gainsDb: number[];
}

const MIN_DB = -60;
const dbToGain = (db: number) => (db <= MIN_DB ? 0 : 10 ** (db / 20));
const audible = (lanes: LaneMix[], i: number) =>
  lanes.some((l) => l.solo) ? lanes[i].solo && !lanes[i].muted : !lanes[i].muted && lanes[i].gainDb > MIN_DB;

let ctx: AudioContext | null = null;
const audioContext = () => (ctx ??= new AudioContext());
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

export function useLaneMixer(video: RefObject<HTMLVideoElement | null>, src: string | null, path: string) {
  const [titles, setTitles] = useState<string[] | null>(null);
  const [lanes, setLanes] = useState<LaneMix[]>([]);
  const [ready, setReady] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const gains = useRef<GainNode[]>([]);
  const master = useRef(1);

  useEffect(() => {
    let alive = true;
    setTitles(null);
    setError(null);
    api.clipTracks(path).then(
      (t) => {
        if (!alive) return;
        setTitles(t);
        setLanes(t.map(() => ({ gainDb: 0, muted: false, solo: false })));
      },
      () => alive && setTitles([]),
    );
    return () => {
      alive = false;
    };
  }, [path]);

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
        // Silence the video's own audio (its first track) without touching
        // its volume/mute controls, which stay the master.
        sourceFor(v);
        gains.current = files.map((f) => {
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
            if (force || Math.abs(a.currentTime - v.currentTime) > 0.08) a.currentTime = v.currentTime;
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
        const volume = () => {
          master.current = v.muted ? 0 : v.volume;
          setLanes((l) => [...l]); // re-apply gains
        };
        const handlers: [string, () => void][] = [
          ["play", play],
          ["playing", play],
          ["pause", pause],
          ["waiting", pause],
          ["seeked", () => sync(true)],
          ["ratechange", () => sync(false)],
          ["timeupdate", () => !v.paused && sync(false)],
          ["volumechange", volume],
        ];
        for (const [ev, fn] of handlers) {
          v.addEventListener(ev, fn);
          cleanup.push(() => v.removeEventListener(ev, fn));
        }
        volume();
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
  }, [video, src, path, trackCount]);

  // Apply the lane gains (and the master).
  useEffect(() => {
    gains.current.forEach((g, i) => {
      if (lanes[i]) g.gain.value = audible(lanes, i) ? dbToGain(lanes[i].gainDb) * master.current : 0;
    });
  }, [lanes, ready]);

  const choice: MixChoice = useMemo(() => {
    const on = lanes.map((_, i) => audible(lanes, i));
    return {
      tracks: on.every(Boolean) ? null : on.flatMap((x, i) => (x ? [i] : [])),
      gainsDb: lanes.map((l) => l.gainDb),
    };
  }, [lanes]);

  const setLane = (i: number, p: Partial<LaneMix>) => setLanes((l) => l.map((x, j) => (j === i ? { ...x, ...p } : x)));
  const reset = () => setLanes((l) => l.map(() => ({ gainDb: 0, muted: false, solo: false })));
  const changed = lanes.some((l) => l.gainDb !== 0 || l.muted || l.solo);

  return { titles, lanes, setLane, reset, changed, ready, error, choice };
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
          {error ? `Lane preview unavailable: ${error}` : "Adjust each lane after the fact; the share copy uses this mix."}
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
