import type { ButtonHTMLAttributes, InputHTMLAttributes, ReactNode } from "react";
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
