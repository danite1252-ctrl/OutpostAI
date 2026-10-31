import { useEffect, useState } from "react";
import { createApi, type Job } from "../api";
import { Lightbox, useAuthedUrl } from "./images";
import { ModelView } from "./model3d";
import { progressFraction } from "./queue";

/** A picture started from chat ("/draw ..."): its place in the queue, progress, then the result. */
export function JobCard({ token, jobId }: { token: string; jobId: string }) {
  const [job, setJob] = useState<Job | null>(null);
  const [missing, setMissing] = useState(false);
  const finished = job != null && (job.status === "done" || job.status === "failed" || job.status === "cancelled");
  useEffect(() => {
    let live = true;
    const load = () =>
      createApi.job(token, jobId).then(
        (j) => live && setJob(j),
        () => live && setMissing(true),
      );
    void load();
    if (finished) return;
    const t = window.setInterval(() => void load(), 1500);
    return () => {
      live = false;
      window.clearInterval(t);
    };
  }, [token, jobId, finished]);

  const box = "flex w-72 max-w-full flex-col gap-2 rounded-2xl border border-line bg-surface p-3";
  if (missing) return <div className={`${box} text-sm text-muted`}>This picture is no longer here.</div>;
  if (!job) return <div className={`${box} h-24 animate-pulse`} />;
  if (job.status === "done" && job.media.length > 0) {
    return (
      <div className="flex flex-wrap gap-2">
        {job.media.map((m) => (
          <Result key={m.id} token={token} id={m.id} kind={m.kind} alt={m.prompt ?? "Picture from chat"} />
        ))}
      </div>
    );
  }
  if (job.status === "failed") return <div className={`${box} text-sm text-danger`}>Couldn't draw it: {job.error ?? "unknown error"}</div>;
  if (job.status === "cancelled") return <div className={`${box} text-sm text-muted`}>Stopped.</div>;
  const f = progressFraction(job.progress);
  const p = job.progress;
  const text =
    job.status === "queued"
      ? job.position
        ? `Waiting: ${job.position} job${job.position === 1 ? "" : "s"} ahead`
        : "Starting…"
      : [p?.phase ?? "Drawing", p && p.steps > 0 ? `step ${p.step} of ${p.steps}` : null].filter(Boolean).join(" · ");
  return (
    <div className={box} aria-live="polite">
      <span className="text-sm text-muted">{text}</span>
      <div className="h-1.5 overflow-hidden rounded-full bg-raised">
        <div className="h-1.5 rounded-full bg-ember transition-all" style={{ width: `${(f ?? 0) * 100}%` }} />
      </div>
    </div>
  );
}

function Result({ token, id, kind, alt }: { token: string; id: string; kind: "image" | "video" | "model"; alt: string }) {
  const { src } = useAuthedUrl(token, `/api/media/${id}`);
  const [open, setOpen] = useState(false);
  if (!src) return <span className="block size-64 animate-pulse rounded-2xl bg-raised" />;
  if (kind === "model") return <ModelView src={src} className="h-72 w-72 max-w-full rounded-2xl border border-line" />;
  if (kind === "video") return <video src={src} controls className="max-h-96 max-w-full rounded-2xl" />;
  return (
    <>
      <button type="button" onClick={() => setOpen(true)} className="overflow-hidden rounded-2xl border border-line" aria-label="View larger">
        <img src={src} alt={alt} className="max-h-80 max-w-full object-contain" />
      </button>
      {open && <Lightbox src={src} alt={alt} onClose={() => setOpen(false)} />}
    </>
  );
}
