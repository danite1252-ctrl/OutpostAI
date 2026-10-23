// Typed client for the Outpost gateway. Mirrors /openapi.json; when the API grows
// (phase 1) this is replaced by a client generated from that spec.

export const GATEWAY = "http://127.0.0.1:7420";

export type Role = "owner" | "member" | "restricted" | "service";

export interface Account {
  id: string;
  name: string;
  role: Role;
  avatar_color: string;
  created_at: number;
}

export interface Session {
  id: string;
  account_id: string;
  kind: "desktop" | "device" | "app";
  label: string;
  created_at: number;
  last_used_at: number;
}

export interface SignInResponse {
  token: string;
  account: Account;
  session: Session;
}

export interface Health {
  status: string;
  version: string;
  name: string;
  setup_required: boolean;
}

export interface DirectoryEntry {
  id: string;
  name: string;
  role: Role;
  avatar_color: string;
}

export interface ModelEntry {
  id: string;
  object: "model";
  created: number;
  owned_by: string;
}

export type EngineId = "ollama" | "comfyui";
export type EngineState =
  | "not_installed"
  | "installing"
  | "stopped"
  | "starting"
  | "running"
  | "external"
  | "error";

export interface EngineInfo {
  id: EngineId;
  name: string;
  state: EngineState;
  managed: boolean;
  version: string | null;
  pinned_version: string;
  url: string;
  size_hint: string;
  progress: {
    step: string;
    step_index: number;
    step_count: number;
    percent: number | null;
    detail: string | null;
  } | null;
  error: string | null;
  log_tail: string[];
}

export interface EnginesStatus {
  gpus: { vendor: string; name: string; vram_mb: number }[];
  torch_variant: string;
  engines: EngineInfo[];
}

export type Badge = "vision" | "tools" | "thinking" | "uncensored" | "code" | "long_context" | "embedding";
export type Category = "uncensored" | "general" | "coding" | "embedding";
export type Fit = "fits" | "partial" | "too_large";
export type Tier = "cpu" | "starter" | "standard" | "performance" | "workstation";

export interface Hardware {
  gpus: { vendor: string; name: string; vram_mb: number }[];
  vram_mb: number;
  ram_mb: number;
  cpu_threads: number;
  disk_free_mb: number;
  tier: Tier;
}

export interface CatalogEntry {
  model: string;
  name: string;
  category: Category;
  size_gb: number;
  context_k: number;
  badges: Badge[];
  license: string | null;
  description: string;
  page: string;
  fit: Fit;
  recommended: boolean;
}

export interface InstalledModel {
  model: string;
  size_bytes: number;
  parameter_size: string | null;
  quantization: string | null;
  family: string | null;
  badges: Badge[];
}

export interface PullState {
  model: string;
  status: string;
  completed: number;
  total: number;
  percent: number | null;
  done: boolean;
  error: string | null;
}

export interface ModelsOverview {
  hardware: Hardware;
  catalog: CatalogEntry[];
  installed: InstalledModel[];
  pulls: PullState[];
  ollama_running: boolean;
}

export interface Persona {
  id: string;
  account_id: string;
  slug: string;
  name: string;
  avatar_color: string;
  system_prompt: string;
  model: string | null;
  temperature: number | null;
  think: boolean;
  use_memory: boolean;
  created_at: number;
  updated_at: number;
}

export type PersonaInput = Partial<{
  name: string;
  system_prompt: string;
  model: string;
  temperature: number;
  think: boolean;
  use_memory: boolean;
  avatar_color: string;
  /** Change the API id (persona:<slug>). Normally made from the name. */
  slug: string;
}>;

export interface Chat {
  id: string;
  account_id: string;
  persona_id: string | null;
  title: string;
  model: string | null;
  created_at: number;
  updated_at: number;
}

export interface Attachment {
  id: string;
  message_id: string;
  kind: "image";
  mime: string;
  bytes: number;
  created_at: number;
}

export interface Message {
  id: string;
  chat_id: string;
  seq: number;
  role: "user" | "assistant" | "system";
  content: string;
  thinking: string | null;
  model: string | null;
  created_at: number;
  attachments: Attachment[];
}

export interface Memory {
  id: string;
  account_id: string;
  persona_id: string | null;
  text: string;
  /** "manual": you added it (always used). "auto": learned from a chat (used when relevant). */
  source: "manual" | "auto";
  chat_id: string | null;
  created_at: number;
}

export interface MemorySettings {
  auto: boolean;
  embedding_model: string | null;
}

export interface NetworkStatus {
  /** The saved "Allow other devices on my network" setting. */
  lan_access: boolean;
  /** Where the gateway is listening right now, e.g. "0.0.0.0:7420". */
  listening_on: string | null;
  /** The setting differs from what is running: restart Outpost to apply it. */
  restart_required: boolean;
  /** This PC's address on the local network. */
  lan_address: string | null;
  /** Base URL for apps on other devices, when network access is running. */
  base_url: string | null;
}

export class ApiError extends Error {
  constructor(
    public status: number,
    public code: string,
    message: string,
    public retryAfter?: number,
    public attemptsLeft?: number,
  ) {
    super(message);
  }
}

async function request<T>(
  method: string,
  path: string,
  opts: { token?: string; body?: unknown } = {},
): Promise<T> {
  const headers: Record<string, string> = {};
  if (opts.token) headers.Authorization = `Bearer ${opts.token}`;
  if (opts.body !== undefined) headers["Content-Type"] = "application/json";

  let res: Response;
  try {
    res = await fetch(GATEWAY + path, {
      method,
      headers,
      body: opts.body === undefined ? undefined : JSON.stringify(opts.body),
    });
  } catch {
    throw new ApiError(0, "offline", "Can't reach the Outpost service.");
  }

  if (res.status === 204) return undefined as T;
  const data = await res.json().catch(() => null);
  if (!res.ok) {
    const e = data?.error ?? {};
    throw new ApiError(
      res.status,
      e.code ?? "unknown",
      e.message ?? `Request failed (${res.status}).`,
      e.retry_after,
      e.attempts_left,
    );
  }
  return data as T;
}

export const api = {
  health: () => request<Health>("GET", "/api/health"),
  directory: () => request<DirectoryEntry[]>("GET", "/api/accounts/directory"),
  setupOwner: (name: string, secret: string) =>
    request<SignInResponse>("POST", "/api/setup/owner", { body: { name, secret } }),
  signIn: (accountId: string, secret: string) =>
    request<SignInResponse>("POST", "/api/auth/sign-in", {
      body: { account_id: accountId, secret, label: "Outpost desktop" },
    }),
  signOut: (token: string) => request<void>("POST", "/api/auth/sign-out", { token }),
  changeMySecret: (token: string, currentSecret: string, newSecret: string) =>
    request<void>("PUT", "/api/me/secret", {
      token,
      body: { current_secret: currentSecret, new_secret: newSecret },
    }),
  listAccounts: (token: string) => request<Account[]>("GET", "/api/accounts", { token }),
  createAccount: (token: string, body: { name: string; role: Role; secret: string | null }) =>
    request<Account>("POST", "/api/accounts", { token, body }),
  deleteAccount: (token: string, id: string) =>
    request<void>("DELETE", `/api/accounts/${encodeURIComponent(id)}`, { token }),
  resetSecret: (token: string, id: string, secret: string) =>
    request<void>("PUT", `/api/accounts/${encodeURIComponent(id)}/secret`, { token, body: { secret } }),
  models: (token: string) => request<{ object: "list"; data: ModelEntry[] }>("GET", "/v1/models", { token }),
  modelsOverview: (token: string) => request<ModelsOverview>("GET", "/api/models", { token }),
  pullModel: (token: string, model: string) => request<void>("POST", "/api/models/pull", { token, body: { model } }),
  removeModel: (token: string, model: string) =>
    request<void>("POST", "/api/models/remove", { token, body: { model } }),
  personas: (token: string) => request<Persona[]>("GET", "/api/personas", { token }),
  createPersona: (token: string, body: PersonaInput) => request<Persona>("POST", "/api/personas", { token, body }),
  updatePersona: (token: string, id: string, body: PersonaInput) =>
    request<Persona>("PATCH", `/api/personas/${id}`, { token, body }),
  deletePersona: (token: string, id: string) => request<void>("DELETE", `/api/personas/${id}`, { token }),
  chats: (token: string) => request<Chat[]>("GET", "/api/chats", { token }),
  createChat: (token: string, personaId?: string) =>
    request<Chat>("POST", "/api/chats", { token, body: { persona_id: personaId ?? null } }),
  chat: (token: string, id: string) => request<{ chat: Chat; messages: Message[] }>("GET", `/api/chats/${id}`, { token }),
  updateChat: (token: string, id: string, body: { title?: string; persona_id?: string; model?: string }) =>
    request<Chat>("PATCH", `/api/chats/${id}`, { token, body }),
  deleteChat: (token: string, id: string) => request<void>("DELETE", `/api/chats/${id}`, { token }),
  memory: (token: string) => request<Memory[]>("GET", "/api/memory", { token }),
  addMemory: (token: string, text: string, personaId?: string) =>
    request<Memory>("POST", "/api/memory", { token, body: { text, persona_id: personaId ?? null } }),
  deleteMemory: (token: string, id: string) => request<void>("DELETE", `/api/memory/${id}`, { token }),
  memorySettings: (token: string) => request<MemorySettings>("GET", "/api/memory/settings", { token }),
  setMemorySettings: (token: string, auto: boolean) =>
    request<MemorySettings>("PUT", "/api/memory/settings", { token, body: { auto } }),
  clearAutoMemory: (token: string) => request<{ removed: number }>("DELETE", "/api/memory/auto", { token }),
  keys: (token: string, accountId?: string) =>
    request<Session[]>("GET", `/api/keys${accountId ? `?account_id=${encodeURIComponent(accountId)}` : ""}`, { token }),
  createKey: (token: string, label: string, accountId?: string) =>
    request<SignInResponse>("POST", "/api/keys", { token, body: { label, account_id: accountId ?? null } }),
  revokeKey: (token: string, id: string) => request<void>("DELETE", `/api/keys/${id}`, { token }),
  network: (token: string) => request<NetworkStatus>("GET", "/api/settings/network", { token }),
  setNetwork: (token: string, lanAccess: boolean) =>
    request<NetworkStatus>("PUT", "/api/settings/network", { token, body: { lan_access: lanAccess } }),
  engines: (token: string) => request<EnginesStatus>("GET", "/api/engines", { token }),
  engineAction: (token: string, id: EngineId, action: "install" | "start" | "stop") =>
    request<void>("POST", `/api/engines/${id}/${action}`, { token }),
};

/** Same rules as the gateway, so the form can explain before submitting. */
export function secretProblem(secret: string): string | null {
  const isPin = /^\d+$/.test(secret);
  if (isPin) return secret.length >= 4 && secret.length <= 12 ? null : "A PIN must be 4 to 12 digits.";
  return secret.length >= 6 ? null : "A password must be at least 6 characters (or use a 4–12 digit PIN).";
}

export interface StreamHandlers {
  start?: (data: { user_message: Message; model: string }) => void;
  thinking?: (text: string) => void;
  token?: (text: string) => void;
  done?: (data: { message: Message; error: string | null }) => void;
  error?: (message: string) => void;
}

/** Send a chat message and follow the streamed reply (Server-Sent Events over POST). */
export async function streamMessage(
  token: string,
  chatId: string,
  content: string,
  images: string[],
  on: StreamHandlers,
  signal?: AbortSignal,
): Promise<void> {
  let res: Response;
  try {
    res = await fetch(`${GATEWAY}/api/chats/${chatId}/messages`, {
      method: "POST",
      headers: { Authorization: `Bearer ${token}`, "Content-Type": "application/json" },
      body: JSON.stringify({ content, images }),
      signal,
    });
  } catch {
    throw new ApiError(0, "offline", "Can't reach the Outpost service.");
  }
  if (!res.ok || !res.body) {
    const data = await res.json().catch(() => null);
    const e = data?.error ?? {};
    throw new ApiError(res.status, e.code ?? "unknown", e.message ?? `Request failed (${res.status}).`);
  }
  const reader = res.body.getReader();
  const decoder = new TextDecoder();
  let buffer = "";
  for (;;) {
    const { value, done } = await reader.read();
    if (done) break;
    buffer += decoder.decode(value, { stream: true }).replace(/\r\n/g, "\n");
    let cut: number;
    while ((cut = buffer.indexOf("\n\n")) >= 0) {
      const block = buffer.slice(0, cut);
      buffer = buffer.slice(cut + 2);
      let name = "message";
      const data: string[] = [];
      for (const line of block.split("\n")) {
        if (line.startsWith("event:")) name = line.slice(6).trim();
        else if (line.startsWith("data:")) data.push(line.slice(5).replace(/^ /, ""));
      }
      if (data.length === 0) continue; // keep-alive
      const payload = JSON.parse(data.join("\n"));
      if (name === "start") on.start?.(payload);
      else if (name === "thinking") on.thinking?.(payload.text);
      else if (name === "token") on.token?.(payload.text);
      else if (name === "done") on.done?.(payload);
      else if (name === "error") on.error?.(payload.message);
    }
  }
}

/** An attached image as an object URL (the request needs the token, so <img src> can't fetch it directly). */
export async function fileObjectUrl(token: string, id: string): Promise<string> {
  const res = await fetch(`${GATEWAY}/api/files/${encodeURIComponent(id)}`, {
    headers: { Authorization: `Bearer ${token}` },
  }).catch(() => null);
  if (!res || !res.ok) throw new ApiError(res?.status ?? 0, "not_found", "Image not found.");
  return URL.createObjectURL(await res.blob());
}

/** Max images per message and size each, matching the gateway. */
export const MAX_IMAGES = 4;
export const MAX_IMAGE_BYTES = 10 * 1024 * 1024;
