import { useId, useState, type ButtonHTMLAttributes, type InputHTMLAttributes, type ReactNode } from "react";
import mark from "../assets/outpost-mark.png";

export function Mark({ size = 40, className = "" }: { size?: number; className?: string }) {
  return (
    <img
      src={mark}
      alt="Outpost"
      width={size}
      height={size}
      className={className}
      style={{ width: size, height: size, borderRadius: size * 0.22 }}
    />
  );
}

export function Wordmark({ className = "" }: { className?: string }) {
  return <span className={`font-display font-extrabold tracking-[0.04em] uppercase ${className}`}>Outpost</span>;
}

export function Avatar({ name, color, size = 40 }: { name: string; color: string; size?: number }) {
  return (
    <span
      aria-hidden="true"
      className="inline-flex shrink-0 items-center justify-center rounded-full font-semibold text-night"
      style={{ width: size, height: size, background: color, fontSize: size * 0.42 }}
    >
      {name.trim().charAt(0).toUpperCase() || "?"}
    </span>
  );
}

type ButtonProps = ButtonHTMLAttributes<HTMLButtonElement> & { variant?: "primary" | "ghost" | "danger" };

export function Button({ variant = "primary", className = "", ...props }: ButtonProps) {
  const styles = {
    primary: "bg-ember text-on-ember font-semibold hover:bg-ember-hover disabled:opacity-50",
    ghost: "border border-line text-ink hover:bg-raised disabled:opacity-50",
    danger: "border border-line text-danger hover:bg-raised disabled:opacity-50",
  }[variant];
  return (
    <button
      type="button"
      className={`inline-flex min-h-11 items-center justify-center gap-2 rounded-xl px-4 text-sm transition-colors ${styles} ${className}`}
      {...props}
    />
  );
}

export function Field({
  label,
  hint,
  ...props
}: InputHTMLAttributes<HTMLInputElement> & { label: string; hint?: ReactNode }) {
  return (
    <label className="flex flex-col gap-1.5 text-sm text-muted">
      {label}
      <input
        className="min-h-11 rounded-xl border border-line bg-surface px-3.5 text-base text-ink placeholder:text-muted/60 focus:border-ember focus:outline-none"
        {...props}
      />
      {hint && <span className="text-xs text-muted">{hint}</span>}
    </label>
  );
}

export function ErrorNote({ children }: { children: ReactNode }) {
  if (!children) return null;
  return (
    <p role="alert" className="rounded-lg bg-danger/10 px-3 py-2 text-sm text-danger">
      {children}
    </p>
  );
}

export function StatusDot({ on }: { on: boolean }) {
  return <span aria-hidden="true" className={`inline-block size-2 rounded-full ${on ? "bg-ok" : "bg-muted/50"}`} />;
}

/** Stroke icons, 24px grid. */
export const icons = {
  chat: "M4 5h16v11H9l-5 4z",
  create: "M4 4h16v16H4z M4 16l5-5 4 4 3-3 4 4 M15 8.5a1.5 1.5 0 1 0 0.01 0",
  train: "M12 3l9 5-9 5-9-5z M5 10.5V16c0 1.5 3 3 7 3s7-1.5 7-3v-5.5",
  models: "M12 3l8 4.5v9L12 21l-8-4.5v-9z M12 12l8-4.5 M12 12v9 M12 12L4 7.5",
  personas: "M12 12a4 4 0 1 0 0-8 4 4 0 0 0 0 8z M4 21c1-4 4.5-6 8-6s7 2 8 6",
  connections: "M9 3v5 M15 3v5 M6 8h12v3a6 6 0 0 1-12 0z M12 17v4",
  settings:
    "M12 15a3 3 0 1 0 0-6 3 3 0 0 0 0 6z M19 12h2 M3 12h2 M12 3v2 M12 19v2 M17 7l1.4-1.4 M5.6 18.4L7 17 M17 17l1.4 1.4 M5.6 5.6L7 7",
  lock: "M6 11h12v10H6z M8.5 11V7.5a3.5 3.5 0 0 1 7 0V11",
  back: "M15 5l-7 7 7 7",
  gallery: "M7 3h14v14 M3 7h14v14H3z M3 17l4-4 3 3 3-3 4 4",
  download: "M12 4v11 M7 10l5 5 5-5 M5 20h14",
  close: "M6 6l12 12 M18 6L6 18",
  queue: "M4 6h16 M4 12h16 M4 18h9 M17 16l3 2-3 2z",
  api: "M8 7l-5 5 5 5 M16 7l5 5-5 5 M14 4l-4 16",
} as const;

export function Icon({ d, size = 20 }: { d: string; size?: number }) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={1.8}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <path d={d} />
    </svg>
  );
}

/** An on/off switch. */
export function Toggle({
  checked,
  onChange,
  disabled,
  label,
  labelledBy,
}: {
  checked: boolean;
  onChange: (on: boolean) => void;
  disabled?: boolean;
  /** Read out when there's no visible label (`labelledBy`). */
  label?: string;
  labelledBy?: string;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={labelledBy ? undefined : label}
      aria-labelledby={labelledBy}
      disabled={disabled}
      onClick={() => onChange(!checked)}
      className={`relative inline-flex h-7 w-12 shrink-0 items-center rounded-full border transition-colors focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ember disabled:cursor-not-allowed disabled:opacity-50 ${
        checked ? "border-ember bg-ember" : "border-line bg-raised"
      }`}
    >
      <span
        aria-hidden="true"
        className={`inline-block size-5 rounded-full shadow transition-transform ${
          checked ? "translate-x-6 bg-ink" : "translate-x-1 bg-muted"
        }`}
      />
    </button>
  );
}

/** A setting that's on or off: what it is (and a hint) on the left, the switch on the right. */
export function ToggleRow({
  title,
  hint,
  checked,
  onChange,
  disabled,
}: {
  title: ReactNode;
  hint?: ReactNode;
  checked: boolean;
  onChange: (on: boolean) => void;
  disabled?: boolean;
}) {
  const id = useId();
  return (
    <div className="flex items-start gap-4">
      <span className="flex min-w-0 flex-1 flex-col">
        <span id={id} className="text-sm font-medium">
          {title}
        </span>
        {hint && <span className="text-xs text-muted">{hint}</span>}
      </span>
      <Toggle checked={checked} onChange={onChange} disabled={disabled} labelledBy={id} />
    </div>
  );
}

function rememberedOpen(id: string, fallback: boolean) {
  try {
    const v = localStorage.getItem(`outpost.settings.${id}`);
    return v === null ? fallback : v === "1";
  } catch {
    return fallback;
  }
}

/** A collapsible group of settings: a header with a one-line summary; opens in place.
 * Whether it's open is remembered on this PC. */
export function Section({
  id,
  title,
  summary,
  defaultOpen = false,
  children,
}: {
  id: string;
  title: string;
  summary?: ReactNode;
  defaultOpen?: boolean;
  children: ReactNode;
}) {
  const [open, setOpen] = useState(() => rememberedOpen(id, defaultOpen));
  const body = useId();
  const toggle = () => {
    setOpen((o) => {
      try {
        localStorage.setItem(`outpost.settings.${id}`, o ? "0" : "1");
      } catch {
        // Not remembered.
      }
      return !o;
    });
  };
  return (
    <section className="flex flex-col gap-4">
      <h2>
        <button
          type="button"
          aria-expanded={open}
          aria-controls={body}
          onClick={toggle}
          className="flex w-full items-center gap-4 rounded-2xl border border-line bg-surface px-5 py-4 text-left hover:bg-raised"
        >
          <span className="flex min-w-0 flex-1 flex-col">
            <span className="text-lg font-semibold">{title}</span>
            {summary && <span className="truncate text-sm text-muted">{summary}</span>}
          </span>
          <svg
            width={20}
            height={20}
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            strokeWidth={1.8}
            strokeLinecap="round"
            strokeLinejoin="round"
            aria-hidden="true"
            className={`shrink-0 text-muted transition-transform ${open ? "rotate-180" : ""}`}
          >
            <path d="M6 9l6 6 6-6" />
          </svg>
        </button>
      </h2>
      {open && (
        <div id={body} className="flex flex-col gap-4">
          {children}
        </div>
      )}
    </section>
  );
}
