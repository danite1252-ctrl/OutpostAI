import { useCallback, useEffect, useState } from "react";
import { ApiError, systemApi, type CrashSummary, type Diagnostics, type UpdateStatus } from "../api";
import { Button, ErrorNote } from "./ui";

function message(err: unknown) {
  if (err instanceof ApiError) return err.message;
  if (typeof err === "string") return err;
  return err instanceof Error ? err.message : "Something went wrong.";
}

function when(secs: number) {
  return new Date(secs * 1000).toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" });
}

/**
 * Download the newer version, check its signature (against the key built into Outpost),
 * install it and restart. Only works in the installed app (not `npm run dev`).
 */
export async function installUpdate(onProgress: (text: string) => void) {
  const { check } = await import("@tauri-apps/plugin-updater");
  const { relaunch } = await import("@tauri-apps/plugin-process");
  onProgress("Looking for the update…");
  const update = await check();
  if (!update) throw new Error("No newer version was found (or this copy of Outpost was started with npm run dev).");
  let total = 0;
  let got = 0;
  await update.download((e) => {
    if (e.event === "Started") total = e.data.contentLength ?? 0;
    else if (e.event === "Progress") {
      got += e.data.chunkLength;
      onProgress(total ? `Downloading… ${Math.round((got / total) * 100)}%` : "Downloading…");
    }
  });
  // Downloaded and its signature checked: stop the engines, then hand over to the installer.
  onProgress("Installing…");
  const { invoke } = await import("@tauri-apps/api/core");
  await invoke("before_update").catch(() => undefined);
  await update.install();
  onProgress("Restarting…");
  await relaunch();
}

/** A slim bar at the top when a newer Outpost is out (owners only). */
export function UpdateBanner({ token, onOpenSettings }: { token: string; onOpenSettings: () => void }) {
  const [status, setStatus] = useState<UpdateStatus | null>(null);
  const [hidden, setHidden] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const load = () => systemApi.updates(token).then(setStatus, () => undefined);
    void load();
    const t = window.setInterval(load, 30 * 60 * 1000);
    return () => window.clearInterval(t);
  }, [token]);

  const update = status?.update;
  if (!update || hidden === update.version) return null;
  return (
    <div role="status" className="flex flex-wrap items-center gap-3 border-b border-line bg-ember/10 px-6 py-2 text-sm">
      <span className="font-medium">Outpost {update.version} is ready.</span>
      {busy ? (
        <span className="text-muted">{busy}</span>
      ) : (
        <>
          <button type="button" className="text-ember underline-offset-2 hover:underline" onClick={onOpenSettings}>
            What's new
          </button>
          <Button
            className="min-h-8 px-3"
            onClick={() => {
              setError(null);
              installUpdate(setBusy).catch((e) => {
                setBusy(null);
                setError(message(e));
              });
            }}
          >
            Update and restart
          </Button>
          <button type="button" className="text-muted hover:text-ink" onClick={() => setHidden(update.version)}>
            Later
          </button>
        </>
      )}
      {error && <span className="text-danger">{error}</span>}
    </div>
  );
}

/** Settings › Updates. */
export function UpdatesPanel({ token, canManage }: { token: string; canManage: boolean }) {
  const [status, setStatus] = useState<UpdateStatus | null>(null);
  const [checking, setChecking] = useState(false);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    systemApi.updates(token).then(setStatus, (e) => setError(message(e)));
  }, [token]);

  async function checkNow() {
    setChecking(true);
    setError(null);
    try {
      setStatus(await systemApi.checkUpdates(token));
    } catch (e) {
      setError(message(e));
    } finally {
      setChecking(false);
    }
  }

  async function toggle(on: boolean) {
    setError(null);
    try {
      setStatus(await systemApi.setUpdates(token, on));
    } catch (e) {
      setError(message(e));
    }
  }

  if (!status) return <ErrorNote>{error}</ErrorNote>;
  const u = status.update;
  return (
    <div className="flex flex-col gap-4">
      <h2 className="text-lg font-semibold">Updates</h2>
      <div className="flex flex-col gap-3 rounded-2xl border border-line bg-surface p-5">
        <p className="text-sm">
          This is Outpost <b>{status.current}</b>.{" "}
          {u ? (
            <>
              <b className="text-ember">Outpost {u.version}</b> is ready{u.date ? ` (released ${u.date.slice(0, 10)})` : ""}.
            </>
          ) : status.checked_at && !status.error ? (
            "It's up to date."
          ) : null}
        </p>
        {u?.notes && (
          <div className="max-h-56 overflow-auto rounded-xl border border-line bg-night p-3 text-sm whitespace-pre-wrap text-soft">
            {u.notes}
          </div>
        )}
        {status.error && <p className="text-sm text-muted">Last check: {status.error}</p>}
        <div className="flex flex-wrap items-center gap-3">
          {u && canManage && (
            <Button
              disabled={!!busy}
              onClick={() => {
                setError(null);
                installUpdate(setBusy).catch((e) => {
                  setBusy(null);
                  setError(message(e));
                });
              }}
            >
              {busy ?? "Update and restart"}
            </Button>
          )}
          {canManage && (
            <Button variant="ghost" disabled={checking} onClick={() => void checkNow()}>
              {checking ? "Checking…" : "Check now"}
            </Button>
          )}
          {status.checked_at && <span className="text-xs text-muted">Last checked {when(status.checked_at)}</span>}
        </div>
        {canManage && (
          <label className="flex items-start gap-3 text-sm">
            <input type="checkbox" className="mt-1" checked={status.enabled} onChange={(e) => void toggle(e.target.checked)} />
            <span>
              Check for updates
              <span className="block text-xs text-muted">
                Once a day Outpost asks github.com/{status.repo} whether there's a newer version. It's the only time Outpost
                goes on the internet by itself, and nothing about you or your PC is sent. Updates only install when you press
                Update, and only if they're signed with Outpost's key.
              </span>
            </span>
          </label>
        )}
        <ErrorNote>{error}</ErrorNote>
      </div>
    </div>
  );
}

/** A note at the top when Outpost crashed or didn't close normally (owners only). */
export function CrashNotice({ token, onOpenSettings }: { token: string; onOpenSettings: () => void }) {
  const [unseen, setUnseen] = useState(0);
  useEffect(() => {
    systemApi.diagnostics(token).then((d) => setUnseen(d.unseen), () => undefined);
  }, [token]);
  if (!unseen) return null;
  const dismiss = () => {
    setUnseen(0);
    void systemApi.markSeen(token).catch(() => undefined);
  };
  return (
    <div role="status" className="flex flex-wrap items-center gap-3 border-b border-line bg-raised px-6 py-2 text-sm">
      <span>
        Outpost had a problem last time. A crash report was saved on this PC (nothing was sent anywhere).
      </span>
      <button
        type="button"
        className="text-ember underline-offset-2 hover:underline"
        onClick={() => {
          dismiss();
          onOpenSettings();
        }}
      >
        View it
      </button>
      <button type="button" className="text-muted hover:text-ink" onClick={dismiss}>
        Dismiss
      </button>
    </div>
  );
}

/** Settings › Diagnostics: crash reports and recent logs, all kept on this PC. */
export function DiagnosticsPanel({ token }: { token: string }) {
  const [diag, setDiag] = useState<Diagnostics | null>(null);
  const [open, setOpen] = useState<{ title: string; text: string } | null>(null);
  const [copied, setCopied] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(() => {
    systemApi.diagnostics(token).then(setDiag, (e) => setError(message(e)));
  }, [token]);
  useEffect(() => {
    load();
    void systemApi.markSeen(token).catch(() => undefined);
  }, [load, token]);

  async function show(c: CrashSummary) {
    try {
      const r = await systemApi.crash(token, c.id);
      setOpen({ title: c.kind === "unclean" ? "Didn't close normally" : "Crash report", text: r.text });
    } catch (e) {
      setError(message(e));
    }
  }

  async function showLog() {
    try {
      const r = await systemApi.recentLog(token);
      setOpen({ title: "Recent log", text: r.text || "(nothing logged yet)" });
    } catch (e) {
      setError(message(e));
    }
  }

  async function remove(id?: string) {
    try {
      if (id) await systemApi.deleteCrash(token, id);
      else if (window.confirm("Delete every crash report?")) await systemApi.deleteAllCrashes(token);
      load();
    } catch (e) {
      setError(message(e));
    }
  }

  const copy = (text: string) =>
    void navigator.clipboard.writeText(text).then(() => {
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1500);
    });

  return (
    <div className="flex flex-col gap-4">
      <h2 className="text-lg font-semibold">Diagnostics</h2>
      <div className="flex flex-col gap-3 rounded-2xl border border-line bg-surface p-5">
        <p className="text-sm text-muted">
          If Outpost hits an error it can't recover from, or doesn't close normally, it saves a report here. Reports and logs
          stay on this PC; nothing is ever sent. To get help, copy a report and share it yourself.
        </p>
        {diag && diag.crashes.length === 0 && <p className="text-sm">No crash reports. 🎉</p>}
        {diag && diag.crashes.length > 0 && (
          <ul className="flex flex-col divide-y divide-line rounded-xl border border-line">
            {diag.crashes.map((c) => (
              <li key={c.id} className="flex flex-wrap items-center gap-3 px-3 py-2 text-sm">
                <span className="w-40 shrink-0 text-muted">{when(c.at)}</span>
                <span className="min-w-0 flex-1 truncate" title={c.summary}>
                  {c.kind === "unclean" ? "Didn't close normally" : c.summary}
                </span>
                <Button variant="ghost" className="min-h-8 px-3" onClick={() => void show(c)}>
                  View
                </Button>
                <Button variant="danger" className="min-h-8 px-3" onClick={() => void remove(c.id)}>
                  Delete
                </Button>
              </li>
            ))}
          </ul>
        )}
        <div className="flex flex-wrap gap-3">
          <Button variant="ghost" onClick={() => void showLog()}>
            Show recent log
          </Button>
          {diag && <Button variant="ghost" onClick={() => copy(diag.logs_dir)}>Copy log folder path</Button>}
          {diag && diag.crashes.length > 1 && (
            <Button variant="danger" onClick={() => void remove()}>
              Delete all reports
            </Button>
          )}
        </div>
        {copied && <p className="text-xs text-ok">Copied.</p>}
        <ErrorNote>{error}</ErrorNote>
      </div>
      {open && (
        <div role="dialog" aria-modal="true" aria-label={open.title} className="fixed inset-0 z-50 flex items-center justify-center bg-black/70 p-6">
          <div className="flex max-h-full w-full max-w-4xl flex-col gap-3 rounded-2xl border border-line bg-surface p-5">
            <div className="flex items-center gap-3">
              <h3 className="flex-1 font-semibold">{open.title}</h3>
              <Button variant="ghost" onClick={() => copy(open.text)}>
                {copied ? "Copied" : "Copy"}
              </Button>
              <Button variant="ghost" onClick={() => setOpen(null)}>
                Close
              </Button>
            </div>
            <pre className="min-h-0 flex-1 overflow-auto rounded-xl bg-night p-3 font-mono text-xs whitespace-pre-wrap text-soft">{open.text}</pre>
          </div>
        </div>
      )}
    </div>
  );
}
