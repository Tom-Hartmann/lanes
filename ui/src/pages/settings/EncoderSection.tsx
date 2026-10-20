import { useEffect, useState } from "react";
import { Callout, Card, Field, NumberInput, Segmented, Select, Slider } from "../../components/ui";
import {
  api,
  type AudioCodec,
  type EncoderBackend,
  type EncoderInfo,
  type EncoderSettings,
  type RateControl,
  type VideoCodec,
} from "../../lib/ipc";
import { reportError } from "../../store/noticeStore";
import { useSettingsStore, type SaveMode } from "../../store/settingsStore";

const CODEC_LABEL: Record<VideoCodec, string> = { h264: "H.264", hevc: "HEVC (H.265)", av1: "AV1" };
const AUDIO_BITRATES = [96, 128, 160, 192, 256, 320];

export function EncoderSection({ enc }: { enc: EncoderSettings }) {
  const update = useSettingsStore((s) => s.update);
  const [encoders, setEncoders] = useState<EncoderInfo[] | null>(null);

  useEffect(() => {
    api.listEncoders().then(setEncoders, (err: unknown) => {
      reportError("Could not list encoders", err);
      setEncoders([]);
    });
  }, []);

  const set = (p: Partial<EncoderSettings>, mode: SaveMode = "now") =>
    update((s) => ({ ...s, encoder: { ...s.encoder, ...p } }), mode);

  const current = encoders?.find((e) => e.backend === enc.backend);
  const codecs: VideoCodec[] =
    enc.backend === "auto"
      ? [...new Set((encoders ?? []).flatMap((e) => e.codecs))]
      : (current?.codecs ?? []);
  const codecList = codecs.length > 0 ? codecs : (["h264"] as VideoCodec[]);
  const codecMissing = encoders !== null && !codecList.includes(enc.codec);
  const backendMissing = encoders !== null && enc.backend !== "auto" && !current;

  const changeBackend = (backend: EncoderBackend) => {
    const info = encoders?.find((e) => e.backend === backend);
    // Keep the codec if the new backend supports it, otherwise fall back to its first codec.
    const codec = info && !info.codecs.includes(enc.codec) ? (info.codecs[0] ?? enc.codec) : enc.codec;
    set({ backend, codec });
  };

  return (
    <Card title="Encoder">
      <Field label="Encoder" hint={current ? (current.hardware ? "Hardware encoder: almost no CPU or game impact." : "Software encoder: uses CPU time.") : "Lanes picks the best hardware encoder."}>
        <Select<EncoderBackend>
          value={enc.backend}
          onChange={changeBackend}
          options={[
            { value: "auto", label: "Automatic" },
            ...(encoders ?? []).map((e) => ({ value: e.backend, label: `${e.name}${e.hardware ? "" : " (CPU)"}` })),
            ...(backendMissing ? [{ value: enc.backend, label: `${enc.backend} (unavailable)` }] : []),
          ]}
        />
      </Field>
      <Field label="Video codec" hint="H.264 plays everywhere; HEVC and AV1 are smaller at the same quality.">
        <Select<VideoCodec>
          value={enc.codec}
          onChange={(codec) => set({ codec })}
          options={[
            ...codecList.map((c) => ({ value: c, label: CODEC_LABEL[c] })),
            ...(codecMissing ? [{ value: enc.codec, label: `${CODEC_LABEL[enc.codec]} (unsupported)` }] : []),
          ]}
        />
      </Field>
      {(codecMissing || backendMissing) && (
        <Callout tone="warn">The selected encoder or codec is not available on this PC. Lanes will fall back automatically.</Callout>
      )}
      <Field label="Rate control" hint={{ cbr: "Constant bitrate: predictable file size.", vbr: "Variable bitrate: better quality per byte.", cqp: "Constant quality: size varies with the scene." }[enc.rateControl]}>
        <Segmented<RateControl>
          value={enc.rateControl}
          onChange={(rateControl) => set({ rateControl })}
          options={[
            { value: "cbr", label: "CBR" },
            { value: "vbr", label: "VBR" },
            { value: "cqp", label: "CQP" },
          ]}
        />
      </Field>
      {enc.rateControl !== "cqp" && (
        <Field label={enc.rateControl === "vbr" ? "Target bitrate" : "Bitrate"} hint={`≈ ${((enc.bitrateKbps / 8) * 60 / 1024).toFixed(0)} MB per minute of video`}>
          <NumberInput value={enc.bitrateKbps} min={500} max={300000} step={500} suffix="kbps" onChange={(bitrateKbps) => set({ bitrateKbps }, "debounced")} className="w-24" />
        </Field>
      )}
      {enc.rateControl === "vbr" && (
        <Field label="Max bitrate">
          <NumberInput value={enc.maxBitrateKbps} min={enc.bitrateKbps} max={400000} step={500} suffix="kbps" onChange={(maxBitrateKbps) => set({ maxBitrateKbps }, "debounced")} className="w-24" />
        </Field>
      )}
      {enc.rateControl === "cqp" && (
        <Field label="Quality" hint="Lower is better and larger. 18–23 looks great for gameplay.">
          <Slider value={enc.quality} min={0} max={51} onChange={(quality) => set({ quality }, "debounced")} className="w-48" readoutWidth="w-8" />
        </Field>
      )}
      <Field label="Keyframe interval" hint="Shorter intervals make clip and trim cuts more precise.">
        <NumberInput value={enc.keyframeIntervalSecs} integer={false} min={0.5} max={10} step={0.5} suffix="s" onChange={(keyframeIntervalSecs) => set({ keyframeIntervalSecs }, "debounced")} />
      </Field>
      <Field label="Preset">
        <Segmented
          value={enc.preset}
          onChange={(preset) => set({ preset })}
          options={[
            { value: "performance", label: "Performance" },
            { value: "balanced", label: "Balanced" },
            { value: "quality", label: "Quality" },
          ]}
        />
      </Field>
      <Field label="Audio">
        <Select<AudioCodec>
          value={enc.audioCodec}
          onChange={(audioCodec) => set({ audioCodec })}
          options={[
            { value: "aac", label: "AAC" },
            { value: "opus", label: "Opus" },
          ]}
        />
        <Select
          value={String(enc.audioBitrateKbps)}
          onChange={(v) => set({ audioBitrateKbps: Number(v) })}
          options={[
            ...AUDIO_BITRATES.map((b) => ({ value: String(b), label: `${b} kbps` })),
            ...(AUDIO_BITRATES.includes(enc.audioBitrateKbps) ? [] : [{ value: String(enc.audioBitrateKbps), label: `${enc.audioBitrateKbps} kbps` }]),
          ]}
        />
      </Field>
      {enc.audioCodec === "opus" && <Callout>Opus in MP4 is not supported by every editor; AAC is the safest choice.</Callout>}
    </Card>
  );
}
