// Lending this PC's graphics card to the server (Settings › Server, while using one).
//
// The server runs Create jobs it can't fit on its own card here instead: this PC checks in,
// runs them in its own ComfyUI (never Ollama) and sends the pictures back. Pause it while
// gaming (here or from the tray icon) and ComfyUI stops, so the card is all yours.

import { useCallback, useEffect, useState } from "react";
import { ApiError, createApi, type Fit, type Pack, type PackSetup } from "../api";
import { Button, ErrorNote, ToggleRow } from "./ui";

interface HelperStatus {
  enabled: boolean;
  paused: boolean;
  state: "off" | "paused" | "ready" | "working" | "problem";
  message: string | null;
  pack_id: string | null;
  progress: { phase: string; step: number; steps: number; percent: number | null } | null;
  done: number;
  vram_mb: number;
  ready_packs: string[];
  comfyui: boolean;
  comfyui_installing: boolean;
  comfyui_error: string | null;
}

interface LocalPack {
  id: string;
  name: string;
  vram_gb: number;
  fit: Fit;
  models: { ready: boolean; missing_gb: number; setup: PackSetup | null };
}

async function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<T>(cmd, args);
}

function message(err: unknown) {
  if (err instanceof ApiError) return err.message;
  if (typeof err === "string") return err;
  return "Something went wrong.";
}

const gb = (mb: number) => `${Math.round(mb / 1024)} GB`;

export function HelperPanel({ token, isOwner }: { token: string; isOwner: boolean }) {
  const [status, setStatus] = useState<HelperStatus | null>(null);
  const [local, setLocal] = useState<LocalPack[]>([]);
  const [server, setServer] = useState<Pack[]>([]);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    const s = await invoke<HelperStatus | null>("helper_status").catch(() => null);
    setStatus(s);
    if (s?.enabled) setLocal(await invoke<LocalPack[]>("helper_packs").catch(() => []));
  }, []);

  useEffect(() => {
    void refresh();
    const t = window.setInterval(() => void refresh(), 2500);
    // The tray's "Pause lending this PC".
    let unlisten: (() => void) | undefined;
    void import("@tauri-apps/api/event")
      .then(({ listen }) => listen("helper-changed", () => void refresh()))
      .then((u) => (unlisten = u))
      .catch(() => {});
    return () => {
      window.clearInterval(t);
      unlisten?.();
    };
  }, [refresh]);

  useEffect(() => {
    if (!status?.enabled) return;
    // Every workflow, as the server sees it (to tell which ones it can't run itself).
    createApi
      .packsAll(token)
      .then(setServer)
      .catch(() => {});
  }, [token, status?.enabled, status?.ready_packs.length]);

  if (!status) return null;

  const set = (enabled: boolean, paused: boolean) => {
    setError(null);
    invoke("set_helper", { enabled, paused })
      .then(refresh)
      .catch((e) => setError(message(e)));
  };

  // Worth downloading here: too big for the server's card (or not on the server), and
  // fitting this PC's.
  const serverCant = new Set(server.filter((p) => p.fit !== "fits" || (!p.models.ready && !p.runs_on)).map((p) => p.id));
  const offer = local.filter((p) => p.fit === "fits" && serverCant.has(p.id));

  const line =
    status.state === "working"
      ? `Making something for the server${status.progress ? `: ${status.progress.phase}${status.progress.steps ? `, step ${status.progress.step} of ${status.progress.steps}` : ""}` : ""}.`
      : status.state === "paused"
        ? "Paused: no jobs are taken, and ComfyUI is stopped so the card is free."
        : status.state === "ready"
          ? (status.message ?? "Ready: the server sends jobs here that are too big for its own card.")
          : status.state === "problem"
            ? (status.message ?? "There's a problem reaching the server.")
            : null;

  return (
    <div className="flex flex-col gap-3 rounded-2xl border border-line bg-surface p-5">
      {!isOwner ? (
        <p className="text-sm text-muted">
          Pictures and videos too big for the server's card can be made on a PC that lends its card. Only an owner can
          lend a PC.
        </p>
      ) : (
        <>
          <ToggleRow
            title="Lend this PC for Create jobs"
            hint={`Jobs too big for the server's card are made here instead${
              status.vram_mb ? ` (this PC has ${gb(status.vram_mb)} of video memory)` : ""
            }. Only ComfyUI runs here, only while there's a job, and the results go to the server's gallery.`}
            checked={status.enabled}
            onChange={(on) => set(on, false)}
          />
          {status.enabled && (
            <ToggleRow
              title="Pause (e.g. while gaming)"
              hint="ComfyUI stops so the card is free; jobs for this PC wait on the server. Also in the tray icon's menu."
              checked={status.paused}
              onChange={(on) => set(true, on)}
            />
          )}
          {status.enabled && line && (
            <p
              className={`text-sm ${status.state === "problem" ? "text-danger" : status.state === "working" ? "text-ink" : "text-muted"}`}
              aria-live="polite"
            >
              {line}
              {status.done > 0 ? ` ${status.done} done since Outpost started.` : ""}
            </p>
          )}
          {status.enabled && !status.comfyui && (
            <div className="flex flex-wrap items-center gap-3">
              <span className="flex-1 text-sm">
                {status.comfyui_installing
                  ? "Installing ComfyUI on this PC…"
                  : "ComfyUI isn't on this PC yet. It's a few GB and takes a few minutes."}
              </span>
              {!status.comfyui_installing && (
                <Button
                  onClick={() =>
                    void invoke("helper_install_comfyui")
                      .then(refresh)
                      .catch((e) => setError(message(e)))
                  }
                >
                  Install ComfyUI here
                </Button>
              )}
              {status.comfyui_error && <ErrorNote>{status.comfyui_error}</ErrorNote>}
            </div>
          )}
          {status.enabled && status.comfyui && (
            <div className="flex flex-col gap-2">
              <p className="text-sm">
                {status.ready_packs.length
                  ? `Ready here: ${status.ready_packs.map((id) => local.find((p) => p.id === id)?.name ?? id).join(", ")}.`
                  : "No workflows are downloaded on this PC yet."}
              </p>
              {offer.filter((p) => !p.models.ready).length > 0 && (
                <>
                  <p className="text-sm text-muted">Too big for the server, but they fit this PC:</p>
                  <ul className="flex flex-col gap-2">
                    {offer
                      .filter((p) => !p.models.ready)
                      .map((p) => (
                        <OfferRow key={p.id} pack={p} onError={setError} onStarted={refresh} />
                      ))}
                  </ul>
                </>
              )}
            </div>
          )}
        </>
      )}
      <ErrorNote>{error}</ErrorNote>
    </div>
  );
}

function OfferRow({ pack, onError, onStarted }: { pack: LocalPack; onError: (e: string) => void; onStarted: () => void }) {
  const setup = pack.models.setup;
  const running = setup?.running ?? false;
  return (
    <li className="flex flex-wrap items-center gap-3 rounded-xl border border-line px-3 py-2">
      <span className="flex min-w-0 flex-1 flex-col">
        <span className="text-sm font-semibold">{pack.name}</span>
        <span className="text-xs text-muted">
          {running
            ? `Downloading${setup?.percent != null ? ` ${Math.round(setup.percent)}%` : "…"}`
            : `${pack.models.missing_gb.toFixed(1)} GB to download · needs about ${pack.vram_gb} GB of video memory`}
          {setup?.error ? ` · ${setup.error}` : ""}
        </span>
      </span>
      {!running && (
        <Button
          variant="ghost"
          onClick={() =>
            void invoke("helper_install_pack", { id: pack.id })
              .then(onStarted)
              .catch((e) => onError(message(e)))
          }
        >
          Download on this PC
        </Button>
      )}
    </li>
  );
}
