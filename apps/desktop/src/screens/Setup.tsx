import { useState, type FormEvent } from "react";
import { api, ApiError, secretProblem, type SignInResponse } from "../api";
import { Button, ErrorNote, Field } from "../components/ui";
import logo from "../assets/outpost-logo.jpg";

export default function Setup({ onDone }: { onDone: (auth: SignInResponse) => void }) {
  const [name, setName] = useState("");
  const [secret, setSecret] = useState("");
  const [confirm, setConfirm] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  async function submit(e: FormEvent) {
    e.preventDefault();
    const problem = !name.trim()
      ? "Enter your name."
      : secretProblem(secret) ?? (secret !== confirm ? "The two entries don't match." : null);
    if (problem) return setError(problem);
    setBusy(true);
    setError(null);
    try {
      onDone(await api.setupOwner(name.trim(), secret));
    } catch (err) {
      setError(err instanceof ApiError ? err.message : "Something went wrong.");
      setBusy(false);
    }
  }

  return (
    <main className="flex h-full flex-wrap items-center justify-center gap-12 overflow-auto bg-deep p-8">
      <img src={logo} alt="Outpost" className="glow w-full max-w-sm rounded-3xl" />
      <form onSubmit={submit} className="flex w-full max-w-sm flex-col gap-5" noValidate>
        <div className="flex flex-col gap-2">
          <h1 className="text-3xl font-semibold tracking-tight">Welcome to Outpost</h1>
          <p className="text-muted">
            Create the owner account. You'll manage models, accounts and remote access from it.
          </p>
        </div>
        <Field label="Your name" value={name} onChange={(e) => setName(e.target.value)} autoFocus maxLength={40} />
        <Field
          label="PIN or password"
          type="password"
          value={secret}
          onChange={(e) => setSecret(e.target.value)}
          hint="4–12 digit PIN, or a password of 6+ characters."
        />
        <Field label="Confirm" type="password" value={confirm} onChange={(e) => setConfirm(e.target.value)} />
        <ErrorNote>{error}</ErrorNote>
        <Button type="submit" disabled={busy}>
          {busy ? "Creating…" : "Create owner account"}
        </Button>
        <p className="text-xs text-muted">Everything stays on this PC. Nothing is sent to the cloud.</p>
      </form>
    </main>
  );
}
