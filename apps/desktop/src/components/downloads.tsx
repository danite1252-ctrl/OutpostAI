import { useCallback, useEffect, useRef, useState } from "react";
import { downloadsApi, type Download } from "../api";
import { Icon, icons } from "./ui";

/** Tell the Downloads button to look again (after starting a download). */
export function downloadsChanged() {
  window.dispatchEvent(new Event("outpost:downloads"));
}

const DISMISSED_KEY = "outpost.downloads.dismissed";

function loadDismissed(): Set<string> {
  try {
    return new Set(JSON.parse(localStorage.getItem(DISMISSED_KEY) ?? "[]") as string[]);
  } catch {
    return new Set();
  }
}

function saveDismissed(s: Set<string>) {
  try {
    localStorage.setItem(DISMISSED_KEY, JSON.stringify([...s].slice(-100)));
  } catch {
    // Not kept: the list just shows them again next time.
  }
}

/** A finished item is hidden by its id and finish time, so a later download of the same thing shows. */
const doneKey = (d: Download) => `${d.id}@${d.finished_at ?? ""}`;

export function formatBytes(n: number) {
  if (n >= 1e9) return `${(n / 1e9).toFixed(n >= 1e10 ? 0 : 1)} GB`;
  if (n >= 1e6) return `${Math.round(n / 1e6)} MB`;
  if (n >= 1e3) return `${Math.round(n / 1e3)} KB`;
  return `${n} B`;
}

export function formatSpeed(bps: number) {
  return `${formatBytes(bps)}/s`;
}

export function formatEta(secs: number) {
  if (secs < 60) return `${Math.max(1, Math.round(secs))} s left`;
  const mins = Math.round(secs / 60);
  if (mins < 60) return `${mins} min left`;
  const h = Math.floor(mins / 60);
  const m = mins % 60;
  return m ? `${h} h ${m} min left` : `${h} h left`;
}

/** One line under the bar, like a browser's downloads list. */
export function describe(d: Download) {
  if (d.error) return d.error;
  if (!d.running) return d.total_bytes ? `Done · ${formatBytes(d.total_bytes)}` : "Done";
  const parts: string[] = [];
  if (d.parts && d.parts > 1) parts.push(`File ${Math.max(1, d.part)} of ${d.parts}`);
  else if (!d.parts && d.part > 1) parts.push(`Part ${d.part}`);
  if (d.total_bytes) parts.push(`${formatBytes(d.done_bytes)} of ${formatBytes(d.total_bytes)}`);
  else if (d.detail) parts.push(d.detail);
  if (d.speed_bps) parts.push(formatSpeed(d.speed_bps));
  if (d.eta_secs != null) parts.push(formatEta(d.eta_secs));
  return parts.join(" · ") || "Starting";
}

export function useDownloads(token: string, open: boolean) {
  const [items, setItems] = useState<Download[]>([]);
  const [dismissed, setDismissed] = useState<Set<string>>(loadDismissed);
  const running = items.filter((d) => d.running).length;

  const refresh = useCallback(
    () =>
      downloadsApi.list(token).then(
        (r) => setItems(r.items),
        () => undefined,
      ),
    [token],
  );

  useEffect(() => {
    void refresh();
    const t = window.setInterval(() => void refresh(), running > 0 || open ? 1000 : 8000);
    const now = () => void refresh();
    window.addEventListener("outpost:downloads", now);
    return () => {
      window.clearInterval(t);
      window.removeEventListener("outpost:downloads", now);
    };
  }, [refresh, running, open]);

  const visible = items.filter((d) => d.running || !dismissed.has(doneKey(d)));
  const clearFinished = () => {
    const next = new Set(dismissed);
    for (const d of items) if (!d.running) next.add(doneKey(d));
    saveDismissed(next);
    setDismissed(next);
  };
  return { items: visible, running, clearFinished };
}

/** The header's Downloads button: a ring while something downloads, and a panel listing them. */
export function DownloadsButton({ token }: { token: string }) {
  const [open, setOpen] = useState(false);
  const { items, running, clearFinished } = useDownloads(token, open);
  const box = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && setOpen(false);
    const onClick = (e: MouseEvent) => {
      if (box.current && !box.current.contains(e.target as Node)) setOpen(false);
    };
    window.addEventListener("keydown", onKey);
    window.addEventListener("mousedown", onClick);
    return () => {
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("mousedown", onClick);
    };
  }, [open]);

  const active = items.filter((d) => d.running);
  const total = active.reduce((n, d) => n + d.total_bytes, 0);
  const done = active.reduce((n, d) => n + d.done_bytes, 0);
  const progress = total > 0 ? done / total : null;
  const failed = items.some((d) => d.error);
  if (items.length === 0 && !open) return null;

  const r = 19;
  const c = 2 * Math.PI * r;
  return (
    <div ref={box} className="relative">
      <button
        type="button"
        onClick={() => setOpen((o) => !o)}
        aria-expanded={open}
        aria-label={running ? `Downloads: ${running} running` : "Downloads"}
        title="Downloads"
        className={`relative flex size-11 items-center justify-center rounded-xl border border-line bg-surface hover:bg-raised ${
          running ? "text-pick" : failed ? "text-danger" : "text-muted"
        }`}
      >
        {running > 0 && (
          <svg className={`absolute inset-0 ${progress == null ? "animate-spin" : "-rotate-90"}`} viewBox="0 0 44 44" aria-hidden="true">
            <circle cx="22" cy="22" r={r} fill="none" stroke="currentColor" strokeOpacity="0.2" strokeWidth="2.5" />
            <circle
              cx="22"
              cy="22"
              r={r}
              fill="none"
              stroke="currentColor"
              strokeWidth="2.5"
              strokeLinecap="round"
              strokeDasharray={c}
              strokeDashoffset={progress == null ? c * 0.75 : c * (1 - progress)}
              className="transition-all"
            />
          </svg>
        )}
        <Icon d={icons.download} size={20} />
        {running > 1 && (
          <span className="absolute -top-1 -right-1 flex size-5 items-center justify-center rounded-full bg-pick text-[11px] font-semibold text-on-pick">
            {running}
          </span>
        )}
      </button>

      {open && (
        <div
          role="dialog"
          aria-label="Downloads"
          className="absolute top-full right-0 z-30 mt-2 flex w-[min(26rem,calc(100vw-2rem))] flex-col rounded-2xl border border-line bg-surface shadow-2xl shadow-black/50"
        >
          <div className="flex items-center gap-2 border-b border-line px-4 py-3">
            <h2 className="flex-1 font-semibold">Downloads</h2>
            {items.some((d) => !d.running) && (
              <button type="button" onClick={clearFinished} className="rounded-lg px-2 py-1 text-xs text-muted hover:bg-raised hover:text-ink">
                Clear finished
              </button>
            )}
            <button
              type="button"
              aria-label="Close"
              onClick={() => setOpen(false)}
              className="flex size-8 items-center justify-center rounded-lg text-muted hover:bg-raised hover:text-ink"
            >
              <Icon d={icons.close} size={16} />
            </button>
          </div>
          {items.length === 0 ? (
            <p className="px-4 py-6 text-center text-sm text-muted">Nothing downloading.</p>
          ) : (
            <ul className="flex max-h-[60vh] flex-col overflow-auto py-1">
              {items.map((d) => (
                <li key={d.id} className="flex flex-col gap-1.5 px-4 py-3" aria-live={d.running ? "polite" : undefined}>
                  <div className="flex items-baseline gap-2">
                    <span className="min-w-0 flex-1 truncate text-sm font-medium" title={d.name}>
                      {d.name}
                    </span>
                    <span className="shrink-0 text-xs text-muted">{d.kind === "model" ? "Chat model" : "Create"}</span>
                  </div>
                  {d.running && (
                    <div className="h-1.5 overflow-hidden rounded-full bg-raised">
                      {d.percent != null ? (
                        <div className="h-1.5 rounded-full bg-pick transition-all" style={{ width: `${d.percent}%` }} />
                      ) : (
                        <div className="h-1.5 w-1/3 animate-pulse rounded-full bg-pick/70" />
                      )}
                    </div>
                  )}
                  {d.running && d.kind === "workflow" && d.detail && (
                    <span className="truncate font-mono text-[11px] text-muted" title={d.detail}>
                      {d.detail}
                    </span>
                  )}
                  <span className={`text-xs ${d.error ? "text-danger" : d.running ? "text-soft" : "text-ok"}`}>{describe(d)}</span>
                </li>
              ))}
            </ul>
          )}
        </div>
      )}
    </div>
  );
}
