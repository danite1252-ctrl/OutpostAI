// Typed client for the Outpost gateway. Mirrors /openapi.json; when the API grows
// (phase 1) this is replaced by a client generated from that spec.

/** Where API calls go: this PC's gateway, or (when the app uses a server) the encrypted
 * tunnel to it on 127.0.0.1. Set by `initGateway` before the window renders. */
export let GATEWAY = "http://127.0.0.1:7420";

/** The Outpost server this app uses instead of its own, or null. */
export interface ServerInfo {
  host: string;
  port: number;
  name: string;
  words: string[];
  fingerprint: string;
  token: string | null;
  /** Everyone paired on this PC (a shared PC), each with their own sign-in. */
  people: Person[];
  /** The addresses tried, in order: the home network first, then Tailscale. */
  hosts: string[];
}

export interface Person {
  account_id: string;
  name: string;
  avatar_color: string;
  token: string;
}
export let SERVER: ServerInfo | null = null;
/** A server this app used before (it now runs Outpost on this PC); going back needs no pairing. */
export let SAVED_SERVER: ServerInfo | null = null;

const inTauri = () => typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

/** Ask the app shell where the gateway is (and whether it's a server). */
export async function initGateway() {
  if (!inTauri()) return;
  const { invoke } = await import("@tauri-apps/api/core");
  GATEWAY = await invoke<string>("gateway_url");
  SERVER = await invoke<ServerInfo | null>("server_status");
  SAVED_SERVER = SERVER ? null : await invoke<ServerInfo | null>("saved_server").catch(() => null);
}

/** Switch to (or with null, sign out of) a sign-in on this app's server. Signing out also
 * forgets that person on this PC. */
export async function saveServerToken(token: string | null) {
  if (!inTauri() || !SERVER) return;
  const { invoke } = await import("@tauri-apps/api/core");
  await invoke("save_server_token", { token });
  const people = token === null ? SERVER.people.filter((p) => p.token !== SERVER?.token) : SERVER.people;
  SERVER = { ...SERVER, token, people };
}

/** Learn the server's current addresses (home network, Tailscale) so this app can reach it
 * at home and away: the home one first, falling back to Tailscale. */
export async function refreshServerHosts(token: string) {
  if (!inTauri() || !SERVER) return;
  const c = await request<{ hosts: string[]; port: number | null }>("GET", "/api/connection", { token });
  if (!c.hosts.length) return;
  const { invoke } = await import("@tauri-apps/api/core");
  await invoke("save_server_hosts", { hosts: c.hosts });
  const typed = SERVER.host;
  SERVER = { ...SERVER, hosts: c.hosts.some((h) => h.toLowerCase() === typed.toLowerCase()) ? c.hosts : [...c.hosts, typed] };
}

/** Make sure the signed-in person is in this PC's list (sign-ins from before shared PCs). */
export async function rememberPerson(person: Person) {
  if (!inTauri() || !SERVER || SERVER.people.some((p) => p.account_id === person.account_id)) return;
  const { invoke } = await import("@tauri-apps/api/core");
  await invoke("save_server", {
    host: SERVER.host,
    port: SERVER.port,
    fingerprint: SERVER.fingerprint,
    name: SERVER.name,
    token: person.token,
    person,
  });
  SERVER = { ...SERVER, token: person.token, people: [...SERVER.people, person] };
}

/** Forget one person on this shared PC. */
export async function forgetPerson(accountId: string) {
  if (!inTauri() || !SERVER) return;
  const { invoke } = await import("@tauri-apps/api/core");
  await invoke("forget_person", { accountId });
  const gone = SERVER.people.find((p) => p.account_id === accountId);
  SERVER = {
    ...SERVER,
    people: SERVER.people.filter((p) => p.account_id !== accountId),
    token: gone && gone.token === SERVER.token ? null : SERVER.token,
  };
}

/** Asks for the PIN when the server wants it again for a risky action (a paired device,
 * e.g. this app using a server). Resolves null if the person cancels. */
type PinPrompter = (reason: string) => Promise<string | null>;
let pinPrompter: PinPrompter | null = null;
export function setPinPrompter(p: PinPrompter | null) {
  pinPrompter = p;
}

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
  /** Ollama's context window on this Outpost, in tokens. */
  context_length?: number;
}

export type EngineId = "ollama" | "comfyui" | "voice";
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
  part: number;
  speed_bps: number | null;
  eta_secs: number | null;
  finished_at: number | null;
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
  /** Speaking voice (e.g. af_heart); null = Outpost's default. */
  voice: string | null;
  /** Read replies aloud in chat. */
  speak_replies: boolean;
  /** Shared with everyone on this Outpost. */
  shared?: boolean;
  /** Someone else's shared persona: whose it is (theirs to edit). */
  owner_name?: string | null;
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
  /** Speaking voice; "" for the default. */
  voice: string;
  speak_replies: boolean;
  /** Share it with everyone on this Outpost. */
  shared: boolean;
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
  /** Document passages the reply used, numbered as it cites them ("[1]"). */
  sources?: Source[];
}

export interface Source {
  n: number;
  document_id: string;
  name: string;
  page: number | null;
}

/** A document to chat with. */
export interface Doc {
  id: string;
  account_id: string;
  name: string;
  mime: string;
  bytes: number;
  /** "reading" (being read), "ready" or "failed" (see error). */
  status: "reading" | "ready" | "failed";
  error: string | null;
  pages: number | null;
  chars: number;
  created_at: number;
}

/** The kinds of document Outpost reads, for file pickers. */
export const DOC_TYPES = ".pdf,.docx,.txt,.md,.markdown,.csv,.html,.htm";

export const docsApi = {
  list: (token: string) => request<Doc[]>("GET", "/api/documents", { token }),
  remove: (token: string, id: string) => request<void>("DELETE", `/api/documents/${encodeURIComponent(id)}`, { token }),
  forChat: (token: string, chatId: string) =>
    request<{ documents: Doc[]; persona_documents: Doc[] }>("GET", `/api/chats/${encodeURIComponent(chatId)}/documents`, { token }),
  setForChat: (token: string, chatId: string, ids: string[]) =>
    request<{ documents: Doc[]; persona_documents: Doc[] }>("PUT", `/api/chats/${encodeURIComponent(chatId)}/documents`, {
      token,
      body: { document_ids: ids },
    }),
  forPersona: (token: string, personaId: string) =>
    request<Doc[]>("GET", `/api/personas/${encodeURIComponent(personaId)}/documents`, { token }),
  setForPersona: (token: string, personaId: string, ids: string[]) =>
    request<Doc[]>("PUT", `/api/personas/${encodeURIComponent(personaId)}/documents`, { token, body: { document_ids: ids } }),
  /** Upload a document (with progress). Resolves with it, or the same one added before. */
  add(token: string, file: File, onProgress: (fraction: number) => void): Promise<{ document: Doc; added: boolean }> {
    return new Promise((resolve, reject) => {
      const xhr = new XMLHttpRequest();
      xhr.open("POST", `${GATEWAY}/api/documents?${new URLSearchParams({ name: file.name })}`);
      xhr.setRequestHeader("Authorization", `Bearer ${token}`);
      xhr.setRequestHeader("Content-Type", "application/octet-stream");
      xhr.upload.onprogress = (e) => {
        if (e.lengthComputable) onProgress(e.loaded / e.total);
      };
      xhr.onerror = () => reject(new ApiError(0, "offline", "The upload stopped: Outpost couldn't be reached."));
      xhr.onload = () => {
        let data: { error?: { code?: string; message?: string } } | null = null;
        try {
          data = JSON.parse(xhr.responseText);
        } catch {
          data = null;
        }
        if (xhr.status === 201 || xhr.status === 200) resolve(data as unknown as { document: Doc; added: boolean });
        else reject(new ApiError(xhr.status, data?.error?.code ?? "unknown", data?.error?.message ?? `Upload failed (${xhr.status}).`));
      };
      xhr.send(file);
    });
  },
};

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
  /** The encrypted port phones pair with, when it is running. */
  secure_port: number | null;
  fingerprint: string | null;
  /** Three words from the certificate; a phone shows the same ones. */
  words: string[];
  /** The saved "Allow my Tailscale devices" setting. */
  tailscale_access: boolean;
  /** This PC on Tailscale, when it's connected. */
  tailscale: { ip: string; dns_name: string | null } | null;
  /** Outpost answers on Tailscale right now. */
  tailscale_running: boolean;
  /** Addresses phones can use, best first. */
  phone_hosts: string[];
}

export interface PairingView {
  id: string;
  code: string;
  expires_at: number;
  fingerprint: string;
  words: string[];
  port: number;
  hosts: string[];
  link: string;
  qr_svg: string;
  name: string;
  /** For an invite: who it's for. */
  invite?: { kind: "join" | "reset"; name: string; role: string | null } | null;
}

export interface PairingStatus {
  used: boolean;
  expired: boolean;
  device: string | null;
  account_name: string | null;
}

export interface DeviceView extends Session {
  account_name: string;
  current: boolean;
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
  opts: { token?: string; body?: unknown; pin?: string; base?: string } = {},
): Promise<T> {
  try {
    return await send<T>(method, path, opts);
  } catch (e) {
    // A paired device is asked for its PIN again before risky actions: ask, then retry.
    if (e instanceof ApiError && e.code === "pin_required" && pinPrompter && opts.pin === undefined) {
      const pin = await pinPrompter(e.message);
      if (pin) return request<T>(method, path, { ...opts, pin });
    }
    throw e;
  }
}

async function send<T>(
  method: string,
  path: string,
  opts: { token?: string; body?: unknown; pin?: string; base?: string } = {},
): Promise<T> {
  const headers: Record<string, string> = {};
  if (opts.token) headers.Authorization = `Bearer ${opts.token}`;
  if (opts.body !== undefined) headers["Content-Type"] = "application/json";
  if (opts.pin) headers["X-Outpost-Pin"] = opts.pin;

  let res: Response;
  try {
    res = await fetch((opts.base ?? GATEWAY) + path, {
      method,
      headers,
      body: opts.body === undefined ? undefined : JSON.stringify(opts.body),
    });
  } catch {
    throw new ApiError(0, "offline", SERVER ? `Can't reach ${SERVER.name}.` : "Can't reach the Outpost service.");
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

/** This computer's clock, so the model knows the user's date and time. */
export function clock(): { utc_offset_minutes: number; time_zone?: string } {
  let zone: string | undefined;
  try {
    zone = Intl.DateTimeFormat().resolvedOptions().timeZone;
  } catch {
    zone = undefined;
  }
  return { utc_offset_minutes: -new Date().getTimezoneOffset(), time_zone: zone };
}

/** What the first look at a server found (from the app shell). */
export interface ServerProbe {
  host: string;
  port: number;
  fingerprint: string;
  words: string[];
  health: Health;
}

export interface PairHello {
  name: string;
  version: string;
  accounts: DirectoryEntry[];
  /** An invite: who it's for (the person then picks a PIN). */
  invite?: { kind: "join" | "reset"; name: string; role: string | null } | null;
}

/** Connecting this app to a server: calls go through a trial tunnel (`base`) until it
 * switches over. */
export const serverSetup = {
  async probe(address: string): Promise<ServerProbe> {
    const { invoke } = await import("@tauri-apps/api/core");
    return invoke<ServerProbe>("probe_server", { address });
  },
  async open(p: { host: string; port: number; fingerprint: string }): Promise<string> {
    const { invoke } = await import("@tauri-apps/api/core");
    return invoke<string>("try_server", { host: p.host, port: p.port, fingerprint: p.fingerprint });
  },
  health: (base: string) => request<Health>("GET", "/api/health", { base }),
  setupOwner: (base: string, body: { name: string; secret: string; setup_code: string; device: string }) =>
    request<SignInResponse>("POST", "/api/setup/owner", { base, body }),
  hello: (base: string, code: string) => request<PairHello>("POST", "/api/pair/hello", { base, body: { code } }),
  complete: (base: string, body: { code: string; account_id: string; secret: string; device_name: string }) =>
    request<SignInResponse>("POST", "/api/pair/complete", { base, body }),
  accept: (base: string, body: { code: string; secret: string; device_name: string }) =>
    request<SignInResponse>("POST", "/api/pair/accept", { base, body }),
  me: (base: string, token: string) =>
    request<{ account: Account; session: Session }>("GET", "/api/me", { base, token }),
  /** Use this server from now on, signed in as this person; the app restarts into it. */
  async switchTo(p: {
    host: string;
    port: number;
    fingerprint: string;
    name: string;
    token: string | null;
    person: Person | null;
  }) {
    const { invoke } = await import("@tauri-apps/api/core");
    await invoke("save_server", p);
    const { relaunch } = await import("@tauri-apps/plugin-process");
    await relaunch();
  },
  /** Go back to running Outpost on this PC; the app restarts. The server stays saved
   * (sign-ins and all), so switching back later needs no pairing. */
  async useThisPc() {
    const { invoke } = await import("@tauri-apps/api/core");
    await invoke("forget_server", { keep: true });
    const { relaunch } = await import("@tauri-apps/plugin-process");
    await relaunch();
  },
  /** Use the saved server again; the app restarts, still signed in. */
  async goBack() {
    const { invoke } = await import("@tauri-apps/api/core");
    await invoke("use_saved_server");
    const { relaunch } = await import("@tauri-apps/plugin-process");
    await relaunch();
  },
  /** Forget the saved server completely (connecting again then means pairing again). */
  async forgetSaved() {
    const { invoke } = await import("@tauri-apps/api/core");
    await invoke("forget_server", { keep: false });
    SAVED_SERVER = null;
  },
};

export const api = {
  health: () => request<Health>("GET", "/api/health"),
  me: (token: string) => request<{ account: Account; session: Session }>("GET", "/api/me", { token }),
  /** Is this the PIN of the account `token` belongs to? (Picking an account on a shared PC.) */
  checkPin: (token: string, pin: string) => send<void>("POST", "/api/me/check-pin", { token, pin }),
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
  /** My own copy of a persona (one someone shared, or mine). */
  copyPersona: (token: string, id: string) => request<Persona>("POST", `/api/personas/${encodeURIComponent(id)}/copy`, { token }),
  /** Everyone using this Outpost (names only). */
  people: (token: string) => request<DirectoryEntry[]>("GET", "/api/people", { token }),
  /** Send copies of a persona (prompt and settings; none of my memories). */
  sendPersona: (token: string, id: string, accountIds: string[]) =>
    request<{ account_name: string; persona_name: string }[]>("POST", `/api/personas/${encodeURIComponent(id)}/send`, {
      token,
      body: { account_ids: accountIds },
    }),
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
  /** Deletes a message (yours take their reply too) and what was learned from it. */
  deleteMessage: (token: string, chatId: string, messageId: string) =>
    request<{ removed: string[]; forgotten: number }>(
      "DELETE",
      `/api/chats/${encodeURIComponent(chatId)}/messages/${encodeURIComponent(messageId)}`,
      { token },
    ),
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
  setNetwork: (token: string, lanAccess: boolean, tailscaleAccess?: boolean) =>
    request<NetworkStatus>("PUT", "/api/settings/network", {
      token,
      body: { lan_access: lanAccess, tailscale_access: tailscaleAccess ?? null },
    }),
  createPairing: (token: string) => request<PairingView>("POST", "/api/pairing", { token }),
  /** Invite someone new (`name`, `role`), or send a PIN reset (`account_id`). */
  createInvite: (token: string, body: { name?: string; role?: Role; account_id?: string }) =>
    request<PairingView>("POST", "/api/invites", { token, body }),
  pairingStatus: (token: string, id: string) =>
    request<PairingStatus>("GET", `/api/pairing/${encodeURIComponent(id)}`, { token }),
  devices: (token: string, all = false) => request<DeviceView[]>("GET", `/api/devices${all ? "?all=true" : ""}`, { token }),
  revokeDevice: (token: string, id: string) => request<void>("DELETE", `/api/devices/${encodeURIComponent(id)}`, { token }),
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
      body: JSON.stringify({ content, images, ...clock() }),
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

// ---------------------------------------------------------------- Create

export type Lane = "text_to_image" | "edit_image" | "text_to_video" | "image_to_video" | "tool";
export type InputKind = "text" | "size" | "integer" | "number" | "seed" | "choice" | "image" | "lora";

export interface PackInput {
  key: string;
  label: string;
  kind: InputKind;
  help: string | null;
  default: unknown;
  required: boolean;
  options: string[];
  min: number | null;
  max: number | null;
  step: number | null;
  advanced: boolean;
}

export interface PackSetup {
  running: boolean;
  current: string | null;
  done_bytes: number;
  total_bytes: number;
  percent: number | null;
  error: string | null;
  file_index: number;
  file_count: number;
  speed_bps: number | null;
  eta_secs: number | null;
  finished_at: number | null;
}

/** The Create tabs. */
/** The Create tabs, plus the picture tools (upscale, cutout) run from the gallery. */
export type Task = "picture" | "character" | "edit" | "video" | "animate" | "upscale" | "cutout";

export interface Pack {
  id: string;
  name: string;
  lane: Lane;
  description: string;
  license: string | null;
  vram_gb: number;
  output: { node: string; kind: "image" | "video" };
  inputs: PackInput[];
  models: {
    ready: boolean;
    files: { file: string; folder: string; size_gb: number; present: boolean }[];
    missing_gb: number;
    setup: PackSetup | null;
  };
  fit: Fit;
  task: Task;
  /** Higher is better within a task. */
  rank: number;
  /** Outpost's pick for its task on this PC. */
  recommended: boolean;
  /** ComfyUI node types it needs that this ComfyUI lacks: update ComfyUI to use it. */
  missing_nodes: string[];
  /** The helper PC that runs it (this server can't): "Runs on Dan's PC". */
  runs_on?: string | null;
}

export interface Download {
  id: string;
  kind: "model" | "workflow";
  name: string;
  detail: string | null;
  part: number;
  parts: number | null;
  done_bytes: number;
  total_bytes: number;
  percent: number | null;
  speed_bps: number | null;
  eta_secs: number | null;
  running: boolean;
  error: string | null;
  finished_at: number | null;
}

export type JobStatus = "queued" | "running" | "done" | "failed" | "cancelled";

export interface Media {
  id: string;
  account_id: string;
  job_id: string | null;
  kind: "image" | "video";
  mime: string;
  bytes: number;
  pack_id: string | null;
  prompt: string | null;
  seed: number | null;
  created_at: number;
  /** In the shared gallery since; null while private. */
  shared_at: number | null;
}

/** A shared-gallery item, with who shared it. */
export interface SharedMedia extends Media {
  by_name: string;
  by_color: string;
}

export interface Job {
  id: string;
  account_id: string;
  pack_id: string;
  inputs: string;
  status: JobStatus;
  error: string | null;
  seed: number | null;
  created_at: number;
  started_at: number | null;
  finished_at: number | null;
  position: number | null;
  progress: { phase: string; step: number; steps: number; percent: number | null } | null;
  media: Media[];
  /** The helper PC running it, if not the server. */
  on?: string | null;
  /** Queued for a helper PC that's off or paused: its name. */
  waiting_for?: string | null;
}

export const downloadsApi = {
  list: (token: string) => request<{ items: Download[]; running: number }>("GET", "/api/downloads", { token }),
};

/** One job in the shared line for the GPU. Other people's show only whose and how far. */
export interface LineEntry {
  mine: boolean;
  job_id: string | null;
  pack_id: string | null;
  by_name: string;
  by_color: string;
  status: JobStatus;
  progress: Job["progress"];
  eta_secs: number | null;
  /** The helper PC running it, if not the server. */
  on?: string | null;
  /** Queued for a helper PC that's off or paused: its name. */
  waiting_for?: string | null;
}

export interface LoraUploaded {
  file: string;
  bytes: number;
  character: Character | null;
}

/** Upload a LoRA file (with progress, which fetch can't report). */
export function uploadLora(
  token: string,
  file: File,
  opts: { replace?: boolean; character?: string; trigger?: string; subject?: string },
  onProgress: (fraction: number) => void,
): Promise<LoraUploaded> {
  const q = new URLSearchParams({ name: file.name });
  if (opts.replace) q.set("replace", "true");
  if (opts.character) q.set("character", opts.character);
  if (opts.trigger) q.set("trigger", opts.trigger);
  if (opts.subject) q.set("subject", opts.subject);
  return new Promise((resolve, reject) => {
    const xhr = new XMLHttpRequest();
    xhr.open("POST", `${GATEWAY}/api/loras?${q}`);
    xhr.setRequestHeader("Authorization", `Bearer ${token}`);
    xhr.setRequestHeader("Content-Type", "application/octet-stream");
    xhr.upload.onprogress = (e) => {
      if (e.lengthComputable) onProgress(e.loaded / e.total);
    };
    xhr.onerror = () => reject(new ApiError(0, "offline", "The upload stopped: Outpost couldn't be reached."));
    xhr.onload = () => {
      let data: { error?: { code?: string; message?: string } } | null = null;
      try {
        data = JSON.parse(xhr.responseText);
      } catch {
        data = null;
      }
      if (xhr.status === 201) resolve(data as unknown as LoraUploaded);
      else reject(new ApiError(xhr.status, data?.error?.code ?? "unknown", data?.error?.message ?? `Upload failed (${xhr.status}).`));
    };
    xhr.send(file);
  });
}

export const createApi = {
  queue: (token: string) => request<LineEntry[]>("GET", "/api/queue", { token }),
  packs: (token: string) => request<Pack[]>("GET", "/api/packs", { token }),
  /** Every workflow, including ones too big for the server's card. */
  packsAll: (token: string) => request<Pack[]>("GET", "/api/packs?all=true", { token }),
  installPack: (token: string, id: string) =>
    request<void>("POST", `/api/packs/${encodeURIComponent(id)}/install`, { token }),
  loras: (token: string) =>
    request<{ loras: string[]; folder: string; characters: Character[] }>("GET", "/api/loras", { token }),
  jobs: (token: string) => request<Job[]>("GET", "/api/jobs?limit=30", { token }),
  createJob: (token: string, pack: string, inputs: Record<string, unknown>) =>
    request<Job>("POST", "/api/jobs", { token, body: { pack, inputs } }),
  job: (token: string, id: string) => request<Job>("GET", `/api/jobs/${encodeURIComponent(id)}`, { token }),
  cancelJob: (token: string, id: string) => request<void>("DELETE", `/api/jobs/${encodeURIComponent(id)}`, { token }),
  /** `accountId`: someone else's gallery (owner only). */
  gallery: (token: string, before?: number, accountId?: string) =>
    request<Media[]>(
      "GET",
      `/api/gallery?limit=60${before ? `&before=${before}` : ""}${accountId ? `&account_id=${encodeURIComponent(accountId)}` : ""}`,
      { token },
    ),
  sharedGallery: (token: string, before?: number) =>
    request<SharedMedia[]>("GET", `/api/gallery/shared?limit=60${before ? `&before=${before}` : ""}`, { token }),
  shareMedia: (token: string, id: string, shared: boolean) =>
    request<Media>("POST", `/api/media/${encodeURIComponent(id)}/share`, { token, body: { shared } }),
  deleteMedia: (token: string, id: string) =>
    request<void>("DELETE", `/api/media/${encodeURIComponent(id)}`, { token }),
};

/** Any authenticated file (attachment or gallery item) as a blob. */
export async function authedBlob(token: string, path: string): Promise<Blob> {
  const res = await fetch(`${GATEWAY}${path}`, { headers: { Authorization: `Bearer ${token}` } }).catch(() => null);
  if (!res || !res.ok) throw new ApiError(res?.status ?? 0, "not_found", "File not found.");
  return res.blob();
}

/** Any authenticated file (attachment or gallery item) as an object URL. */
export async function authedObjectUrl(token: string, path: string): Promise<string> {
  return URL.createObjectURL(await authedBlob(token, path));
}

// ---------------------------------------------------------------- Training Studio

export interface TriggerCheck {
  word: string;
  ok: boolean;
  problem: string | null;
  suggestion: string | null;
}

export interface TrainingStatus {
  toolkit_dir: string;
  /** Why training can't run yet; null when ai-toolkit is ready. */
  toolkit_problem: string | null;
  caption_models: string[];
  character_pack_ready: boolean;
  vram_needed_gb: number;
  vram_gb: number;
  /** Outpost's own install of ai-toolkit. */
  install?: {
    installed: boolean;
    installing: boolean;
    version: string | null;
    progress: { step: string; step_index: number; step_count: number; percent: number | null; detail: string | null } | null;
    error: string | null;
    dir: string;
    pinned_version: string;
    log_tail: string[];
  };
}

export interface Dataset {
  id: string;
  account_id: string;
  name: string;
  trigger_word: string;
  created_at: number;
  updated_at: number;
  image_count: number;
  captioned_count: number;
}

export interface DatasetImage {
  id: string;
  dataset_id: string;
  mime: string;
  bytes: number;
  caption: string;
  created_at: number;
}

export interface CaptionRun {
  running: boolean;
  done: number;
  total: number;
  failed: number;
  model: string;
  error: string | null;
}

export interface Checkpoint {
  step: number;
  /** Empty for step 0: the pictures drawn before training. */
  file: string;
  samples: string[];
}

export interface TrainingView {
  id: string;
  account_id: string;
  dataset_id: string;
  job_id: string;
  trigger_word: string;
  steps: number;
  image_count: number;
  created_at: number;
  status: JobStatus;
  error: string | null;
  started_at: number | null;
  finished_at: number | null;
  position: number | null;
  progress: Job["progress"];
  eta_secs: number | null;
  checkpoints: Checkpoint[];
  chosen_step: number | null;
  /** The test prompts, in the order of each checkpoint's pictures. */
  sample_prompts: string[];
}

export interface DatasetView extends Dataset {
  trigger_check: TriggerCheck;
  images: DatasetImage[];
  captioning: CaptionRun | null;
  /** Test prompts made from the captions. */
  suggested_prompts: string[];
  trainings: TrainingView[];
}

export interface Character {
  id: string;
  account_id: string;
  name: string;
  trigger_word: string;
  lora_file: string;
  training_id: string | null;
  step: number | null;
  created_at: number;
  /** What it is ("woman", "dog") and how it's drawn, from its captions. */
  subject: string | null;
  style: string | null;
}

/** A file in the LoRA folder and what it says about itself. */
export interface LoraFile {
  file: string;
  /** Made for Z-Image, so the Characters workflow can use it. */
  z_image: boolean;
  base: string | null;
  /** Its trigger word, when its trainer recorded it. */
  trigger: string | null;
  trained_as: string | null;
  /** The character already using it. */
  character: string | null;
}

const ds = (id: string) => `/api/datasets/${encodeURIComponent(id)}`;

export const trainApi = {
  status: (token: string) => request<TrainingStatus>("GET", "/api/training", { token }),
  installToolkit: (token: string) => request<void>("POST", "/api/training/install", { token }),
  setToolkitDir: (token: string, dir: string) =>
    request<TrainingStatus>("PUT", "/api/training", { token, body: { toolkit_dir: dir } }),
  checkTrigger: (token: string, word: string) =>
    request<TriggerCheck>("GET", `/api/training/trigger-check?word=${encodeURIComponent(word)}`, { token }),
  datasets: (token: string) => request<Dataset[]>("GET", "/api/datasets", { token }),
  createDataset: (token: string, name: string, trigger?: string) =>
    request<DatasetView>("POST", "/api/datasets", { token, body: { name, trigger_word: trigger ?? null } }),
  dataset: (token: string, id: string) => request<DatasetView>("GET", ds(id), { token }),
  updateDataset: (token: string, id: string, body: { name?: string; trigger_word?: string }) =>
    request<DatasetView>("PATCH", ds(id), { token, body }),
  deleteDataset: (token: string, id: string) => request<void>("DELETE", ds(id), { token }),
  addImage: (token: string, id: string, image: string, caption = "") =>
    request<DatasetImage>("POST", `${ds(id)}/images`, { token, body: { image, caption } }),
  importFolder: (token: string, id: string, folder: string) =>
    request<{ added: number; skipped: string[] }>("POST", `${ds(id)}/import`, { token, body: { folder } }),
  setCaption: (token: string, id: string, imageId: string, caption: string) =>
    request<DatasetImage>("PATCH", `${ds(id)}/images/${encodeURIComponent(imageId)}`, { token, body: { caption } }),
  deleteImage: (token: string, id: string, imageId: string) =>
    request<void>("DELETE", `${ds(id)}/images/${encodeURIComponent(imageId)}`, { token }),
  caption: (token: string, id: string, body: { model?: string; overwrite?: boolean }) =>
    request<CaptionRun>("POST", `${ds(id)}/caption`, { token, body }),
  train: (token: string, id: string, steps: number, samplePrompts?: string[]) =>
    request<TrainingView>("POST", `${ds(id)}/train`, { token, body: { steps, sample_prompts: samplePrompts ?? null } }),
  training: (token: string, id: string) => request<TrainingView>("GET", `/api/trainings/${encodeURIComponent(id)}`, { token }),
  useCheckpoint: (token: string, id: string, step: number, name?: string) =>
    request<Character>("POST", `/api/trainings/${encodeURIComponent(id)}/use`, { token, body: { step, name: name ?? null } }),
  characters: (token: string) => request<Character[]>("GET", "/api/characters", { token }),
  loraFiles: (token: string) => request<LoraFile[]>("GET", "/api/characters/lora-files", { token }),
  addCharacter: (
    token: string,
    body: { lora_file: string; name: string; trigger_word: string; subject?: string; style?: string },
  ) => request<Character>("POST", "/api/characters", { token, body }),
  renameCharacter: (token: string, id: string, name: string) =>
    request<Character>("PATCH", `/api/characters/${encodeURIComponent(id)}`, { token, body: { name } }),
  deleteCharacter: (token: string, id: string) =>
    request<void>("DELETE", `/api/characters/${encodeURIComponent(id)}`, { token }),
  imagePath: (datasetId: string, imageId: string) => `${ds(datasetId)}/images/${encodeURIComponent(imageId)}`,
  samplePath: (trainingId: string, file: string) =>
    `/api/trainings/${encodeURIComponent(trainingId)}/samples/${encodeURIComponent(file)}`,
};

/** A chat reply that started a picture carries `[[outpost-job:<id>]]`, one per picture. */
export const JOB_CARD = /\[\[outpost-job:([0-9a-f-]{36})\]\]/;

/** The job ids in a chat reply, in order. */
export function jobCards(content: string): string[] {
  return [...content.matchAll(/\[\[outpost-job:([0-9a-f-]{36})\]\]/g)].map((m) => m[1]);
}

// ---------------------------------------------------------------- API, MCP and webhooks

export interface Webhook {
  id: string;
  account_id: string;
  url: string;
  events: string[];
  created_at: number;
  last_at: number | null;
  /** HTTP status of the last delivery; 0 when it couldn't connect. */
  last_status: number | null;
  last_error: string | null;
}

export const hooksApi = {
  list: (token: string) => request<{ webhooks: Webhook[]; events: string[] }>("GET", "/api/webhooks", { token }),
  create: (token: string, url: string, events: string[]) =>
    request<{ webhook: Webhook; secret: string }>("POST", "/api/webhooks", { token, body: { url, events } }),
  remove: (token: string, id: string) => request<void>("DELETE", `/api/webhooks/${encodeURIComponent(id)}`, { token }),
  test: (token: string, id: string) =>
    request<{ status: number; error: string | null }>("POST", `/api/webhooks/${encodeURIComponent(id)}/test`, { token }),
};

export interface McpTool {
  name: string;
  title?: string;
  description: string;
}

export interface CrashSummary {
  id: string;
  /** "crash" (an error Outpost caught) or "unclean" (it didn't close normally). */
  kind: "crash" | "unclean" | string;
  /** Unix seconds. */
  at: number;
  summary: string;
}

export interface Diagnostics {
  logs_dir: string;
  crash_dir: string;
  crashes: CrashSummary[];
  unseen: number;
}

export interface Release {
  version: string;
  notes: string | null;
  date: string | null;
}

export interface UpdateStatus {
  current: string;
  enabled: boolean;
  repo: string;
  releases_page: string;
  checked_at: number | null;
  update: Release | null;
  mobile: { version: string; url: string } | null;
  /** A server that can install `update` itself. */
  server_can_install?: boolean;
  /** A server install in progress, or "Failed: ..." */
  installing?: string | null;
  error: string | null;
}

export const systemApi = {
  diagnostics: (token: string) => request<Diagnostics>("GET", "/api/diagnostics", { token }),
  markSeen: (token: string) => request<void>("POST", "/api/diagnostics/seen", { token }),
  crash: (token: string, id: string) => request<{ id: string; text: string }>("GET", `/api/diagnostics/crashes/${encodeURIComponent(id)}`, { token }),
  deleteCrash: (token: string, id: string) => request<void>("DELETE", `/api/diagnostics/crashes/${encodeURIComponent(id)}`, { token }),
  deleteAllCrashes: (token: string) => request<void>("DELETE", "/api/diagnostics/crashes", { token }),
  recentLog: (token: string) => request<{ text: string }>("GET", "/api/diagnostics/log", { token }),
  updates: (token: string) => request<UpdateStatus>("GET", "/api/updates", { token }),
  checkUpdates: (token: string) => request<UpdateStatus>("POST", "/api/updates/check", { token }),
  setUpdates: (token: string, enabled: boolean) => request<UpdateStatus>("PUT", "/api/updates/settings", { token, body: { enabled } }),
  /** Install the newest release on the server and restart it (a server only). */
  installServer: (token: string) => request<void>("POST", "/api/updates/install", { token }),
  /** Restart Outpost on the server (a server only). */
  restartServer: (token: string) => request<void>("POST", "/api/system/restart", { token }),
};

/** Wait for the server to come back after a restart (up to two minutes). */
export async function waitForServer(): Promise<Health | null> {
  // Give it a moment to go down first.
  await new Promise((r) => setTimeout(r, 3000));
  for (let i = 0; i < 60; i++) {
    try {
      return await api.health();
    } catch {
      await new Promise((r) => setTimeout(r, 2000));
    }
  }
  return null;
}

export const integrationsApi = {
  get: (token: string) => request<{ port: number; bridge_path: string | null }>("GET", "/api/integrations", { token }),
  /** The OpenAPI document (no sign-in needed). */
  openapi: () => request<OpenApiDoc>("GET", "/openapi.json"),
  /** The MCP tools, asked over MCP itself. */
  mcpTools: async (token: string) => {
    const r = await request<{ result?: { tools: McpTool[] } }>("POST", "/mcp", {
      token,
      body: { jsonrpc: "2.0", id: 1, method: "tools/list", params: {} },
    });
    return r.result?.tools ?? [];
  },
};

/** The parts of an OpenAPI 3 document the API page shows. */
export interface OpenApiDoc {
  info: { title: string; version: string };
  tags?: { name: string; description?: string }[];
  paths: Record<
    string,
    Record<
      string,
      {
        tags?: string[];
        summary?: string;
        description?: string;
        parameters?: { name: string; in: string; required?: boolean; description?: string }[];
        requestBody?: { content?: Record<string, { schema?: { $ref?: string } }> };
      }
    >
  >;
}

// ---------------------------------------------------------------- Voice

export interface VoiceInfo {
  id: string;
  name: string;
  language: string;
  gender: string;
}

export interface VoiceStatus {
  state: EngineState;
  ready: boolean;
  voices: VoiceInfo[];
  default_voice: string;
}

export const voiceApi = {
  status: (token: string) => request<VoiceStatus>("GET", "/api/voice", { token }),
  transcribe: (token: string, audio: string, language?: string) =>
    request<{ text: string; language: string; duration: number }>("POST", "/api/voice/transcribe", {
      token,
      body: { audio, language: language ?? null },
    }),
  /** Speak text; the reply is WAV audio. */
  speak: async (token: string, text: string, opts: { voice?: string; personaId?: string } = {}): Promise<Blob> => {
    let res: Response;
    try {
      res = await fetch(`${GATEWAY}/api/voice/speak`, {
        method: "POST",
        headers: { Authorization: `Bearer ${token}`, "Content-Type": "application/json" },
        body: JSON.stringify({ text, voice: opts.voice ?? null, persona_id: opts.personaId ?? null }),
      });
    } catch {
      throw new ApiError(0, "offline", "Can't reach the Outpost service.");
    }
    if (!res.ok) {
      const e = (await res.json().catch(() => null))?.error ?? {};
      throw new ApiError(res.status, e.code ?? "unknown", e.message ?? "Couldn't speak that.");
    }
    return res.blob();
  },
};
