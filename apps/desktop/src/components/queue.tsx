import { useCallback, useEffect, useRef, useState } from "react";
import { createApi, type Job } from "../api";
import { Icon, icons } from "./ui";

/** Tell the Queue button to look again (after queueing or cancelling a job). */
export function jobsChanged() {
  window.dispatchEvent(new Event("outpost:jobs"));
}

/** How far a running job is, 0 to 1, when it can be told. */
export function jobFraction(j: Job): number | null {
  const p = j.progress;
  if (!p) return null;
  if (p.percent != null) return Math.min(1, Math.max(0, p.percent / 100));
  if (p.steps > 0) return Math.min(1, p.step / p.steps);
  return null;
}

function promptOf(j: Job) {
  try {
    return (JSON.parse(j.inputs).prompt as string) || "";
  } catch {
    return "";
  }
}

/** Finished jobs stay in the list this long, so you can see they're done. */
const RECENT_SECS = 15 * 60;

/** The header's Queue button: how many of your jobs are waiting or running, and how far the running one is. */
export function QueueButton({ token, onOpenCreate, onOpenGallery }: { token: string; onOpenCreate: () => void; onOpenGallery: () => void }) {
  const [open, setOpen] = useState(false);
  const [jobs, setJobs] = useState<Job[]>([]);
  const [names, setNames] = useState<Record<string, string>>({});
  const box = useRef<HTMLDivElement | null>(null);

  const active = jobs.filter((j) => j.status === "queued" || j.status === "running");
  const refresh = useCallback(() => createApi.jobs(token).then(setJobs, () => undefined), [token]);

  useEffect(() => {
    void refresh();
    const t = window.setInterval(() => void refresh(), active.length > 0 || open ? 1500 : 10000);
    const now = () => void refresh();
    window.addEventListener("outpost:jobs", now);
    return () => {
      window.clearInterval(t);
      window.removeEventListener("outpost:jobs", now);
    };
  }, [refresh, active.length, open]);

  // Workflow names, once.
  useEffect(() => {
    createApi.packs(token).then(
      (ps) => setNames(Object.fromEntries(ps.map((p) => [p.id, p.name]))),
      () => undefined,
    );
  }, [token]);

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

  const now = Date.now() / 1000;
  const recent = jobs
    .filter((j) => (j.status === "done" || j.status === "failed") && (j.finished_at ?? 0) > now - RECENT_SECS)
    .slice(0, 4);
  const running = active.find((j) => j.status === "running") ?? null;
  const fraction = running ? jobFraction(running) : null;
  // Running first, then by place in line.
  const ordered = [...active].sort((a, b) =>
    a.status === b.status ? (a.position ?? 0) - (b.position ?? 0) : a.status === "running" ? -1 : 1,
  );

  async function cancel(j: Job) {
    try {
      await createApi.cancelJob(token, j.id);
    } finally {
      void refresh();
    }
  }

  if (active.length === 0 && recent.length === 0 && !open) return null;

  const r = 19;
  const c = 2 * Math.PI * r;
  const failed = recent.some((j) => j.status === "failed");
  return (
    <div ref={box} className="relative">
      <button
        type="button"
        onClick={() => setOpen((o) => !o)}
        aria-expanded={open}
        aria-label={
          active.length
            ? `Queue: ${active.length} job${active.length === 1 ? "" : "s"}${fraction != null ? `, ${Math.round(fraction * 100)}% done` : ""}`
            : "Queue"
        }
        title="Queue"
        className={`relative flex size-11 items-center justify-center rounded-xl border border-line bg-surface hover:bg-raised ${
          active.length ? "text-ember" : failed ? "text-danger" : "text-ok"
        }`}
      >
        {active.length > 0 && (
          <svg className={`absolute inset-0 ${running && fraction == null ? "animate-spin" : "-rotate-90"}`} viewBox="0 0 44 44" aria-hidden="true">
            <circle cx="22" cy="22" r={r} fill="none" stroke="currentColor" strokeOpacity="0.2" strokeWidth="2.5" />
            {running && (
              <circle
                cx="22"
                cy="22"
                r={r}
                fill="none"
                stroke="currentColor"
                strokeWidth="2.5"
                strokeLinecap="round"
                strokeDasharray={c}
                strokeDashoffset={fraction == null ? c * 0.75 : c * (1 - fraction)}
                className="transition-all"
              />
            )}
          </svg>
        )}
        <Icon d={icons.queue} size={19} />
        {active.length > 0 && (
          <span className="absolute -top-1 -right-1 flex size-5 items-center justify-center rounded-full bg-ember text-[11px] font-semibold text-on-ember">
            {active.length}
          </span>
        )}
      </button>

      {open && (
        <div
          role="dialog"
          aria-label="Queue"
          className="absolute top-full right-0 z-30 mt-2 flex w-[min(26rem,calc(100vw-2rem))] flex-col rounded-2xl border border-line bg-surface shadow-2xl shadow-black/50"
        >
          <div className="flex items-center gap-2 border-b border-line px-4 py-3">
            <h2 className="flex-1 font-semibold">Queue</h2>
            <button
              type="button"
              onClick={() => {
                setOpen(false);
                onOpenCreate();
              }}
              className="rounded-lg px-2 py-1 text-xs text-muted hover:bg-raised hover:text-ink"
            >
              Open Create
            </button>
            <button
              type="button"
              aria-label="Close"
              onClick={() => setOpen(false)}
              className="flex size-8 items-center justify-center rounded-lg text-muted hover:bg-raised hover:text-ink"
            >
              <Icon d={icons.close} size={16} />
            </button>
          </div>
          {ordered.length === 0 && recent.length === 0 && <p className="px-4 py-6 text-center text-sm text-muted">Nothing waiting.</p>}
          <ul className="flex max-h-[60vh] flex-col overflow-auto py-1">
            {ordered.map((j) => {
              const f = jobFraction(j);
              const p = j.progress;
              return (
                <li key={j.id} className="flex flex-col gap-1.5 px-4 py-3" aria-live={j.status === "running" ? "polite" : undefined}>
                  <div className="flex items-start gap-2">
                    <span className="flex min-w-0 flex-1 flex-col">
                      <span className="truncate text-sm font-medium" title={promptOf(j)}>
                        {promptOf(j) || names[j.pack_id] || j.pack_id}
                      </span>
                      <span className="text-xs text-muted">
                        {names[j.pack_id] ?? j.pack_id} ·{" "}
                        {j.status === "running"
                          ? `${p?.phase ?? "Running"}${p && p.steps > 0 ? ` · step ${p.step} of ${p.steps}` : ""}${f != null ? ` · ${Math.round(f * 100)}%` : ""}`
                          : j.position
                            ? `${j.position} ahead`
                            : "Next up"}
                      </span>
                    </span>
                    <button
                      type="button"
                      onClick={() => void cancel(j)}
                      className="shrink-0 rounded-lg border border-line px-2 py-1 text-xs hover:bg-raised"
                    >
                      {j.status === "running" ? "Stop" : "Cancel"}
                    </button>
                  </div>
                  {j.status === "running" && (
                    <div className="h-1.5 overflow-hidden rounded-full bg-raised">
                      {f != null ? (
                        <div className="h-1.5 rounded-full bg-ember transition-all" style={{ width: `${f * 100}%` }} />
                      ) : (
                        <div className="h-1.5 w-1/3 animate-pulse rounded-full bg-ember/70" />
                      )}
                    </div>
                  )}
                </li>
              );
            })}
            {recent.map((j) => (
              <li key={j.id} className="flex items-start gap-2 px-4 py-3">
                <span className="flex min-w-0 flex-1 flex-col">
                  <span className="truncate text-sm">{promptOf(j) || names[j.pack_id] || j.pack_id}</span>
                  <span className={`text-xs ${j.status === "failed" ? "text-danger" : "text-ok"}`}>
                    {j.status === "failed" ? (j.error ?? "Failed") : "Done"}
                  </span>
                </span>
                {j.status === "done" && (
                  <button
                    type="button"
                    onClick={() => {
                      setOpen(false);
                      onOpenGallery();
                    }}
                    className="shrink-0 rounded-lg border border-line px-2 py-1 text-xs hover:bg-raised"
                  >
                    View
                  </button>
                )}
              </li>
            ))}
          </ul>
        </div>
      )}
    </div>
  );
}
