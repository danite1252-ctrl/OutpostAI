import { useCallback, useEffect, useState, type FormEvent } from "react";
import { api, ApiError, GATEWAY, type Account, type Session, type SignInResponse } from "../api";
import { Button, ErrorNote, Field } from "./ui";

/** App keys for Hermes and other OpenAI-compatible apps. */
export function AppKeysPanel({ auth }: { auth: SignInResponse }) {
  const isOwner = auth.account.role === "owner";
  const [accounts, setAccounts] = useState<Account[]>([]);
  const [accountId, setAccountId] = useState(auth.account.id);
  const [keys, setKeys] = useState<Session[]>([]);
  const [label, setLabel] = useState("");
  const [fresh, setFresh] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const fail = (err: unknown) => setError(err instanceof ApiError ? err.message : "Something went wrong.");

  const load = useCallback(() => {
    api.keys(auth.token, accountId === auth.account.id ? undefined : accountId).then(setKeys, fail);
  }, [auth, accountId]);
  useEffect(load, [load]);
  useEffect(() => {
    if (isOwner) api.listAccounts(auth.token).then(setAccounts, () => undefined);
  }, [auth.token, isOwner]);

  async function create(e: FormEvent) {
    e.preventDefault();
    setError(null);
    try {
      const k = await api.createKey(auth.token, label.trim(), accountId === auth.account.id ? undefined : accountId);
      setFresh(k.token);
      setLabel("");
      load();
    } catch (err) {
      fail(err);
    }
  }

  async function revoke(k: Session) {
    if (!window.confirm(`Revoke "${k.label}"? Apps using it stop working immediately.`)) return;
    try {
      await api.revokeKey(auth.token, k.id);
      load();
    } catch (err) {
      fail(err);
    }
  }

  return (
    <div className="flex flex-col gap-4">
      <div>
        <h2 className="text-lg font-semibold">App keys</h2>
        <p className="text-sm text-muted">
          For Hermes, SillyTavern, Open WebUI or your own scripts. A key acts as its account: it sees that account's
          personas and memory. Tip: make a <b>Service</b> account for Hermes in Accounts, then a key for it here.
        </p>
      </div>
      <div className="rounded-2xl border border-line bg-surface p-5 font-mono text-sm leading-relaxed text-soft">
        Base URL: {GATEWAY}/v1
        <br />
        Model: persona:assistant (or any model from GET /v1/models)
      </div>
      <form onSubmit={create} className="flex flex-wrap items-end gap-3">
        <div className="min-w-64 flex-1">
          <Field label="What will use it?" value={label} onChange={(e) => setLabel(e.target.value)} placeholder="Hermes on the NAS" maxLength={60} />
        </div>
        {isOwner && (
          <label className="flex flex-col gap-1.5 text-sm text-muted">
            Account
            <select value={accountId} onChange={(e) => setAccountId(e.target.value)} className="min-h-11 rounded-xl border border-line bg-surface px-3 text-ink">
              {accounts.map((a) => (
                <option key={a.id} value={a.id}>
                  {a.name} ({a.role})
                </option>
              ))}
            </select>
          </label>
        )}
        <Button type="submit" disabled={!label.trim()}>
          Create key
        </Button>
      </form>
      {fresh && (
        <div className="flex flex-col gap-2 rounded-2xl border border-ember bg-surface p-4" role="status">
          <span className="text-sm font-medium">Copy this key now. It won't be shown again.</span>
          <code className="rounded-lg bg-deep p-3 font-mono text-sm break-all">{fresh}</code>
          <div className="flex gap-2">
            <Button variant="ghost" onClick={() => void navigator.clipboard.writeText(fresh)}>
              Copy
            </Button>
            <Button variant="ghost" onClick={() => setFresh(null)}>
              Done
            </Button>
          </div>
        </div>
      )}
      <ErrorNote>{error}</ErrorNote>
      {keys.length > 0 && (
        <ul className="overflow-hidden rounded-2xl border border-line">
          {keys.map((k) => (
            <li key={k.id} className="flex items-center gap-4 border-b border-line bg-surface px-5 py-3 last:border-b-0">
              <span className="flex flex-1 flex-col">
                <span className="font-medium">{k.label}</span>
                <span className="text-xs text-muted">Last used {new Date(k.last_used_at * 1000).toLocaleString()}</span>
              </span>
              <Button variant="danger" onClick={() => void revoke(k)}>
                Revoke
              </Button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
