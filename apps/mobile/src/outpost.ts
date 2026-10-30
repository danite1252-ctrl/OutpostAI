/**
 * Talking to the paired Outpost: every request goes over its encrypted port and only
 * to the certificate pinned at pairing (see modules/outpost-net).
 */
import OutpostNet, { type NativeResponse } from "../modules/outpost-net";

export interface Connection {
  /** Addresses to try, best first (the PC's address can change). */
  hosts: string[];
  port: number;
  fingerprint: string;
  /** The Outpost's name, for finding it again on the network. */
  name: string;
  token?: string;
}

export class ApiError extends Error {
  constructor(
    public status: number,
    public code: string,
    message: string,
    public attemptsLeft?: number,
    public retryAfter?: number,
  ) {
    super(message);
  }
}

function parseError(r: NativeResponse): ApiError {
  try {
    const e = JSON.parse(r.body)?.error ?? {};
    return new ApiError(r.status, e.code ?? "unknown", e.message ?? `Outpost said no (${r.status}).`, e.attempts_left, e.retry_after);
  } catch {
    return new ApiError(r.status, "unknown", `Outpost said no (${r.status}).`);
  }
}

let streamIds = 0;

export interface StreamHandlers {
  event: (name: string, data: unknown) => void;
  done: () => void;
  error: (e: ApiError) => void;
}

export class Outpost {
  constructor(public conn: Connection) {}

  /** Whether the Outpost answered the last request; screens show a banner when not. */
  online = true;
  private watchers = new Set<(online: boolean) => void>();

  onStatus(fn: (online: boolean) => void): () => void {
    this.watchers.add(fn);
    return () => this.watchers.delete(fn);
  }

  private setOnline(on: boolean) {
    if (on === this.online) return;
    this.online = on;
    this.watchers.forEach((fn) => fn(on));
  }

  /** A cheap request to see whether the Outpost is back. */
  async ping(): Promise<boolean> {
    try {
      await this.send("GET", "/api/health", undefined, { timeoutMs: 6000 });
      return true;
    } catch {
      return false;
    }
  }

  private base(host = this.conn.hosts[0]) {
    return `https://${host}:${this.conn.port}`;
  }

  private headers(extra?: Record<string, string>) {
    const h: Record<string, string> = { Accept: "application/json", ...extra };
    if (this.conn.token) h.Authorization = `Bearer ${this.conn.token}`;
    return h;
  }

  /** One request, trying each known address until one answers. */
  private async send(
    method: string,
    path: string,
    body?: unknown,
    opts: { headers?: Record<string, string>; base64?: boolean; timeoutMs?: number } = {},
  ): Promise<NativeResponse> {
    let last: unknown;
    for (const host of this.conn.hosts) {
      try {
        const r = await OutpostNet.request({
          url: this.base(host) + path,
          method,
          headers: { ...this.headers(opts.headers), ...(body !== undefined ? { "Content-Type": "application/json" } : {}) },
          body: body === undefined ? null : JSON.stringify(body),
          fingerprint: this.conn.fingerprint,
          base64: opts.base64,
          timeoutMs: opts.timeoutMs ?? 30000,
        });
        // The address that answered goes first next time.
        if (host !== this.conn.hosts[0]) this.conn.hosts = [host, ...this.conn.hosts.filter((h) => h !== host)];
        this.setOnline(true);
        return r;
      } catch (e) {
        last = e;
      }
    }
    const msg = last instanceof Error ? last.message : "";
    if (/certificate/i.test(msg)) throw new ApiError(0, "wrong_outpost", msg);
    this.setOnline(false);
    throw new ApiError(0, "offline", "Can't reach your Outpost. Is the phone on the same Wi-Fi, and the PC on?");
  }

  async json<T>(method: string, path: string, body?: unknown, headers?: Record<string, string>): Promise<T> {
    const r = await this.send(method, path, body, { headers });
    if (r.status < 200 || r.status >= 300) throw parseError(r);
    if (r.status === 204 || !r.body) return undefined as T;
    return JSON.parse(r.body) as T;
  }

  get<T>(path: string) {
    return this.json<T>("GET", path);
  }

  /** A picture (gallery item, chat attachment) as a data URI for <Image>. */
  async image(path: string): Promise<string> {
    const r = await this.send("GET", path, undefined, { base64: true, timeoutMs: 60000 });
    if (r.status !== 200) throw parseError(r);
    const mime = (r.contentType || "image/png").split(";")[0];
    return `data:${mime};base64,${r.body}`;
  }

  /** A file (a gallery video) saved on the phone, for players that read from disk. */
  async download(path: string, name: string): Promise<string> {
    try {
      return await OutpostNet.download(
        { url: this.base() + path, method: "GET", headers: this.headers(), fingerprint: this.conn.fingerprint },
        name,
      );
    } catch (e) {
      if (!(e instanceof Error && /said no/.test(e.message))) this.setOnline(false);
      throw new ApiError(0, "offline", e instanceof Error ? e.message : "Couldn't download that.");
    }
  }

  /** Say `text` in a persona's voice on the PC; resolves with a WAV file on the phone. */
  async speech(text: string, personaId: string | null, name: string): Promise<string> {
    let last: unknown;
    for (const host of this.conn.hosts) {
      try {
        return await OutpostNet.download(
          {
            url: this.base(host) + "/api/voice/speak",
            method: "POST",
            headers: { ...this.headers({ Accept: "audio/wav" }), "Content-Type": "application/json" },
            body: JSON.stringify({ text, persona_id: personaId }),
            fingerprint: this.conn.fingerprint,
          },
          name,
        );
      } catch (e) {
        last = e;
        // Outpost answered (just not with a voice): no point trying the other addresses.
        if (e instanceof Error && /said no/.test(e.message)) break;
      }
    }
    const msg = last instanceof Error ? last.message : "";
    if (/said no/.test(msg)) throw new ApiError(0, "voice", "Your PC couldn't speak that. Is voice set up in Outpost on the PC (Settings → Engines)?");
    this.setOnline(false);
    throw new ApiError(0, "offline", "Can't reach your Outpost to speak the reply.");
  }

  /** What the PC's Whisper heard in a recording (base64). */
  async transcribe(audio: string, mime = "audio/mp4", language?: string): Promise<{ text: string; language: string }> {
    // Whisper on the PC's processor can take a while on a long recording.
    const r = await this.send("POST", "/api/voice/transcribe", { audio: `data:${mime};base64,${audio}`, language }, { timeoutMs: 120000 });
    if (r.status < 200 || r.status >= 300) throw parseError(r);
    return JSON.parse(r.body) as { text: string; language: string };
  }

  /** A Server-Sent Events reply. Returns a function that stops following it. */
  stream(path: string, body: unknown, on: StreamHandlers): () => void {
    const id = `s${++streamIds}-${Date.now()}`;
    let name = "message";
    let data: string[] = [];
    let ended = false;
    const finish = () => {
      ended = true;
      subs.forEach((s) => s.remove());
    };
    const subs = [
      OutpostNet.addListener("onChunk", (e) => {
        if (e.id !== id) return;
        const line = e.line;
        if (line === "") {
          if (data.length) {
            try {
              on.event(name, JSON.parse(data.join("\n")));
            } catch {
              // Not JSON: ignore (keep-alives).
            }
          }
          name = "message";
          data = [];
        } else if (line.startsWith("event:")) name = line.slice(6).trim();
        else if (line.startsWith("data:")) data.push(line.slice(5).replace(/^ /, ""));
      }),
      OutpostNet.addListener("onEnd", (e) => {
        if (e.id !== id) return;
        finish();
        if (e.status >= 400) on.error(parseError({ status: e.status, body: e.body, contentType: "" }));
        else on.done();
      }),
      OutpostNet.addListener("onError", (e) => {
        if (e.id !== id) return;
        finish();
        this.setOnline(false);
        on.error(new ApiError(0, "offline", e.message));
      }),
    ];
    OutpostNet.stream(id, {
      url: this.base() + path,
      method: "POST",
      headers: { ...this.headers({ Accept: "text/event-stream" }), "Content-Type": "application/json" },
      body: JSON.stringify(body),
      fingerprint: this.conn.fingerprint,
    });
    return () => {
      if (!ended) OutpostNet.cancel(id);
    };
  }
}

// ------------------------------------------------------------------ types

export type Role = "owner" | "member" | "restricted" | "service";

export interface Account {
  id: string;
  name: string;
  role: Role;
  avatar_color: string;
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

export interface Persona {
  id: string;
  slug: string;
  name: string;
  avatar_color: string;
  model: string | null;
  /** Speaking voice on the PC (Kokoro); null = Outpost's default. */
  voice?: string | null;
  /** Read replies aloud. */
  speak_replies?: boolean;
}

export interface Chat {
  id: string;
  persona_id: string | null;
  title: string;
  updated_at: number;
}

export interface Attachment {
  id: string;
  mime: string;
}

export interface Message {
  id: string;
  role: "user" | "assistant" | "system";
  content: string;
  thinking: string | null;
  attachments: Attachment[];
}

export type Task = "picture" | "character" | "edit" | "video" | "animate" | "upscale" | "cutout";
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

export interface Pack {
  id: string;
  name: string;
  description: string;
  task: Task;
  rank: number;
  recommended: boolean;
  fit: "fits" | "partial" | "too_large";
  vram_gb: number;
  inputs: PackInput[];
  models: { ready: boolean; missing_gb: number };
  missing_nodes: string[];
}

export interface Media {
  id: string;
  account_id: string;
  kind: "image" | "video";
  mime: string;
  prompt: string | null;
  seed: number | null;
  pack_id: string | null;
  created_at: number;
  shared_at: number | null;
  by_name?: string;
}

export interface Progress {
  phase: string;
  step: number;
  steps: number;
  percent: number | null;
}

export interface Job {
  id: string;
  pack_id: string;
  inputs: string;
  status: "queued" | "running" | "done" | "failed" | "cancelled";
  error: string | null;
  position: number | null;
  progress: Progress | null;
  finished_at: number | null;
  media: Media[];
}

export interface LineEntry {
  mine: boolean;
  job_id: string | null;
  pack_id: string | null;
  by_name: string;
  by_color: string;
  status: Job["status"];
  progress: Progress | null;
  eta_secs: number | null;
}

export interface HelloResponse {
  name: string;
  version: string;
  accounts: Account[];
  /** An invite: who it's for. The person picks a PIN instead of choosing an account. */
  invite?: { kind: "join" | "reset"; name: string; role: string | null } | null;
}

export function fraction(p: Progress | null): number | null {
  if (!p) return null;
  if (p.percent != null) return Math.min(1, Math.max(0, p.percent / 100));
  if (p.steps > 0) return Math.min(1, p.step / p.steps);
  return null;
}

export function eta(secs: number) {
  if (secs < 60) return `${Math.max(1, Math.round(secs))} s left`;
  const m = Math.round(secs / 60);
  return m < 60 ? `${m} min left` : `${Math.floor(m / 60)} h ${m % 60} min left`;
}

/** This phone's clock, so the model knows the user's date and time. */
export function clock(): { utc_offset_minutes: number; time_zone?: string } {
  let zone: string | undefined;
  try {
    zone = Intl.DateTimeFormat().resolvedOptions().timeZone;
  } catch {
    zone = undefined;
  }
  return { utc_offset_minutes: -new Date().getTimezoneOffset(), time_zone: zone };
}
