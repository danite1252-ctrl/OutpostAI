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
