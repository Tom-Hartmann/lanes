// Small reusable form controls in the app's zinc/sky "pro tool" style.

import { useEffect, useRef, useState, type ButtonHTMLAttributes, type ReactNode } from "react";

const cx = (...c: (string | false | null | undefined)[]) => c.filter(Boolean).join(" ");
export { cx };

// ------------------------------------------------------------------- layout

export function PageHeader({ title, subtitle, actions }: { title: string; subtitle?: ReactNode; actions?: ReactNode }) {
  return (
    <header className="flex flex-wrap items-start justify-between gap-3">
      <div className="min-w-0">
        <h1 className="text-lg font-semibold text-zinc-100">{title}</h1>
        {subtitle && <p className="mt-0.5 max-w-3xl text-sm text-zinc-500">{subtitle}</p>}
      </div>
      {actions && <div className="flex shrink-0 flex-wrap items-center gap-2">{actions}</div>}
    </header>
  );
}

export function Card({
  title,
  actions,
  children,
  className,
  bodyClassName,
}: {
  title?: ReactNode;
  actions?: ReactNode;
  children: ReactNode;
  className?: string;
  bodyClassName?: string;
}) {
  return (
    <section className={cx("rounded-lg border border-zinc-800 bg-zinc-900/60", className)}>
      {(title || actions) && (
        <header className="flex items-center justify-between gap-2 border-b border-zinc-800 px-4 py-2">
          <h2 className="truncate text-sm font-medium text-zinc-300">{title}</h2>
          {actions && <div className="flex shrink-0 items-center gap-2">{actions}</div>}
        </header>
      )}
      <div className={cx("p-4", bodyClassName)}>{children}</div>
    </section>
  );
}

/** A label/description on the left and a control on the right; wraps on narrow widths. */
export function Field({
  label,
  hint,
  children,
  disabled,
}: {
  label: ReactNode;
  hint?: ReactNode;
  children: ReactNode;
  disabled?: boolean;
}) {
  return (
    <div className={cx("flex flex-wrap items-center justify-between gap-x-4 gap-y-1.5 py-2", disabled && "opacity-50")}>
      <div className="min-w-40 flex-1">
        <div className="text-sm text-zinc-200">{label}</div>
        {hint && <div className="text-xs text-zinc-500">{hint}</div>}
      </div>
      <div className="flex shrink-0 items-center gap-2">{children}</div>
    </div>
  );
}

export function Callout({ tone = "info", children }: { tone?: "info" | "warn" | "error"; children: ReactNode }) {
  const style = {
    info: "border-sky-900/60 bg-sky-950/30 text-sky-200/90",
    warn: "border-amber-900/60 bg-amber-950/30 text-amber-200/90",
    error: "border-red-900/60 bg-red-950/30 text-red-200/90",
  }[tone];
  return <div className={cx("rounded-md border px-3 py-2 text-xs leading-5", style)}>{children}</div>;
}

export function Badge({
  tone = "neutral",
  children,
  title,
}: {
  tone?: "neutral" | "sky" | "green" | "amber" | "red";
  children: ReactNode;
  title?: string;
}) {
  const style = {
    neutral: "bg-zinc-800 text-zinc-400",
    sky: "bg-sky-500/15 text-sky-300",
    green: "bg-emerald-500/15 text-emerald-400",
    amber: "bg-amber-500/15 text-amber-400",
    red: "bg-red-500/15 text-red-400",
  }[tone];
  return (
    <span title={title} className={cx("inline-flex items-center rounded px-1.5 py-0.5 text-[11px] font-medium", style)}>
      {children}
    </span>
  );
}

// ------------------------------------------------------------------ buttons

type ButtonVariant = "default" | "primary" | "danger" | "ghost" | "record";

const BUTTON_STYLE: Record<ButtonVariant, string> = {
  default: "bg-zinc-800 text-zinc-200 hover:bg-zinc-700",
  primary: "bg-sky-600 text-white hover:bg-sky-500",
  danger: "bg-red-600/80 text-white hover:bg-red-500",
  ghost: "text-zinc-400 hover:bg-zinc-800 hover:text-zinc-200",
  record: "bg-red-600 text-white hover:bg-red-500",
};

export function Button({
  variant = "default",
  size = "md",
  className,
  ...rest
}: ButtonHTMLAttributes<HTMLButtonElement> & { variant?: ButtonVariant; size?: "sm" | "md" }) {
  return (
    <button
      type="button"
      {...rest}
      className={cx(
        "inline-flex items-center justify-center gap-1.5 rounded-md font-medium whitespace-nowrap transition-colors disabled:pointer-events-none disabled:opacity-40",
        size === "sm" ? "px-2 py-1 text-xs" : "px-3 py-1.5 text-sm",
        BUTTON_STYLE[variant],
        className,
      )}
    />
  );
}

/** A button that needs a second click within 3 s to fire. */
export function ConfirmButton({
  onConfirm,
  children,
  confirmLabel = "Click again to confirm",
  size = "sm",
  disabled,
  title,
}: {
  onConfirm: () => void;
  children: ReactNode;
  confirmLabel?: string;
  size?: "sm" | "md";
  disabled?: boolean;
  title?: string;
}) {
  const [armed, setArmed] = useState(false);
  useEffect(() => {
    if (!armed) return;
    const t = setTimeout(() => setArmed(false), 3000);
    return () => clearTimeout(t);
  }, [armed]);
  return (
    <Button
      size={size}
      title={title}
      disabled={disabled}
      variant={armed ? "danger" : "ghost"}
      onClick={() => {
        if (armed) {
          setArmed(false);
          onConfirm();
        } else setArmed(true);
      }}
    >
      {armed ? confirmLabel : children}
    </Button>
  );
}

/** Small square toggle button, e.g. "M" for mute. */
export function ToggleChip({
  active,
  onClick,
  children,
  title,
  tone = "sky",
}: {
  active: boolean;
  onClick: () => void;
  children: ReactNode;
  title?: string;
  tone?: "sky" | "red" | "amber";
}) {
  const on = { sky: "bg-sky-600 text-white", red: "bg-red-600 text-white", amber: "bg-amber-500 text-zinc-950" }[tone];
  return (
    <button
      type="button"
      title={title}
      aria-pressed={active}
      onClick={onClick}
      className={cx(
        "h-6 min-w-6 rounded px-1.5 text-[11px] font-semibold transition-colors",
        active ? on : "bg-zinc-800 text-zinc-400 hover:bg-zinc-700 hover:text-zinc-200",
      )}
    >
      {children}
    </button>
  );
}

// ----------------------------------------------------------------- controls

export function Toggle({
  checked,
  onChange,
  disabled,
  label,
}: {
  checked: boolean;
  onChange: (v: boolean) => void;
  disabled?: boolean;
  label?: string;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={label}
      disabled={disabled}
      onClick={() => onChange(!checked)}
      className={cx(
        "relative inline-flex h-5 w-9 shrink-0 items-center rounded-full transition-colors disabled:opacity-40",
        checked ? "bg-sky-600" : "bg-zinc-700",
      )}
    >
      <span
        className={cx(
          "inline-block h-4 w-4 rounded-full bg-white shadow transition-transform",
          checked ? "translate-x-4.5" : "translate-x-0.5",
        )}
      />
    </button>
  );
}

export interface Option<T extends string> {
  value: T;
  label: string;
  disabled?: boolean;
}

export function Select<T extends string>({
  value,
  options,
  onChange,
  disabled,
  className,
}: {
  value: T;
  options: Option<T>[];
  onChange: (v: T) => void;
  disabled?: boolean;
  className?: string;
}) {
  return (
    <select
      value={value}
      disabled={disabled}
      onChange={(e) => onChange(e.target.value as T)}
      className={cx(
        "max-w-full rounded-md border border-zinc-700 bg-zinc-900 px-2 py-1 text-sm text-zinc-200 outline-none focus:border-sky-600 disabled:opacity-40",
        className,
      )}
    >
      {options.map((o) => (
        <option key={o.value} value={o.value} disabled={o.disabled}>
          {o.label}
        </option>
      ))}
    </select>
  );
}

export function Segmented<T extends string>({
  value,
  options,
  onChange,
  disabled,
}: {
  value: T;
  options: Option<T>[];
  onChange: (v: T) => void;
  disabled?: boolean;
}) {
  return (
    <div className={cx("inline-flex rounded-md border border-zinc-700 bg-zinc-900 p-0.5", disabled && "opacity-40")}>
      {options.map((o) => (
        <button
          key={o.value}
          type="button"
          disabled={disabled || o.disabled}
          onClick={() => onChange(o.value)}
          className={cx(
            "rounded px-2.5 py-0.5 text-xs font-medium whitespace-nowrap transition-colors disabled:cursor-not-allowed",
            value === o.value ? "bg-zinc-700 text-zinc-100" : "text-zinc-400 hover:text-zinc-200",
            o.disabled && "opacity-40",
          )}
        >
          {o.label}
        </button>
      ))}
    </div>
  );
}

export function TextInput({
  value,
  onChange,
  placeholder,
  className,
  disabled,
  mono,
  onBlur,
}: {
  value: string;
  onChange: (v: string) => void;
  placeholder?: string;
  className?: string;
  disabled?: boolean;
  mono?: boolean;
  onBlur?: () => void;
}) {
  return (
    <input
      type="text"
      value={value}
      disabled={disabled}
      placeholder={placeholder}
      spellCheck={false}
      onChange={(e) => onChange(e.target.value)}
      onBlur={onBlur}
      className={cx(
        "min-w-0 rounded-md border border-zinc-700 bg-zinc-900 px-2 py-1 text-sm text-zinc-200 outline-none select-text placeholder:text-zinc-600 focus:border-sky-600 disabled:opacity-40",
        mono && "font-mono",
        className,
      )}
    />
  );
}

/**
 * A number input that keeps its own text while typing and commits parsed,
 * clamped values. Commits on every valid keystroke (callers debounce saving).
 */
export function NumberInput({
  value,
  onChange,
  min,
  max,
  step = 1,
  suffix,
  integer = true,
  disabled,
  className,
}: {
  value: number;
  onChange: (v: number) => void;
  min?: number;
  max?: number;
  step?: number;
  suffix?: string;
  integer?: boolean;
  disabled?: boolean;
  className?: string;
}) {
  const [text, setText] = useState(String(value));
  const focused = useRef(false);
  useEffect(() => {
    if (!focused.current) setText(String(value));
  }, [value]);

  const parse = (t: string): number | null => {
    const n = Number(t.replace(",", "."));
    if (t.trim() === "" || !Number.isFinite(n)) return null;
    let v = integer ? Math.round(n) : n;
    if (min !== undefined) v = Math.max(min, v);
    if (max !== undefined) v = Math.min(max, v);
    return v;
  };

  return (
    <span className="inline-flex items-center gap-1">
      <input
        type="number"
        inputMode="decimal"
        value={text}
        min={min}
        max={max}
        step={step}
        disabled={disabled}
        onFocus={() => (focused.current = true)}
        onChange={(e) => {
          setText(e.target.value);
          const v = parse(e.target.value);
          if (v !== null && v === Number(e.target.value)) onChange(v);
        }}
        onBlur={() => {
          focused.current = false;
          const v = parse(text);
          if (v === null) setText(String(value));
          else {
            setText(String(v));
            if (v !== value) onChange(v);
          }
        }}
        className={cx(
          "w-20 rounded-md border border-zinc-700 bg-zinc-900 px-2 py-1 text-right text-sm text-zinc-200 tabular-nums outline-none select-text focus:border-sky-600 disabled:opacity-40",
          className,
        )}
      />
      {suffix && <span className="text-xs text-zinc-500">{suffix}</span>}
    </span>
  );
}

export function Slider({
  value,
  onChange,
  min,
  max,
  step = 1,
  format,
  disabled,
  className,
  readoutWidth = "w-16",
  onReset,
}: {
  value: number;
  onChange: (v: number) => void;
  min: number;
  max: number;
  step?: number;
  format?: (v: number) => string;
  disabled?: boolean;
  className?: string;
  readoutWidth?: string;
  /** Double-click resets to this callback's value. */
  onReset?: () => void;
}) {
  return (
    <span className={cx("inline-flex min-w-0 items-center gap-2", disabled && "opacity-40", className)}>
      <input
        type="range"
        min={min}
        max={max}
        step={step}
        value={value}
        disabled={disabled}
        onDoubleClick={onReset}
        onChange={(e) => onChange(Number(e.target.value))}
        className="lanes-range min-w-0 flex-1"
      />
      <span className={cx("shrink-0 text-right font-mono text-xs text-zinc-400 tabular-nums", readoutWidth)}>
        {format ? format(value) : value}
      </span>
    </span>
  );
}

/** Volume slider in dB (-60..+12), double-click resets to 0 dB. */
export function DbSlider({
  value,
  onChange,
  min = -60,
  max = 12,
  disabled,
  className,
}: {
  value: number;
  onChange: (v: number) => void;
  min?: number;
  max?: number;
  disabled?: boolean;
  className?: string;
}) {
  return (
    <Slider
      value={value}
      min={min}
      max={max}
      step={0.5}
      disabled={disabled}
      className={className}
      onChange={onChange}
      onReset={() => onChange(0)}
      format={(v) => (v <= -60 ? "-∞ dB" : `${v > 0 ? "+" : ""}${v.toFixed(1)} dB`)}
    />
  );
}

export function Spinner() {
  return <span className="inline-block h-3.5 w-3.5 animate-spin rounded-full border-2 border-zinc-600 border-t-sky-400" />;
}

export function EmptyState({ children }: { children: ReactNode }) {
  return (
    <div className="rounded-lg border border-dashed border-zinc-800 px-4 py-8 text-center text-sm text-zinc-500">
      {children}
    </div>
  );
}
