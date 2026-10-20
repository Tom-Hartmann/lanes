// Small, allocation-light formatting helpers shared by all pages.

const UNITS = ["B", "KB", "MB", "GB", "TB"];

export function formatBytes(bytes: number, digits = 1): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return "0 B";
  let v = bytes;
  let i = 0;
  while (v >= 1024 && i < UNITS.length - 1) {
    v /= 1024;
    i++;
  }
  return `${v.toFixed(i === 0 ? 0 : digits)} ${UNITS[i]}`;
}

export const formatRate = (bytesPerSec: number) => `${formatBytes(bytesPerSec)}/s`;

/** 75 -> "1:15", 3725 -> "1:02:05". */
export function formatDuration(totalSecs: number): string {
  const s = Math.max(0, Math.floor(totalSecs));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const sec = s % 60;
  const mm = h > 0 ? String(m).padStart(2, "0") : String(m);
  return `${h > 0 ? `${h}:` : ""}${mm}:${String(sec).padStart(2, "0")}`;
}

/** Seconds with one decimal: "12.3 s"; or "2 min 30 s" for longer spans when `long`. */
export function formatSecs(secs: number): string {
  if (secs < 60) return `${Math.round(secs)} s`;
  const m = Math.floor(secs / 60);
  const s = Math.round(secs % 60);
  return s === 0 ? `${m} min` : `${m} min ${s} s`;
}

export function formatDb(db: number): string {
  if (db <= -99) return "-∞ dB";
  const r = Math.round(db * 10) / 10;
  return `${r > 0 ? "+" : ""}${r.toFixed(1)} dB`;
}

export function formatDate(ms: number): string {
  return new Date(ms).toLocaleString(undefined, {
    year: "numeric",
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

const pad = (n: number) => String(n).padStart(2, "0");

/** Expands the filename template placeholders for a live preview. */
export function expandTemplate(
  template: string,
  vars: { game: string; profile: string; preset: string; now?: Date },
): string {
  const d = vars.now ?? new Date();
  const values: Record<string, string> = {
    game: vars.game,
    profile: vars.profile,
    preset: vars.preset,
    date: `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`,
    time: `${pad(d.getHours())}-${pad(d.getMinutes())}-${pad(d.getSeconds())}`,
  };
  return template.replace(/\{(\w+)\}/g, (m, key: string) => values[key] ?? m);
}

export const TEMPLATE_PLACEHOLDERS = ["game", "profile", "date", "time", "preset"] as const;

/** Placeholders in the template that the backend does not know. */
export function unknownPlaceholders(template: string): string[] {
  const known = new Set<string>(TEMPLATE_PLACEHOLDERS);
  const out: string[] = [];
  for (const m of template.matchAll(/\{(\w*)\}/g)) if (!known.has(m[1])) out.push(m[0]);
  return out;
}

export const clamp = (v: number, min: number, max: number) => Math.min(max, Math.max(min, v));
