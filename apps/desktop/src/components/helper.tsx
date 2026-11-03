// Lending this PC's graphics card to the server (Settings › Server, while using one).
//
// The server runs Create jobs it can't fit on its own card here instead: this PC checks in,
// runs them in its own ComfyUI and sends the pictures back. Lending for chat too, the server's
// chats can use the models in this PC's Ollama, running here. With ai-toolkit
// installed and a 16 GB card it takes trainings too, sending each checkpoint back as it's
// saved. Pause it while gaming (here or from the tray icon) and ComfyUI stops, so the card is
// all yours (a training stops too, keeping the checkpoints it sent).

import { useCallback, useEffect, useState } from "react";
import { ApiError, createApi, type CatalogEntry, type Fit, type InstalledModel, type Pack, type PackSetup, type PullState } from "../api";
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
  trainer_ready: boolean;
  trainer: {
    installing: boolean;
    error: string | null;
    pinned_version: string;
    progress: { step: string; step_index: number; step_count: number; percent: number | null } | null;
  } | null;
  chat: boolean;
  ollama: boolean;
  ollama_running: boolean;
  ollama_installing: boolean;
  ollama_error: string | null;
  chat_models: string[];
  chatting: number;
  chats: number;
}

/** ai-toolkit's settings need about 16 GB; cards report a little under. */
const TRAIN_MIN_MB = 15 * 1024;

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
  const [confirmPause, setConfirmPause] = useState(false);

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

  const training = status.state === "working" && status.pack_id === "train";
  const line =
    training
      ? `Training a character for the server${status.progress?.steps ? `: step ${status.progress.step} of ${status.progress.steps}` : status.progress ? `: ${status.progress.phase}` : ""}. Each checkpoint goes to the server as it's saved.`
      : status.state === "working"
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
            title="Lend this PC's graphics card"
            hint={`Jobs too big for the server's card are made here instead${
              status.vram_mb ? ` (this PC has ${gb(status.vram_mb)} of video memory)` : ""
            }, and (if you like) chats run here with this PC's models. Results go to the server.`}
            checked={status.enabled}
            onChange={(on) => set(on, false)}
          />
          {status.enabled && (
            <ToggleRow
              title="Pause (e.g. while gaming)"
              hint="ComfyUI stops and chat models unload so the card is free; jobs for this PC wait on the server, and chats use the server's own models. Also in the tray icon's menu."
              checked={status.paused}
              onChange={(on) => (on && training ? setConfirmPause(true) : set(true, on))}
            />
          )}
          {confirmPause && training && (
            <div className="flex flex-wrap items-center gap-3 rounded-xl border border-line bg-raised px-3 py-2" role="alert">
              <span className="flex-1 text-sm">
                A training is running here. Pausing stops it; the checkpoints already sent to the server are kept.
              </span>
              <Button
                variant="danger"
                onClick={() => {
                  setConfirmPause(false);
                  set(true, true);
                }}
              >
                Stop it and pause
              </Button>
              <Button variant="ghost" onClick={() => setConfirmPause(false)}>
                Keep training
              </Button>
            </div>
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
          {status.enabled && <ChatHere status={status} onError={setError} onChanged={refresh} />}
          {status.enabled && <TrainingHere status={status} onError={setError} onStarted={refresh} />}
        </>
      )}
      <ErrorNote>{error}</ErrorNote>
    </div>
  );
}

interface ChatModelsHere {
  installed: InstalledModel[];
  catalog: CatalogEntry[];
  pulls: PullState[];
}

/** Chats on this PC: the server runs chats with the models in Ollama here. */
function ChatHere({ status, onError, onChanged }: { status: HelperStatus; onError: (e: string) => void; onChanged: () => void }) {
  const [models, setModels] = useState<ChatModelsHere | null>(null);
  const [name, setName] = useState("");
  const load = useCallback(() => {
    if (!status.chat || !status.ollama_running) return;
    invoke<ChatModelsHere>("helper_chat_models")
      .then(setModels)
      .catch(() => {});
  }, [status.chat, status.ollama_running]);
  useEffect(() => {
    load();
    const t = window.setInterval(load, 3000);
    return () => window.clearInterval(t);
  }, [load]);

  const pull = (model: string) =>
    void invoke("helper_pull_chat_model", { model })
      .then(load)
      .catch((e) => onError(message(e)));
  const remove = (model: string) =>
    void invoke("helper_remove_chat_model", { model })
      .then(() => {
        load();
        onChanged();
      })
      .catch((e) => onError(message(e)));

  const chatModels = (models?.installed ?? []).filter((m) => !m.badges.includes("embedding"));
  const have = new Set(chatModels.map((m) => m.model.toLowerCase()));
  const pulling = (models?.pulls ?? []).filter((p) => !p.done);
  const failed = (models?.pulls ?? []).filter((p) => p.done && p.error);
  const suggested = (models?.catalog ?? [])
    // What fits this card, plus each kind's best pick (a coding pick may run partly on the processor).
    .filter((c) => (c.fit === "fits" || (c.recommended && c.fit === "partial")) && !have.has(c.model.toLowerCase()) && !pulling.some((p) => p.model === c.model))
    .sort((a, b) => Number(b.recommended) - Number(a.recommended) || b.size_gb - a.size_gb)
    .slice(0, 5);

  return (
    <div className="flex flex-col gap-2 border-t border-line pt-3">
      <ToggleRow
        title="Run chats here too"
        hint="Chats with a model on this PC run here on this card (bigger models than the server can fit). The server's own models still run on the server unless this card is bigger."
        checked={status.chat}
        onChange={(on) =>
          void invoke("set_helper_chat", { on })
            .then(onChanged)
            .catch((e) => onError(message(e)))
        }
      />
      {status.chat && !status.ollama && (
        <div className="flex flex-wrap items-center gap-3">
          <span className="flex-1 text-sm">
            {status.ollama_installing ? "Installing Ollama on this PC…" : "Ollama isn't on this PC yet (about 2 GB)."}
          </span>
          {!status.ollama_installing && (
            <Button
              onClick={() =>
                void invoke("helper_install_ollama")
                  .then(onChanged)
                  .catch((e) => onError(message(e)))
              }
            >
              Install Ollama here
            </Button>
          )}
          {status.ollama_error && <ErrorNote>{status.ollama_error}</ErrorNote>}
        </div>
      )}
      {status.chat && status.ollama && !status.ollama_running && !status.paused && (
        <p className="text-sm text-muted">Starting Ollama on this PC…</p>
      )}
      {status.chat && status.ollama_running && (
        <>
          <p className="text-sm" aria-live="polite">
            {chatModels.length
              ? status.chatting > 0
                ? `Answering ${status.chatting === 1 ? "a chat" : `${status.chatting} chats`} for the server now.`
                : "Chats with these models run here:"
              : "No chat models on this PC yet: get one below and it shows up in the server's model lists."}
            {status.chats > 0 ? ` ${status.chats} replies since Outpost started.` : ""}
          </p>
          {chatModels.length > 0 && (
            <ul className="flex flex-col gap-1">
              {chatModels.map((m) => (
                <li key={m.model} className="flex items-center gap-3 rounded-xl border border-line px-3 py-2 text-sm">
                  <span className="flex-1 font-mono">{m.model}</span>
                  <span className="text-muted">{(m.size_bytes / 1e9).toFixed(1)} GB</span>
                  <Button variant="ghost" onClick={() => remove(m.model)}>
                    Delete
                  </Button>
                </li>
              ))}
            </ul>
          )}
          {pulling.map((p) => (
            <p key={p.model} className="text-sm">
              Downloading {p.model}
              {p.percent != null ? `: ${Math.round(p.percent)}%` : `: ${p.status}`}
            </p>
          ))}
          {failed.map((p) => (
            <ErrorNote key={p.model}>{`${p.model}: ${p.error}`}</ErrorNote>
          ))}
          {suggested.length > 0 && (
            <>
              <p className="text-sm text-muted">Fit this PC's card:</p>
              <ul className="flex flex-col gap-1">
                {suggested.map((c) => (
                  <li key={c.model} className="flex items-center gap-3 rounded-xl border border-line px-3 py-2 text-sm">
                    <span className="flex-1">
                      {c.name} <span className="text-muted">({c.size_gb.toFixed(1)} GB)</span>
                      {c.recommended && (
                        <span className="ml-2 text-xs text-ember">
                          {c.fit === "partial" ? `Best for ${c.category === "coding" ? "code" : "this card"} · partly on the processor, a bit slower` : c.category === "coding" ? "Best for code" : "Best fit"}
                        </span>
                      )}
                    </span>
                    <Button onClick={() => pull(c.model)}>Get</Button>
                  </li>
                ))}
              </ul>
            </>
          )}
          <form
            className="flex flex-wrap items-center gap-2"
            onSubmit={(e) => {
              e.preventDefault();
              if (name.trim()) pull(name.trim());
              setName("");
            }}
          >
            <input
              className="min-w-0 flex-1 rounded-xl border border-line bg-raised px-3 py-2 text-sm"
              placeholder="Or any Ollama model, e.g. qwen3:14b"
              value={name}
              onChange={(e) => setName(e.target.value)}
              aria-label="Ollama model name"
            />
            <Button type="submit" disabled={!name.trim()}>
              Get
            </Button>
          </form>
        </>
      )}
    </div>
  );
}

/** Trainings on this PC: ai-toolkit installed here, and a card big enough. */
function TrainingHere({ status, onError, onStarted }: { status: HelperStatus; onError: (e: string) => void; onStarted: () => void }) {
  const t = status.trainer;
  const installing = t?.installing ?? false;
  if (status.vram_mb > 0 && status.vram_mb < TRAIN_MIN_MB && !status.trainer_ready) {
    return <p className="text-sm text-muted">Training needs a 16 GB card; this PC has {gb(status.vram_mb)}, so it lends for Create only.</p>;
  }
  if (status.trainer_ready && !installing) {
    return (
      <p className="text-sm text-muted">
        Trainings: ready. When the server's card is too small to train, trainings run here. The first one downloads Z-Image
        Turbo for training (about 20 GB) into this PC's Hugging Face cache.
      </p>
    );
  }
  const p = t?.progress;
  return (
    <div className="flex flex-col gap-2">
      <div className="flex flex-wrap items-center gap-3">
        <span className="flex-1 text-sm">
          {installing
            ? "Installing ai-toolkit on this PC…"
            : "Train characters here too: this PC's card is big enough. It needs ai-toolkit (about 8 GB, with PyTorch)."}
        </span>
        {!installing && (
          <Button
            variant="ghost"
            onClick={() =>
              void invoke("helper_install_trainer")
                .then(onStarted)
                .catch((e) => onError(message(e)))
            }
          >
            {t?.error ? "Try installing again" : "Install ai-toolkit here"}
          </Button>
        )}
      </div>
      {installing && p && (
        <span className="text-xs text-muted" aria-live="polite">
          Step {p.step_index} of {p.step_count}: {p.step}
          {p.percent != null ? ` · ${Math.round(p.percent)}%` : ""}
        </span>
      )}
      {t?.error && !installing && <ErrorNote>The last install failed: {t.error}</ErrorNote>}
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
