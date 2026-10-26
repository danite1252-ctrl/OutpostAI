import { useCallback, useEffect, useState } from "react";
import { api, ApiError, type EngineId, type EngineInfo, type EngineState, type EnginesStatus } from "../api";
import { Button, ErrorNote, StatusDot } from "./ui";

const BUSY: EngineState[] = ["installing", "starting"];

/** Poll engine status: every second while something is installing or starting. */
export function useEngines(token: string, onUnauthorized?: () => void) {
  const [status, setStatus] = useState<EnginesStatus | null>(null);

  const refresh = useCallback(async () => {
    try {
      setStatus(await api.engines(token));
    } catch (err) {
      if (err instanceof ApiError && err.status === 401) onUnauthorized?.();
    }
  }, [token, onUnauthorized]);

  const busy = status?.engines.some((e) => BUSY.includes(e.state)) ?? false;
  useEffect(() => {
    void refresh();
    const t = window.setInterval(() => void refresh(), busy ? 1000 : 8000);
    return () => window.clearInterval(t);
  }, [refresh, busy]);

  return { status, refresh };
}

export function engineReady(status: EnginesStatus | null, id: EngineId) {
  const e = status?.engines.find((x) => x.id === id);
  return e?.state === "running" || e?.state === "external";
}

const STATE_LABEL: Record<EngineState, string> = {
  not_installed: "Not installed",
  installing: "Installing",
  stopped: "Stopped",
  starting: "Starting",
  running: "Running",
  external: "Running (your own copy)",
  error: "Problem",
};

const PURPOSE: Record<EngineId, string> = {
  ollama: "Runs chat and vision models.",
  comfyui: "Runs image and video workflows (Qwen-Image, Wan 2.2).",
  voice: "Hears you and speaks replies (Whisper and Kokoro), on the CPU.",
};

export function GpuLine({ status }: { status: EnginesStatus | null }) {
  if (!status) return null;
  if (status.gpus.length === 0) {
    return (
      <p className="rounded-xl bg-amber/10 px-4 py-3 text-sm text-amber">
        No NVIDIA graphics card found. Chat will work, but ComfyUI will run on the CPU and be very slow.
      </p>
    );
  }
  return (
    <p className="text-sm text-soft">
      {status.gpus
        .map((g) => `${g.name} · ${(g.vram_mb / 1024).toFixed(0)} GB`)
        .join(", ")}
    </p>
  );
}

export function EngineCard({
  engine,
  token,
  canManage,
  onChanged,
}: {
  engine: EngineInfo;
  token: string;
  canManage: boolean;
  onChanged: () => void;
}) {
  const [error, setError] = useState<string | null>(null);
  const [sending, setSending] = useState(false);

  async function act(action: "install" | "start" | "stop") {
    setSending(true);
    setError(null);
    try {
      await api.engineAction(token, engine.id, action);
      onChanged();
    } catch (err) {
      setError(err instanceof ApiError ? err.message : "Something went wrong.");
    } finally {
      setSending(false);
    }
  }

  const s = engine.state;
  const p = engine.progress;
  const installed = engine.version !== null;
  const outdated = installed && engine.version !== engine.pinned_version;

  return (
    <article className="flex flex-col gap-4 rounded-2xl border border-line bg-surface p-5">
      <header className="flex flex-wrap items-center gap-3">
        <h3 className="flex-1 text-lg font-semibold">{engine.name}</h3>
        <span className="flex items-center gap-2 rounded-full bg-raised px-3 py-1 text-xs text-soft">
          <StatusDot on={s === "running" || s === "external"} />
          {STATE_LABEL[s]}
        </span>
      </header>
      <p className="text-sm text-muted">
        {PURPOSE[engine.id]}{" "}
        {installed ? `Version ${engine.version}.` : s === "external" ? "" : `Installs ${engine.pinned_version}, ${engine.size_hint}.`}
        {outdated && <span className="text-pick"> Version {engine.pinned_version} is available.</span>}
      </p>

      {s === "external" && (
        <p className="text-sm text-soft">
          A copy you installed yourself is already running at <span className="font-mono">{engine.url}</span>, so
          Outpost uses it.
        </p>
      )}

      {s === "installing" && p && (
        <div className="flex flex-col gap-2" aria-live="polite">
          <div className="flex justify-between gap-4 text-sm">
            <span>
              Step {p.step_index} of {p.step_count}: {p.step}
            </span>
            {p.detail && <span className="text-muted">{p.detail}</span>}
          </div>
          <div className="h-2 overflow-hidden rounded-full bg-raised">
            {p.percent !== null ? (
              <div className="h-2 rounded-full bg-ember transition-all" style={{ width: `${p.percent}%` }} />
            ) : (
              <div className="h-2 w-1/3 animate-pulse rounded-full bg-ember/70" />
            )}
          </div>
        </div>
      )}

      {s === "starting" && <p className="text-sm text-soft">Starting… the first start of ComfyUI can take a few minutes.</p>}

      <ErrorNote>{s === "error" ? engine.error : null}</ErrorNote>
      <ErrorNote>{error}</ErrorNote>

      {engine.log_tail.length > 0 && (
        <details className="text-xs">
          <summary className="cursor-pointer text-muted">Recent output</summary>
          <pre className="mt-2 max-h-48 overflow-auto rounded-lg bg-deep p-3 font-mono whitespace-pre-wrap text-muted">
            {engine.log_tail.join("\n")}
          </pre>
        </details>
      )}

      {canManage && s !== "external" && (
        <div className="flex flex-wrap gap-2">
          {(s === "not_installed" || (s === "error" && !installed)) && (
            <Button onClick={() => void act("install")} disabled={sending}>
              Install {engine.name}
            </Button>
          )}
          {(s === "stopped" || (s === "error" && installed)) && (
            <Button onClick={() => void act("start")} disabled={sending}>
              Start
            </Button>
          )}
          {(s === "running" || s === "starting") && (
            <Button variant="ghost" onClick={() => void act("stop")} disabled={sending}>
              Stop
            </Button>
          )}
          {installed && s !== "installing" && (
            <Button variant={outdated ? "primary" : "ghost"} onClick={() => void act("install")} disabled={sending}>
              {outdated ? `Update to ${engine.pinned_version}` : "Reinstall"}
            </Button>
          )}
        </div>
      )}
    </article>
  );
}

export function EnginesPanel({
  token,
  canManage,
  status,
  onChanged,
}: {
  token: string;
  canManage: boolean;
  status: EnginesStatus | null;
  onChanged: () => void;
}) {
  return (
    <div className="flex flex-col gap-4">
      <GpuLine status={status} />
      <div className="grid grid-cols-1 gap-4 lg:grid-cols-2">
        {status?.engines.map((e) => (
          <EngineCard key={e.id} engine={e} token={token} canManage={canManage} onChanged={onChanged} />
        ))}
      </div>
      {!canManage && <p className="text-xs text-muted">Only an owner can install or stop engines.</p>}
    </div>
  );
}
