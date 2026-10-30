// Using an Outpost server (a home server with no screen) instead of Outpost on this PC.
//
// Connecting: type the server's address → compare its three check words with the ones the
// server shows → then either set it up (a new server: its setup code, your name and PIN)
// or pair with it (a code from "Pair a device" on something already paired, then your
// account and PIN). The app then restarts as a paired device of the server.

import { useEffect, useRef, useState, type FormEvent } from "react";
import {
  api,
  ApiError,
  forgetPerson,
  GATEWAY,
  saveServerToken,
  SERVER,
  systemApi,
  waitForServer,
  secretProblem,
  serverSetup,
  setPinPrompter,
  type DirectoryEntry,
  type PairHello,
  type Person,
  type Persona,
  type ServerProbe,
  type SignInResponse,
} from "../api";
import { Avatar, Button, ErrorNote, Field } from "./ui";

function message(err: unknown) {
  if (err instanceof ApiError) return err.message;
  if (typeof err === "string") return err;
  return err instanceof Error ? err.message : "Something went wrong.";
}

type Step =
  | { kind: "address" }
  | { kind: "check"; probe: ServerProbe }
  | { kind: "setup"; probe: ServerProbe; base: string }
  | { kind: "code"; probe: ServerProbe; base: string }
  | { kind: "account"; probe: ServerProbe; base: string; code: string; hello: PairHello }
  | { kind: "pin"; probe: ServerProbe; base: string; code: string; account: DirectoryEntry }
  | { kind: "newpin"; probe: ServerProbe; base: string; code: string; invite: NonNullable<PairHello["invite"]> }
  | { kind: "switching" };

function Words({ words }: { words: string[] }) {
  return (
    <p className="flex flex-wrap justify-center gap-3 rounded-2xl border border-line bg-surface px-5 py-4 font-mono text-xl text-ink">
      {words.map((w) => (
        <span key={w}>{w}</span>
      ))}
    </p>
  );
}

/**
 * Connect this app to an Outpost server. With `again`, the app already uses that server
 * but lost its sign-in (unpaired, or the server was reset): it skips the address and words.
 */
export function ConnectServer({
  onCancel,
  again,
}: {
  onCancel?: () => void;
  again?: { probe: ServerProbe; base: string };
}) {
  const [step, setStep] = useState<Step>(() =>
    again
      ? again.probe.health.setup_required
        ? { kind: "setup", ...again }
        : { kind: "code", ...again }
      : { kind: "address" },
  );
  const [address, setAddress] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [deviceName, setDeviceName] = useState("Outpost desktop");

  async function run(f: () => Promise<void>) {
    setBusy(true);
    setError(null);
    try {
      await f();
    } catch (e) {
      setError(message(e));
    } finally {
      setBusy(false);
    }
  }

  const finish = (probe: ServerProbe, signed: SignInResponse) =>
    run(async () => {
      setStep({ kind: "switching" });
      await serverSetup.switchTo({
        host: probe.host,
        port: probe.port,
        fingerprint: probe.fingerprint,
        name: probe.health.name,
        token: signed.token,
        person: {
          account_id: signed.account.id,
          name: signed.account.name,
          avatar_color: signed.account.avatar_color,
          token: signed.token,
        },
      });
    });

  const title =
    step.kind === "setup" ? "Set up the server" : step.kind === "address" || step.kind === "check" ? "Connect to an Outpost server" : "Sign in to the server";

  return (
    <main className="flex h-full items-center justify-center overflow-auto bg-deep p-8">
      <div className="flex w-full max-w-md flex-col gap-5">
        <h1 className="text-3xl font-semibold tracking-tight">{title}</h1>

        {step.kind === "address" && (
          <form
            className="flex flex-col gap-5"
            noValidate
            onSubmit={(e) => {
              e.preventDefault();
              void run(async () => setStep({ kind: "check", probe: await serverSetup.probe(address) }));
            }}
          >
            <p className="text-muted">
              Use Outpost on another machine, like a home server, instead of this PC. Chats, pictures and
              models then live on the server, and this app becomes one of its paired devices.
            </p>
            <Field
              label="Server address"
              value={address}
              onChange={(e) => setAddress(e.target.value)}
              placeholder="192.168.1.98"
              autoFocus
              hint="Its address on your network. The encrypted port 7421 is used unless you add another (host:port)."
            />
            <ErrorNote>{error}</ErrorNote>
            <div className="flex gap-3">
              <Button type="submit" disabled={busy || !address.trim()}>
                {busy ? "Looking…" : "Find it"}
              </Button>
              {onCancel && (
                <Button type="button" variant="ghost" onClick={onCancel}>
                  Cancel
                </Button>
              )}
            </div>
          </form>
        )}

        {step.kind === "check" && (
          <div className="flex flex-col gap-5">
            <p className="text-muted">
              Found <b className="text-ink">{step.probe.health.name}</b> (Outpost {step.probe.health.version}) at{" "}
              {step.probe.host}. Check that it shows these same three words:
            </p>
            <Words words={step.probe.words} />
            <p className="text-sm text-muted">
              {step.probe.health.setup_required ? (
                <>
                  On the server, run <code className="text-ink">outpost-gateway --setup-code</code>. It shows the
                  words and the setup code.
                </>
              ) : (
                <>
                  They're under <b className="text-ink">Connections</b> on a device already using this server,
                  and next to every pairing code.
                </>
              )}{" "}
              Different words mean something else is answering at that address, so don't go on.
            </p>
            <ErrorNote>{error}</ErrorNote>
            <div className="flex gap-3">
              <Button
                disabled={busy}
                onClick={() =>
                  void run(async () => {
                    const base = await serverSetup.open(step.probe);
                    const health = await serverSetup.health(base);
                    const probe = { ...step.probe, health };
                    setStep(health.setup_required ? { kind: "setup", probe, base } : { kind: "code", probe, base });
                  })
                }
              >
                They match
              </Button>
              <Button variant="ghost" onClick={() => setStep({ kind: "address" })}>
                They don't
              </Button>
            </div>
          </div>
        )}

        {step.kind === "setup" && (
          <SetupServer
            probe={step.probe}
            base={step.base}
            deviceName={deviceName}
            onDeviceName={setDeviceName}
            onDone={(signed) => void finish(step.probe, signed)}
          />
        )}

        {step.kind === "code" && (
          <form
            className="flex flex-col gap-5"
            noValidate
            onSubmit={(e) => {
              e.preventDefault();
              const code = String(new FormData(e.currentTarget).get("code") ?? "").replace(/\D/g, "");
              void run(async () => {
                const hello = await serverSetup.hello(step.base, code);
                if (hello.invite) {
                  setStep({ kind: "newpin", probe: step.probe, base: step.base, code, invite: hello.invite });
                } else {
                  setStep({ kind: "account", probe: step.probe, base: step.base, code, hello });
                }
              });
            }}
          >
            <p className="text-muted">
              On a device already using <b className="text-ink">{step.probe.health.name}</b> (your phone, or another
              PC), go to <b className="text-ink">Connections › Pair a device</b> and type the six-digit code here. An
              invite or PIN-reset code goes here too.
            </p>
            <Field label="Pairing code" name="code" inputMode="numeric" autoFocus maxLength={7} placeholder="123456" />
            <Field
              label="Name for this computer"
              value={deviceName}
              onChange={(e) => setDeviceName(e.target.value)}
              maxLength={60}
              hint="Shown in the server's list of paired devices, e.g. Dan's gaming PC."
            />
            <ErrorNote>{error}</ErrorNote>
            <div className="flex gap-3">
              <Button type="submit" disabled={busy}>
                {busy ? "Checking…" : "Next"}
              </Button>
              {onCancel && (
                <Button type="button" variant="ghost" onClick={onCancel}>
                  Cancel
                </Button>
              )}
            </div>
          </form>
        )}

        {step.kind === "account" && (
          <div className="flex flex-col gap-5">
            <p className="text-muted">Who's using this PC?</p>
            <ul className="flex flex-wrap gap-4">
              {step.hello.accounts.map((a) => (
                <li key={a.id}>
                  <button
                    type="button"
                    onClick={() => setStep({ kind: "pin", probe: step.probe, base: step.base, code: step.code, account: a })}
                    className="flex w-28 flex-col items-center gap-2 rounded-2xl p-3 hover:bg-surface"
                  >
                    <Avatar name={a.name} color={a.avatar_color} size={64} />
                    <span className="font-medium">{a.name}</span>
                  </button>
                </li>
              ))}
            </ul>
          </div>
        )}

        {step.kind === "pin" && (
          <form
            className="flex flex-col gap-5"
            noValidate
            onSubmit={(e) => {
              e.preventDefault();
              const secret = String(new FormData(e.currentTarget).get("secret") ?? "");
              void run(async () => {
                const signed = await serverSetup.complete(step.base, {
                  code: step.code,
                  account_id: step.account.id,
                  secret,
                  device_name: deviceName.trim() || "Outpost desktop",
                });
                await finish(step.probe, signed);
              });
            }}
          >
            <div className="flex items-center gap-3">
              <Avatar name={step.account.name} color={step.account.avatar_color} size={48} />
              <span className="text-lg font-medium">{step.account.name}</span>
            </div>
            <Field label="PIN or password" name="secret" type="password" autoFocus autoComplete="current-password" />
            <ErrorNote>{error}</ErrorNote>
            <Button type="submit" disabled={busy}>
              {busy ? "Signing in…" : "Sign in"}
            </Button>
          </form>
        )}

        {step.kind === "newpin" && (
          <ChoosePin
            invite={step.invite}
            onSubmit={async (secret) => {
              const signed = await serverSetup.accept(step.base, {
                code: step.code,
                secret,
                device_name: deviceName.trim() || "Outpost desktop",
              });
              await finish(step.probe, signed);
            }}
          />
        )}

        {step.kind === "switching" && (
          <>
            <p className="text-muted" aria-live="polite">
              Connected. Restarting Outpost to use the server…
            </p>
            <ErrorNote>{error}</ErrorNote>
          </>
        )}
      </div>
    </main>
  );
}

/** An invite: the person picks their own PIN (a new account, or a reset). */
function ChoosePin({
  invite,
  onSubmit,
}: {
  invite: NonNullable<PairHello["invite"]>;
  onSubmit: (secret: string) => Promise<void>;
}) {
  const [secret, setSecret] = useState("");
  const [confirm, setConfirm] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  async function submit(e: FormEvent) {
    e.preventDefault();
    const problem = secretProblem(secret) ?? (secret !== confirm ? "The two entries don't match." : null);
    if (problem) return setError(problem);
    setBusy(true);
    setError(null);
    try {
      await onSubmit(secret);
    } catch (err) {
      setError(message(err));
      setBusy(false);
    }
  }
  return (
    <form onSubmit={submit} className="flex flex-col gap-5" noValidate>
      <p className="text-muted">
        {invite.kind === "join" ? (
          <>
            You're joining as <b className="text-ink">{invite.name}</b>. Pick your own PIN: you'll need it for some
            changes, and on a shared PC.
          </>
        ) : (
          <>
            Pick a new PIN for <b className="text-ink">{invite.name}</b>. It replaces the old one, and your other
            devices will need to sign in again.
          </>
        )}
      </p>
      <Field
        label="PIN or password"
        type="password"
        value={secret}
        onChange={(e) => setSecret(e.target.value)}
        autoFocus
        hint="4–12 digit PIN, or a password of 6+ characters."
      />
      <Field label="Confirm" type="password" value={confirm} onChange={(e) => setConfirm(e.target.value)} />
      <ErrorNote>{error}</ErrorNote>
      <Button type="submit" disabled={busy}>
        {busy ? "Saving…" : invite.kind === "join" ? "Join" : "Save my new PIN"}
      </Button>
    </form>
  );
}

function SetupServer({
  probe,
  base,
  deviceName,
  onDeviceName,
  onDone,
}: {
  probe: ServerProbe;
  base: string;
  deviceName: string;
  onDeviceName: (v: string) => void;
  onDone: (signed: SignInResponse) => void;
}) {
  const [code, setCode] = useState("");
  const [name, setName] = useState("");
  const [secret, setSecret] = useState("");
  const [confirm, setConfirm] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  async function submit(e: FormEvent) {
    e.preventDefault();
    const problem = !code.trim()
      ? "Enter the setup code from the server."
      : !name.trim()
        ? "Enter your name."
        : secretProblem(secret) ?? (secret !== confirm ? "The two entries don't match." : null);
    if (problem) return setError(problem);
    setBusy(true);
    setError(null);
    try {
      const signed = await serverSetup.setupOwner(base, {
        name: name.trim(),
        secret,
        setup_code: code,
        device: deviceName.trim() || "Outpost desktop",
      });
      onDone(signed);
    } catch (err) {
      setError(message(err));
      setBusy(false);
    }
  }

  return (
    <form onSubmit={submit} className="flex flex-col gap-5" noValidate>
      <p className="text-muted">
        <b className="text-ink">{probe.health.name}</b> has no owner yet. Create the owner account. You'll manage
        models, accounts and the server from it.
      </p>
      <Field
        label="Setup code"
        value={code}
        onChange={(e) => setCode(e.target.value)}
        autoFocus
        inputMode="numeric"
        placeholder="1234-5678"
        hint="On the server: outpost-gateway --setup-code. It works once and lasts 30 minutes."
      />
      <Field label="Your name" value={name} onChange={(e) => setName(e.target.value)} maxLength={40} />
      <Field
        label="PIN or password"
        type="password"
        value={secret}
        onChange={(e) => setSecret(e.target.value)}
        hint="4–12 digit PIN, or a password of 6+ characters."
      />
      <Field label="Confirm" type="password" value={confirm} onChange={(e) => setConfirm(e.target.value)} />
      <Field
        label="Name for this computer"
        value={deviceName}
        onChange={(e) => onDeviceName(e.target.value)}
        maxLength={60}
      />
      <ErrorNote>{error}</ErrorNote>
      <Button type="submit" disabled={busy}>
        {busy ? "Creating…" : "Create owner account"}
      </Button>
    </form>
  );
}

/** Asks for the PIN again when the server wants it (risky actions from a paired device). */
export function PinPromptHost() {
  const [ask, setAsk] = useState<{ reason: string; resolve: (pin: string | null) => void } | null>(null);
  const [pin, setPin] = useState("");
  const input = useRef<HTMLInputElement>(null);

  useEffect(() => {
    setPinPrompter(
      (reason) =>
        new Promise((resolve) => {
          setPin("");
          setAsk({ reason, resolve });
        }),
    );
    return () => setPinPrompter(null);
  }, []);
  useEffect(() => {
    if (ask) input.current?.focus();
  }, [ask]);

  if (!ask) return null;
  const close = (value: string | null) => {
    ask.resolve(value);
    setAsk(null);
  };
  return (
    <div role="dialog" aria-modal="true" aria-label="Enter your PIN" className="fixed inset-0 z-[60] flex items-center justify-center bg-black/70 p-6">
      <form
        className="flex w-full max-w-xs flex-col gap-4 rounded-2xl border border-line bg-night p-6"
        onSubmit={(e) => {
          e.preventDefault();
          if (pin) close(pin);
        }}
      >
        <h2 className="text-lg font-semibold">Enter your PIN</h2>
        <p className="text-sm text-muted">The server asks for it again before changes like this.</p>
        <input
          ref={input}
          type="password"
          autoComplete="current-password"
          value={pin}
          onChange={(e) => setPin(e.target.value)}
          aria-label="PIN or password"
          className="min-h-12 rounded-xl border border-line bg-surface px-4 text-center text-lg tracking-[0.3em] text-ink focus:border-ember focus:outline-none"
        />
        <div className="flex gap-3">
          <Button type="submit" disabled={!pin}>
            Continue
          </Button>
          <Button type="button" variant="ghost" onClick={() => close(null)}>
            Cancel
          </Button>
        </div>
      </form>
    </div>
  );
}

/** Settings: which Outpost this app uses. */
export function ServerPanel({
  token,
  isOwner,
  onConnect,
  onAddPerson,
}: {
  token: string;
  isOwner: boolean;
  onConnect: () => void;
  onAddPerson?: () => void;
}) {
  const [error, setError] = useState<string | null>(null);
  const [confirming, setConfirming] = useState(false);
  const [restarting, setRestarting] = useState<string | null>(null);
  return (
    <div className="flex flex-col gap-4">
      <h2 className="text-lg font-semibold">Server</h2>
      <div className="flex flex-col gap-3 rounded-2xl border border-line bg-surface p-5">
        {SERVER ? (
          <>
            <p>
              This app uses <b>{SERVER.name}</b> at {SERVER.host}
              {SERVER.port === 7421 ? "" : `:${SERVER.port}`}. Nothing runs on this PC.
            </p>
            <p className="text-sm text-muted">
              Its check words: <span className="font-mono text-ink">{SERVER.words.join(" ")}</span>
            </p>
            <p className="text-sm text-muted">
              {SERVER.people.length > 1
                ? `Shared by ${SERVER.people.map((p) => p.name).join(", ")}: it opens on "Who's using Outpost?" and asks each person's PIN.`
                : "Only you are signed in on this PC, so it stays signed in. Add someone (from the lock button, or below) and it asks who's using it."}
            </p>
            <div className="flex flex-wrap gap-3">
              {onAddPerson && (
                <Button variant="ghost" onClick={onAddPerson}>
                  Add someone to this PC
                </Button>
              )}
              {isOwner && (
                <Button
                  variant="ghost"
                  disabled={!!restarting}
                  title="E.g. after changing network or Tailscale access"
                  onClick={() => {
                    setError(null);
                    setRestarting("Restarting the server…");
                    systemApi
                      .restartServer(token)
                      .then(() => waitForServer())
                      .then((back) => setRestarting(back ? "The server is back." : "The server hasn't come back yet."))
                      .catch((e) => {
                        setRestarting(null);
                        setError(message(e));
                      });
                  }}
                >
                  {restarting ?? "Restart the server"}
                </Button>
              )}
            </div>
            {confirming ? (
              <div className="flex flex-wrap items-center gap-3">
                <span className="text-sm">Stop using the server and run Outpost on this PC again?</span>
                <Button onClick={() => void serverSetup.useThisPc().catch((e) => setError(message(e)))}>Yes, restart</Button>
                <Button variant="ghost" onClick={() => setConfirming(false)}>
                  Cancel
                </Button>
              </div>
            ) : (
              <div>
                <Button variant="ghost" onClick={() => setConfirming(true)}>
                  Use Outpost on this PC instead
                </Button>
              </div>
            )}
          </>
        ) : (
          <>
            <p className="text-sm text-muted">
              Outpost runs on this PC. If you have a home server running Outpost, this app can use that instead:
              its chats, pictures, models and graphics card. Outpost on this PC stops while it does, and this PC's
              own data stays here, untouched.
            </p>
            {isOwner && (
              <div>
                <Button variant="ghost" onClick={onConnect}>
                  Connect to an Outpost server
                </Button>
              </div>
            )}
          </>
        )}
        <ErrorNote>{error}</ErrorNote>
      </div>
      {SERVER && <CopyGallery token={token} />}
    </div>
  );
}

interface LocalGallery {
  account_id: string;
  name: string;
  avatar_color: string;
  items: number;
  bytes: number;
  personas: number;
}

interface LocalItem {
  id: string;
  mime: string;
  bytes: number;
  prompt: string | null;
  pack_id: string | null;
  seed: number | null;
  created_at: number;
}

interface LocalPersonas {
  personas: Persona[];
  memories: { id: string; persona_id: string | null; text: string }[];
}

const EXT: Record<string, string> = {
  "image/png": "png",
  "image/jpeg": "jpg",
  "image/webp": "webp",
  "image/gif": "gif",
  "video/mp4": "mp4",
  "video/webm": "webm",
};

const mb = (b: number) => (b >= 1024 ** 3 ? `${(b / 1024 ** 3).toFixed(1)} GB` : `${Math.max(1, Math.round(b / 1024 ** 2))} MB`);

type Tally = { what: string; done: number; total: number; added: number; skipped: number; failed: number };

/** Copy pictures, videos and personas from this PC's own Outpost to my account on the server. */
function CopyGallery({ token }: { token: string }) {
  const [galleries, setGalleries] = useState<LocalGallery[] | null>(null);
  const [picked, setPicked] = useState<LocalGallery | null>(null);
  const [pin, setPin] = useState("");
  const [items, setItems] = useState<LocalItem[] | null>(null);
  const [run, setRun] = useState<Tally | null>(null);
  const [running, setRunning] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [note, setNote] = useState<string | null>(null);
  const stop = useRef(false);

  useEffect(() => {
    import("@tauri-apps/api/core")
      .then(({ invoke }) => invoke<LocalGallery[]>("local_galleries"))
      .then(setGalleries, () => setGalleries([]));
  }, []);

  async function unlock(e: FormEvent) {
    e.preventDefault();
    if (!picked) return;
    setError(null);
    const { invoke } = await import("@tauri-apps/api/core");
    try {
      setItems(await invoke<LocalItem[]>("local_gallery_items", { accountId: picked.account_id, pin }));
      setPin("");
    } catch (err) {
      setError(message(err));
    }
  }

  async function copyPictures() {
    if (!picked || !items) return;
    setError(null);
    setNote(null);
    const { invoke } = await import("@tauri-apps/api/core");
    stop.current = false;
    setRunning(true);
    const tally: Tally = { what: "pictures", done: 0, total: items.length, added: 0, skipped: 0, failed: 0 };
    setRun({ ...tally });
    for (const item of items) {
      if (stop.current) break;
      try {
        const data = await invoke<ArrayBuffer>("local_gallery_file", { accountId: picked.account_id, id: item.id });
        const q = new URLSearchParams({ name: `${item.id}.${EXT[item.mime] ?? "png"}`, created_at: String(item.created_at) });
        if (item.prompt) q.set("prompt", item.prompt);
        if (item.pack_id) q.set("pack_id", item.pack_id);
        if (item.seed !== null && item.seed >= 0) q.set("seed", String(item.seed));
        const res = await fetch(`${GATEWAY}/api/media/import?${q}`, {
          method: "POST",
          headers: { Authorization: `Bearer ${token}`, "Content-Type": "application/octet-stream" },
          body: data,
        });
        const body = await res.json().catch(() => null);
        if (!res.ok) throw new Error(body?.error?.message ?? `Copy failed (${res.status}).`);
        if (body?.added) tally.added += 1;
        else tally.skipped += 1;
      } catch (err) {
        tally.failed += 1;
        setError(message(err));
      }
      tally.done += 1;
      setRun({ ...tally });
    }
    setRunning(false);
  }

  /** Personas (by name: ones already on the server are kept as they are) and the memories
   * added to them by hand. */
  async function copyPersonas() {
    if (!picked) return;
    setError(null);
    setNote(null);
    setRunning(true);
    const { invoke } = await import("@tauri-apps/api/core");
    try {
      const local = await invoke<LocalPersonas>("local_personas", { accountId: picked.account_id });
      const there = (await api.personas(token)).filter((q) => !q.owner_name);
      const tally: Tally = { what: "personas", done: 0, total: local.personas.length, added: 0, skipped: 0, failed: 0 };
      setRun({ ...tally });
      const idOnServer = new Map<string, string>();
      const missingModels = new Set<string>();
      const installed = new Set((await api.models(token).catch(() => ({ data: [] as { id: string }[] }))).data.map((m) => m.id));
      for (const p of local.personas) {
        const same = there.find((q) => q.name.trim().toLowerCase() === p.name.trim().toLowerCase());
        try {
          if (same) {
            idOnServer.set(p.id, same.id);
            tally.skipped += 1;
          } else {
            const made = await api.createPersona(token, {
              name: p.name,
              system_prompt: p.system_prompt,
              model: p.model ?? "",
              temperature: p.temperature ?? undefined,
              think: p.think,
              use_memory: p.use_memory,
              voice: p.voice ?? "",
              speak_replies: p.speak_replies,
              avatar_color: p.avatar_color,
            });
            idOnServer.set(p.id, made.id);
            tally.added += 1;
            if (p.model && !installed.has(p.model)) missingModels.add(p.model);
          }
        } catch (err) {
          tally.failed += 1;
          setError(message(err));
        }
        tally.done += 1;
        setRun({ ...tally });
      }
      // Their hand-added memories (skipping any the server already has).
      const known = new Set((await api.memory(token)).map((m) => `${m.persona_id ?? ""}|${m.text.trim()}`));
      let memories = 0;
      for (const m of local.memories) {
        const persona = m.persona_id ? idOnServer.get(m.persona_id) : undefined;
        if (m.persona_id && !persona) continue;
        if (known.has(`${persona ?? ""}|${m.text.trim()}`)) continue;
        await api.addMemory(token, m.text, persona).then(() => (memories += 1), () => undefined);
      }
      setNote(
        [
          memories ? `${memories} ${memories === 1 ? "memory" : "memories"} came with them.` : "",
          missingModels.size
            ? `Not on the server yet: ${[...missingModels].join(", ")}. Download them in Models, or pick another model for those personas.`
            : "",
        ]
          .filter(Boolean)
          .join(" "),
      );
    } catch (err) {
      setError(message(err));
    }
    setRunning(false);
  }

  if (!galleries || galleries.length === 0) return null;
  return (
    <div className="flex flex-col gap-3 rounded-2xl border border-line bg-surface p-5">
      <h3 className="font-semibold">Copy from this PC</h3>
      <p className="text-sm text-muted">
        Bring your gallery (with prompts, seeds and dates) and your personas (with the memories you added to them) from
        Outpost on this PC to your account on {SERVER?.name}. Nothing on this PC changes, and things already copied are
        skipped, so it's safe to run again.
      </p>
      <div className="flex flex-wrap gap-2">
        {galleries.map((g) => (
          <button
            key={g.account_id}
            type="button"
            disabled={running}
            onClick={() => {
              setPicked(g);
              setItems(null);
              setRun(null);
              setError(null);
              setNote(null);
            }}
            className={`flex items-center gap-2 rounded-xl border px-3 py-2 text-sm ${
              picked?.account_id === g.account_id ? "border-ember bg-raised" : "border-line hover:bg-raised"
            }`}
          >
            <Avatar name={g.name} color={g.avatar_color} size={24} />
            {g.name} · {g.items} {g.items === 1 ? "item" : "items"}
            {g.items ? ` (${mb(g.bytes)})` : ""}, {g.personas} {g.personas === 1 ? "persona" : "personas"}
          </button>
        ))}
      </div>
      {picked && !items && (
        <form onSubmit={(e) => void unlock(e)} className="flex flex-wrap items-end gap-3" noValidate>
          <div className="w-56">
            <Field
              label={`${picked.name}'s PIN on this PC`}
              type="password"
              value={pin}
              onChange={(e) => setPin(e.target.value)}
              autoComplete="off"
            />
          </div>
          <Button type="submit" disabled={!pin}>
            Continue
          </Button>
        </form>
      )}
      {picked && items && !running && (
        <div className="flex flex-wrap gap-3">
          {items.length > 0 && <Button onClick={() => void copyPictures()}>Copy {items.length} pictures and videos</Button>}
          {picked.personas > 0 && (
            <Button variant={items.length > 0 ? "ghost" : "primary"} onClick={() => void copyPersonas()}>
              Copy {picked.personas} {picked.personas === 1 ? "persona" : "personas"}
            </Button>
          )}
        </div>
      )}
      {run && (
        <div className="flex flex-col gap-1" aria-live="polite">
          <div className="h-2 overflow-hidden rounded-full bg-night">
            <div className="h-full bg-ember transition-[width]" style={{ width: `${run.total ? (run.done / run.total) * 100 : 100}%` }} />
          </div>
          <span className="text-xs text-muted">
            {running ? `Copying ${run.what}: ${run.done} of ${run.total}…` : `Done.`} {run.added} copied
            {run.skipped ? `, ${run.skipped} already there` : ""}
            {run.failed ? `, ${run.failed} couldn't be copied` : ""}.
          </span>
          {running && run.what === "pictures" && (
            <div>
              <Button variant="ghost" onClick={() => (stop.current = true)}>
                Stop
              </Button>
            </div>
          )}
        </div>
      )}
      {note && <p className="text-sm text-muted">{note}</p>}
      <ErrorNote>{error}</ErrorNote>
    </div>
  );
}

/** A shared PC using a server: who's using it? Each person enters their own PIN, which
 * the server checks. */
export function WhoPicker({ onPicked, onAdd }: { onPicked: (auth: SignInResponse) => void; onAdd: () => void }) {
  const [people, setPeople] = useState<Person[]>(SERVER?.people ?? []);
  const [picked, setPicked] = useState<Person | null>(null);
  const [pin, setPin] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  async function enter(e: FormEvent) {
    e.preventDefault();
    if (!picked || !pin) return;
    setBusy(true);
    setError(null);
    try {
      await api.checkPin(picked.token, pin);
      await saveServerToken(picked.token);
      const me = await api.me(picked.token);
      onPicked({ token: picked.token, ...me });
    } catch (err) {
      setPin("");
      setBusy(false);
      if (err instanceof ApiError && err.code === "bad_secret") {
        const left = err.attemptsLeft ?? 0;
        setError(`Wrong PIN. ${left} ${left === 1 ? "try" : "tries"} left before a short lock.`);
      } else if (err instanceof ApiError && err.status === 401) {
        // Unpaired on the server: this PC can't sign them in any more.
        await forgetPerson(picked.account_id).catch(() => undefined);
        setPeople((p) => p.filter((q) => q.account_id !== picked.account_id));
        setPicked(null);
        setError(`${picked.name} isn't paired with this PC any more. Add them again with a pairing code or an invite.`);
      } else {
        setError(message(err));
      }
    }
  }

  return (
    <main className="flex h-full flex-col items-center justify-center gap-8 overflow-auto bg-deep p-8">
      <h1 className="text-3xl font-semibold tracking-tight">Who's using Outpost?</h1>
      {picked ? (
        <form onSubmit={(e) => void enter(e)} className="flex w-full max-w-xs flex-col items-center gap-4" noValidate>
          <Avatar name={picked.name} color={picked.avatar_color} size={96} />
          <p className="text-xl font-medium">{picked.name}</p>
          <input
            type="password"
            autoFocus
            aria-label="PIN or password"
            placeholder="PIN or password"
            value={pin}
            onChange={(e) => setPin(e.target.value)}
            className="min-h-12 w-full rounded-xl border border-line bg-surface px-4 text-center text-lg tracking-[0.3em] text-ink placeholder:tracking-normal placeholder:text-muted/60 focus:border-ember focus:outline-none"
          />
          <ErrorNote>{error}</ErrorNote>
          <Button type="submit" disabled={busy || !pin} className="w-full">
            {busy ? "Checking…" : "Sign in"}
          </Button>
          <Button
            variant="ghost"
            onClick={() => {
              setPicked(null);
              setPin("");
              setError(null);
            }}
          >
            Someone else
          </Button>
        </form>
      ) : (
        <>
          <ul className="flex max-w-3xl flex-wrap justify-center gap-6">
            {people.map((p) => (
              <li key={p.account_id}>
                <button
                  type="button"
                  onClick={() => setPicked(p)}
                  className="group flex w-32 flex-col items-center gap-3 rounded-2xl p-3 hover:bg-surface"
                >
                  <span className="rounded-full ring-2 ring-transparent ring-offset-4 ring-offset-deep group-hover:ring-ember">
                    <Avatar name={p.name} color={p.avatar_color} size={88} />
                  </span>
                  <span className="font-medium">{p.name}</span>
                </button>
              </li>
            ))}
          </ul>
          <ErrorNote>{error}</ErrorNote>
          <Button variant="ghost" onClick={onAdd}>
            Add someone to this PC
          </Button>
          <p className="text-xs text-muted">Using {SERVER?.name}</p>
        </>
      )}
    </main>
  );
}
