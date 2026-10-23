import { useState } from "react";
import type { SignInResponse } from "../api";
import { engineReady, useEngines } from "../components/engines";
import { Avatar, Icon, icons, Mark, StatusDot, Wordmark } from "../components/ui";
import Settings from "./Settings";
import Models from "./Models";

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
  const { status: engines, refresh: refreshEngines } = useEngines(auth.token, onSignedOutElsewhere);
  const ollamaUp = engineReady(engines, "ollama");
  const comfyUp = engineReady(engines, "comfyui");

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
              Ollama
            </span>
            <span className="flex items-center gap-2">
              <StatusDot on={comfyUp} />
              ComfyUI
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
          {section === "models" && (
            <Models
              token={auth.token}
              canManage={auth.account.role === "owner"}
              ollama={engines?.engines.find((e) => e.id === "ollama") ?? null}
              onEnginesChanged={() => void refreshEngines()}
            />
          )}
          {section === "settings" && (
            <Settings
              auth={auth}
              version={version}
              engines={engines}
              onEnginesChanged={() => void refreshEngines()}
              onSignedOut={onSignedOutElsewhere}
            />
          )}
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
