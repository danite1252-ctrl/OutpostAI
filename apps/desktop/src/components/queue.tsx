import { useCallback, useEffect, useRef, useState } from "react";
import { createApi, type Job, type LineEntry } from "../api";
import { formatEta } from "./downloads";
import { Avatar, Icon, icons } from "./ui";

/** Tell the Queue button to look again (after queueing or cancelling a job). */
export function jobsChanged() {
  window.dispatchEvent(new Event("outpost:jobs"));
}

type Progress = Job["progress"];

/** How far a running job is, 0 to 1, when it can be told. */
export function progressFraction(p: Progress): number | null {
  if (!p) return null;
  if (p.percent != null) return Math.min(1, Math.max(0, p.percent / 100));
  if (p.steps > 0) return Math.min(1, p.step / p.steps);
  return null;
}

function promptOf(j: Job | undefined) {
  if (!j) return "";
  try {
    return (JSON.parse(j.inputs).prompt as string) || "";
  } catch {
    return "";
  }
}

/** Finished jobs of yours stay in the list this long, so you can see they're done. */
const RECENT_SECS = 15 * 60;

/**
 * The header's Queue button, for everyone: how many jobs are in line for the graphics
 * card and how far the running one is. Your own jobs show what they are and can be
 * cancelled; other people's show only whose they are ("Sam is creating").
 */
export function QueueButton({ token, onOpenCreate, onOpenGallery }: { token: string; onOpenCreate: () => void; onOpenGallery: () => void }) {
  const [open, setOpen] = useState(false);
  const [line, setLine] = useState<LineEntry[]>([]);
  const [jobs, setJobs] = useState<Job[]>([]);
  const [names, setNames] = useState<Record<string, string>>({});
  const box = useRef<HTMLDivElement | null>(null);

  const refresh = useCallback(() => {
    void createApi.queue(token).then(setLine, () => undefined);
    void createApi.jobs(token).then(setJobs, () => undefined);
  }, [token]);

  const busy = line.length > 0;
  useEffect(() => {
    refresh();
    const t = window.setInterval(refresh, busy || open ? 1500 : 10000);
    window.addEventListener("outpost:jobs", refresh);
    return () => {
      window.clearInterval(t);
      window.removeEventListener("outpost:jobs", refresh);
    };
  }, [refresh, busy, open]);

  // Workflow names, once.
  useEffect(() => {
    createApi.packs(token).then(
      (ps) => setNames({ ...Object.fromEntries(ps.map((p) => [p.id, p.name])), train: "Training a character" }),
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
  const running = line.find((e) => e.status === "running") ?? null;
  const fraction = running ? progressFraction(running.progress) : null;
  const mineWaiting = line.filter((e) => e.mine).length;
  const jobOf = (e: LineEntry) => jobs.find((j) => j.id === e.job_id);

  async function cancel(e: LineEntry) {
    if (!e.job_id) return;
    try {
      await createApi.cancelJob(token, e.job_id);
    } finally {
      refresh();
    }
  }

  if (line.length === 0 && recent.length === 0 && !open) return null;

  const r = 19;
  const c = 2 * Math.PI * r;
  const failed = recent.some((j) => j.status === "failed");
  // Orange when any of it is yours, blue when it's only other people's.
  const tone = mineWaiting ? "text-ember" : busy ? "text-pick" : failed ? "text-danger" : "text-ok";
  const label = busy
    ? `Queue: ${line.length} job${line.length === 1 ? "" : "s"}${mineWaiting ? `, ${mineWaiting} yours` : ""}${
        fraction != null ? `, running one ${Math.round(fraction * 100)}% done` : ""
      }`
    : "Queue";

  return (
    <div ref={box} className="relative">
      <button
        type="button"
        onClick={() => setOpen((o) => !o)}
        aria-expanded={open}
        aria-label={label}
        title={running && !running.mine ? `${running.by_name} is creating` : "Queue"}
        className={`relative flex size-11 items-center justify-center rounded-xl border border-line bg-surface hover:bg-raised ${tone}`}
      >
        {busy && (
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
        {busy && (
          <span
            className={`absolute -top-1 -right-1 flex size-5 items-center justify-center rounded-full text-[11px] font-semibold ${
              mineWaiting ? "bg-ember text-on-ember" : "bg-pick text-on-pick"
            }`}
          >
            {line.length}
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
          {busy && (
            <p className="border-b border-line px-4 py-2 text-xs text-muted">
              One job runs at a time on the graphics card, in the order they were added.
            </p>
          )}
          {line.length === 0 && recent.length === 0 && <p className="px-4 py-6 text-center text-sm text-muted">Nothing waiting.</p>}
          <ol className="flex max-h-[60vh] flex-col overflow-auto py-1">
            {line.map((e, i) => {
              const f = progressFraction(e.progress);
              const p = e.progress;
              const isRunning = e.status === "running";
              const title = e.mine
                ? promptOf(jobOf(e)) || (e.pack_id && names[e.pack_id]) || "Your job"
                : isRunning
                  ? `${e.by_name} is creating`
                  : `${e.by_name}'s job`;
              const detail = isRunning
                ? [
                    e.mine ? (p?.phase ?? "Running") : null,
                    p && p.steps > 0 ? `step ${p.step} of ${p.steps}` : !e.mine ? (p?.phase ?? "Running") : null,
                    f != null ? `${Math.round(f * 100)}%` : null,
                    e.eta_secs != null ? formatEta(e.eta_secs) : null,
                  ]
                    .filter(Boolean)
                    .join(" · ")
                : i === (running ? 1 : 0)
                  ? "Next up"
                  : `${i - (running ? 1 : 0)} ahead`;
              return (
                <li key={e.job_id ?? `${e.by_name}-${i}`} className="flex flex-col gap-1.5 px-4 py-3" aria-live={isRunning ? "polite" : undefined}>
                  <div className="flex items-start gap-2.5">
                    <Avatar name={e.by_name} color={e.by_color} size={24} />
                    <span className="flex min-w-0 flex-1 flex-col">
                      <span className={`truncate text-sm ${e.mine ? "font-medium" : ""}`} title={title}>
                        {title}
                      </span>
                      <span className="text-xs text-muted">
                        {e.mine && e.pack_id ? `${names[e.pack_id] ?? e.pack_id} · ` : ""}
                        {detail}
                      </span>
                    </span>
                    {e.mine && (
                      <button
                        type="button"
                        onClick={() => void cancel(e)}
                        className="shrink-0 rounded-lg border border-line px-2 py-1 text-xs hover:bg-raised"
                      >
                        {isRunning ? "Stop" : "Cancel"}
                      </button>
                    )}
                  </div>
                  {isRunning && (
                    <div className="h-1.5 overflow-hidden rounded-full bg-raised">
                      {f != null ? (
                        <div
                          className={`h-1.5 rounded-full transition-all ${e.mine ? "bg-ember" : "bg-pick"}`}
                          style={{ width: `${f * 100}%` }}
                        />
                      ) : (
                        <div className={`h-1.5 w-1/3 animate-pulse rounded-full ${e.mine ? "bg-ember/70" : "bg-pick/70"}`} />
                      )}
                    </div>
                  )}
                </li>
              );
            })}
          </ol>
          {recent.length > 0 && (
            <ul className="flex flex-col border-t border-line py-1">
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
          )}
        </div>
      )}
    </div>
  );
}
