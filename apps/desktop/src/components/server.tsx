// Using an Outpost server (a home server with no screen) instead of Outpost on this PC.
//
// Connecting: type the server's address → compare its three check words with the ones the
// server shows → then either set it up (a new server: its setup code, your name and PIN)
// or pair with it (a code from "Pair a device" on something already paired, then your
// account and PIN). The app then restarts as a paired device of the server.

import { useEffect, useRef, useState, type FormEvent } from "react";
import {
  ApiError,
  SERVER,
  secretProblem,
  serverSetup,
  setPinPrompter,
  type DirectoryEntry,
  type PairHello,
  type ServerProbe,
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

  const finish = (probe: ServerProbe, token: string) =>
    run(async () => {
      setStep({ kind: "switching" });
      await serverSetup.switchTo({
        host: probe.host,
        port: probe.port,
        fingerprint: probe.fingerprint,
        name: probe.health.name,
        token,
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
            onDone={(token) => void finish(step.probe, token)}
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
                setStep({ kind: "account", probe: step.probe, base: step.base, code, hello });
              });
            }}
          >
            <p className="text-muted">
              On a device already using <b className="text-ink">{step.probe.health.name}</b> (your phone, or another
              PC), go to <b className="text-ink">Connections › Pair a device</b> and type the six-digit code here.
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
                await finish(step.probe, signed.token);
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
  onDone: (token: string) => void;
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
      onDone(signed.token);
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
export function ServerPanel({ isOwner, onConnect }: { isOwner: boolean; onConnect: () => void }) {
  const [error, setError] = useState<string | null>(null);
  const [confirming, setConfirming] = useState(false);
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
    </div>
  );
}
