import { useEffect, useState, type FormEvent } from "react";
import { api, ApiError, type DirectoryEntry, type SignInResponse } from "../api";
import { Avatar, Button, ErrorNote, Icon, icons } from "../components/ui";

export default function SignIn({
  account,
  onBack,
  onDone,
}: {
  account: DirectoryEntry;
  onBack: () => void;
  onDone: (auth: SignInResponse) => void;
}) {
  const [secret, setSecret] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [lockedFor, setLockedFor] = useState(0);
  const [busy, setBusy] = useState(false);

  // Count down a lockout.
  useEffect(() => {
    if (lockedFor <= 0) return;
    const t = window.setTimeout(() => setLockedFor((s) => s - 1), 1000);
    return () => window.clearTimeout(t);
  }, [lockedFor]);

  async function submit(e: FormEvent) {
    e.preventDefault();
    if (!secret || lockedFor > 0) return;
    setBusy(true);
    setError(null);
    try {
      onDone(await api.signIn(account.id, secret));
    } catch (err) {
      setSecret("");
      setBusy(false);
      if (!(err instanceof ApiError)) return setError("Something went wrong.");
      if (err.code === "locked" && err.retryAfter) {
        setLockedFor(err.retryAfter);
        setError(null);
      } else if (err.code === "bad_secret") {
        const left = err.attemptsLeft ?? 0;
        setError(`Wrong PIN or password. ${left} ${left === 1 ? "try" : "tries"} left before a short lock.`);
      } else {
        setError(err.message);
      }
    }
  }

  return (
    <main className="relative flex h-full flex-col items-center justify-center gap-8 bg-deep px-6">
      <Button variant="ghost" onClick={onBack} className="absolute top-6 left-6" aria-label="Back to accounts">
        <Icon d={icons.back} />
        Accounts
      </Button>
      <div className="flex flex-col items-center gap-4">
        <Avatar name={account.name} color={account.avatar_color} size={104} />
        <h1 className="text-2xl font-semibold">{account.name}</h1>
      </div>
      <form onSubmit={submit} className="flex w-full max-w-xs flex-col gap-4">
        <label className="sr-only" htmlFor="secret">
          PIN or password
        </label>
        <input
          id="secret"
          type="password"
          autoFocus
          autoComplete="current-password"
          placeholder="PIN or password"
          value={secret}
          disabled={lockedFor > 0}
          onChange={(e) => setSecret(e.target.value)}
          className="min-h-12 rounded-xl border border-line bg-surface px-4 text-center text-lg tracking-[0.3em] text-ink placeholder:tracking-normal placeholder:text-muted/60 focus:border-ember focus:outline-none disabled:opacity-50"
        />
        {lockedFor > 0 ? (
          <p role="alert" className="text-center text-sm text-amber">
            Too many wrong attempts. Try again in {lockedFor}s.
          </p>
        ) : (
          <ErrorNote>{error}</ErrorNote>
        )}
        <Button type="submit" disabled={busy || !secret || lockedFor > 0}>
          {busy ? "Checking…" : "Sign in"}
        </Button>
      </form>
      <p className="max-w-xs text-center text-xs text-muted">Forgot it? An owner can set a new one in Settings.</p>
    </main>
  );
}
