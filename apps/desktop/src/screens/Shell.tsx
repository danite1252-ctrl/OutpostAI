import { useState } from "react";
import type { SignInResponse } from "../api";
import { engineReady, useEngines } from "../components/engines";
import { Avatar, Icon, icons, Mark, StatusDot, Wordmark } from "../components/ui";
import Settings from "./Settings";
import Models from "./Models";
import Chat from "./Chat";
import Personas from "./Personas";
import Create from "./Create";
import Gallery from "./Gallery";
import Connections from "./Connections";
import Train from "./Train";
import Api from "./Api";
import { DownloadsButton } from "../components/downloads";
import { QueueButton } from "../components/queue";
import type { CreateRequest } from "../components/gallery";

type Section = "chat" | "create" | "gallery" | "train" | "models" | "personas" | "connections" | "api" | "settings";

const NAV: { id: Section; label: string; icon: string }[] = [
  { id: "chat", label: "Chat", icon: icons.chat },
  { id: "create", label: "Create", icon: icons.create },
  { id: "gallery", label: "Gallery", icon: icons.gallery },
  { id: "train", label: "Train", icon: icons.train },
  { id: "models", label: "Models", icon: icons.models },
  { id: "personas", label: "Personas", icon: icons.personas },
  { id: "connections", label: "Connections", icon: icons.connections },
  { id: "api", label: "API", icon: icons.api },
  { id: "settings", label: "Settings", icon: icons.settings },
];

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
  const [section, setSection] = useState<Section>("chat");
  const [createRequest, setCreateRequest] = useState<CreateRequest | null>(null);
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
          <QueueButton
            token={auth.token}
            onOpenCreate={() => setSection("create")}
            onOpenGallery={() => setSection("gallery")}
          />
          <DownloadsButton token={auth.token} />
          <div className="flex items-center gap-2.5 rounded-xl border border-line bg-surface py-1.5 pr-3 pl-1.5">
            <Avatar name={auth.account.name} color={auth.account.avatar_color} size={32} />
            <span className="flex flex-col leading-tight">
              <span className="text-sm font-medium">{auth.account.name}</span>
              <span className="text-xs text-muted capitalize">{auth.account.role}</span>
            </span>
          </div>
        </header>

        {section === "chat" && (
          <div className="min-h-0 flex-1">
            <Chat token={auth.token} onUnauthorized={onSignedOutElsewhere} />
          </div>
        )}
        <main className={`min-h-0 flex-1 overflow-auto p-8 ${section === "chat" ? "hidden" : ""}`}>
          {section === "personas" && <Personas token={auth.token} />}
          {section === "create" && (
            <Create
              token={auth.token}
              canManage={auth.account.role === "owner"}
              comfyInstalled={(engines?.engines.find((e) => e.id === "comfyui")?.state ?? "not_installed") !== "not_installed"}
              comfy={engines?.engines.find((e) => e.id === "comfyui") ?? null}
              onEnginesChanged={() => void refreshEngines()}
              request={createRequest}
              onRequestHandled={() => setCreateRequest(null)}
              onOpenGallery={() => setSection("gallery")}
              onUnauthorized={onSignedOutElsewhere}
            />
          )}
          {section === "gallery" && (
            <Gallery
              token={auth.token}
              me={auth.account}
              onRequest={(r) => {
                setCreateRequest(r);
                setSection("create");
              }}
              onUnauthorized={onSignedOutElsewhere}
            />
          )}
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
          {section === "connections" && <Connections auth={auth} />}
          {section === "api" && <Api auth={auth} onOpenConnections={() => setSection("connections")} />}
          {section === "train" && <Train auth={auth} onUnauthorized={onSignedOutElsewhere} />}
        </main>
      </div>
    </div>
  );
}
