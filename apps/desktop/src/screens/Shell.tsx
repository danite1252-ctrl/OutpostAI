import { useCallback, useEffect, useState } from "react";
import { api, ApiError, type Engines, type ModelEntry, type SignInResponse } from "../api";
import { Avatar, Button, Icon, icons, Mark, StatusDot, Wordmark } from "../components/ui";
import Settings from "./Settings";

type Section = "chat" | "create" | "train" | "models" | "personas" | "connections" | "settings";

const NAV: { id: Section; label: string; icon: string }[] = [
  { id: "chat", label: "Chat", icon: icons.chat },
  { id: "create", label: "Create", icon: icons.create },
  { id: "train", label: "Train", icon: icons.train },
  { id: "models", label: "Models", icon: icons.models },
  { id: "personas", label: "Personas", icon: icons.personas },
  { id: "connections", label: "Connections", icon: icons.connections },
  { id: "settings", label: "Settings", icon: icons.settings },
];

const COMING: Record<Exclude<Section, "models" | "settings">, { phase: number; text: string }> = {
  chat: { phase: 1, text: "Chat with personas, memory and vision models." },
  create: { phase: 2, text: "Qwen-Image and Wan 2.2 workflows, with a shared GPU queue." },
  train: { phase: 4, text: "Training Studio: datasets, vision captions and LoRAs." },
  personas: { phase: 1, text: "Personas with their own prompt, model, voice and tools." },
  connections: { phase: 3, text: "Pair your phone, Tailscale, Cloudflare and app keys for Hermes." },
};

export default function Shell({
  auth,
  version,
  onLock,
  onSignedOutElsewhere,
}: {
  auth: SignInResponse;
  version: string;
  onLock: () => void;
  onSignedOutElsewhere: () => void;
}) {
  const [section, setSection] = useState<Section>("models");
  const [engines, setEngines] = useState<Engines | null>(null);
  const [models, setModels] = useState<ModelEntry[] | null>(null);

  const refresh = useCallback(async () => {
    try {
      const [e, m] = await Promise.all([api.engines(auth.token), api.models(auth.token)]);
      setEngines(e);
      setModels(m.data);
    } catch (err) {
      if (err instanceof ApiError && err.status === 401) onSignedOutElsewhere();
    }
  }, [auth.token, onSignedOutElsewhere]);

  useEffect(() => {
    void refresh();
    const t = window.setInterval(() => void refresh(), 15_000);
    return () => window.clearInterval(t);
  }, [refresh]);

  const ollamaUp = engines?.ollama.reachable ?? false;

  return (
    <div className="flex h-full bg-night">
      <nav aria-label="Main" className="flex w-[76px] shrink-0 flex-col items-center gap-1.5 border-r border-line py-4">
        <Mark size={44} className="mb-4" />
        {NAV.map((n) => (
          <button
            key={n.id}
            type="button"
            aria-label={n.label}
            aria-current={section === n.id ? "page" : undefined}
            title={n.label}
            onClick={() => setSection(n.id)}
            className={`flex size-12 items-center justify-center rounded-xl transition-colors ${
              section === n.id ? "bg-raised text-ember" : "text-muted hover:bg-surface hover:text-ink"
            }`}
          >
            <Icon d={n.icon} size={22} />
          </button>
        ))}
        <div className="flex-1" />
        <button
          type="button"
          aria-label="Lock Outpost"
          title="Lock"
          onClick={onLock}
          className="flex size-12 items-center justify-center rounded-xl text-muted hover:bg-surface hover:text-ink"
        >
          <Icon d={icons.lock} size={22} />
        </button>
      </nav>

      <div className="flex min-w-0 flex-1 flex-col">
        <header className="flex flex-wrap items-center gap-4 border-b border-line px-6 py-3">
          <Wordmark className="text-lg" />
          <div className="flex-1" />
          <div className="flex flex-wrap items-center gap-4 text-sm text-muted">
            <span className="flex items-center gap-2">
              <StatusDot on />
              Gateway :7420
            </span>
            <span className="flex items-center gap-2">
              <StatusDot on={ollamaUp} />
              {ollamaUp ? `Ollama ${engines?.ollama.version ?? ""}` : "Ollama not running"}
            </span>
          </div>
          <div className="flex items-center gap-2.5 rounded-xl border border-line bg-surface py-1.5 pr-3 pl-1.5">
            <Avatar name={auth.account.name} color={auth.account.avatar_color} size={32} />
            <span className="flex flex-col leading-tight">
              <span className="text-sm font-medium">{auth.account.name}</span>
              <span className="text-xs text-muted capitalize">{auth.account.role}</span>
            </span>
          </div>
        </header>

        <main className="min-h-0 flex-1 overflow-auto p-8">
          {section === "models" && <ModelsView models={models} ollamaUp={ollamaUp} onRefresh={refresh} />}
          {section === "settings" && <Settings auth={auth} version={version} onSignedOut={onSignedOutElsewhere} />}
          {section !== "models" && section !== "settings" && (
            <ComingSoon label={NAV.find((n) => n.id === section)!.label} {...COMING[section]} />
          )}
        </main>
      </div>
    </div>
  );
}

function ComingSoon({ label, phase, text }: { label: string; phase: number; text: string }) {
  return (
    <section className="mx-auto mt-16 flex max-w-lg flex-col items-center gap-3 text-center">
      <span className="rounded-full bg-raised px-3 py-1 font-mono text-xs text-amber">Phase {phase}</span>
      <h1 className="text-2xl font-semibold">{label}</h1>
      <p className="text-muted">{text}</p>
    </section>
  );
}

function ModelsView({
  models,
  ollamaUp,
  onRefresh,
}: {
  models: ModelEntry[] | null;
  ollamaUp: boolean;
  onRefresh: () => void;
}) {
  return (
    <section className="mx-auto flex max-w-4xl flex-col gap-6">
      <div className="flex flex-wrap items-end gap-4">
        <div className="flex-1">
          <h1 className="text-2xl font-semibold">Models</h1>
          <p className="text-muted">
            What <code className="font-mono text-soft">GET /v1/models</code> returns right now. The full catalogue
            with fit scores and badges arrives in phase 1.
          </p>
        </div>
        <Button variant="ghost" onClick={onRefresh}>
          Refresh
        </Button>
      </div>
      {!ollamaUp && (
        <div className="rounded-2xl border border-line bg-surface p-5 text-sm leading-relaxed text-soft">
          Ollama isn't running. Install it from ollama.com, start it, then press Refresh. Outpost will install and
          manage it for you from phase 1.
        </div>
      )}
      {ollamaUp && models?.length === 0 && (
        <div className="rounded-2xl border border-line bg-surface p-5 text-sm text-soft">
          Ollama is running but has no models yet. Try <code className="font-mono">ollama pull qwen3:8b</code>.
        </div>
      )}
      {models && models.length > 0 && (
        <ul className="overflow-hidden rounded-2xl border border-line">
          {models.map((m) => (
            <li key={m.id} className="flex items-center gap-4 border-b border-line bg-surface px-5 py-3.5 last:border-b-0">
              <span className="flex-1 font-mono">{m.id}</span>
              <span className="text-sm text-muted">{m.owned_by}</span>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
