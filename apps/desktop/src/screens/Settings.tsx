import { useCallback, useEffect, useState, type FormEvent } from "react";
import { api, ApiError, secretProblem, type Account, type EnginesStatus, type Role, type SignInResponse } from "../api";
import { EnginesPanel } from "../components/engines";
import { Avatar, Button, ErrorNote, Field } from "../components/ui";

const ROLES: { value: Role; label: string; hint: string }[] = [
  { value: "member", label: "Member", hint: "Full use; can't manage the Outpost" },
  { value: "restricted", label: "Restricted", hint: "For kids or guests; safe mode (from phase 1)" },
  { value: "owner", label: "Owner", hint: "Manages accounts, models and remote access" },
  { value: "service", label: "Service", hint: "For apps like Hermes: no PIN, uses an app key" },
];

function message(err: unknown) {
  return err instanceof ApiError ? err.message : "Something went wrong.";
}

export default function Settings({
  auth,
  version,
  engines,
  onEnginesChanged,
  onSignedOut,
}: {
  auth: SignInResponse;
  version: string;
  engines: EnginesStatus | null;
  onEnginesChanged: () => void;
  onSignedOut: () => void;
}) {
  const isOwner = auth.account.role === "owner";
  return (
    <section className="mx-auto flex max-w-4xl flex-col gap-10">
      <h1 className="text-2xl font-semibold">Settings</h1>
      <div className="flex flex-col gap-4">
        <h2 className="text-lg font-semibold">Engines</h2>
        <EnginesPanel token={auth.token} canManage={isOwner} status={engines} onChanged={onEnginesChanged} />
      </div>
      {isOwner && <AccountsPanel token={auth.token} selfId={auth.account.id} />}
      <p className="rounded-2xl border border-line bg-surface p-4 text-sm text-muted">
        App keys, paired phones and network access are under <b className="text-ink">Connections</b> in the side bar.
      </p>
      <ChangeMySecret token={auth.token} onChanged={onSignedOut} />
      <p className="text-xs text-muted">Outpost {version} · MIT licence · runs entirely on this PC</p>
    </section>
  );
}

function AccountsPanel({ token, selfId }: { token: string; selfId: string }) {
  const [accounts, setAccounts] = useState<Account[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [resetFor, setResetFor] = useState<Account | null>(null);

  const load = useCallback(() => {
    api.listAccounts(token).then(setAccounts, (e) => setError(message(e)));
  }, [token]);
  useEffect(load, [load]);

  async function remove(a: Account) {
    if (!window.confirm(`Delete ${a.name}'s account? Their chats and pictures will be removed in later phases too.`)) return;
    try {
      await api.deleteAccount(token, a.id);
      load();
    } catch (e) {
      setError(message(e));
    }
  }

  return (
    <div className="flex flex-col gap-4">
      <h2 className="text-lg font-semibold">Accounts</h2>
      <ErrorNote>{error}</ErrorNote>
      <ul className="overflow-hidden rounded-2xl border border-line">
        {accounts.map((a) => (
          <li key={a.id} className="flex flex-wrap items-center gap-4 border-b border-line bg-surface px-5 py-3 last:border-b-0">
            <Avatar name={a.name} color={a.avatar_color} size={36} />
            <span className="flex flex-1 flex-col">
              <span className="font-medium">
                {a.name} {a.id === selfId && <span className="text-xs text-muted">(you)</span>}
              </span>
              <span className="text-xs text-muted capitalize">{a.role}</span>
            </span>
            {a.role !== "service" && (
              <Button variant="ghost" onClick={() => setResetFor(a)}>
                Set new PIN
              </Button>
            )}
            {a.id !== selfId && (
              <Button variant="danger" onClick={() => void remove(a)}>
                Delete
              </Button>
            )}
          </li>
        ))}
      </ul>
      {resetFor && (
        <ResetSecret token={token} account={resetFor} onClose={() => setResetFor(null)} />
      )}
      <AddAccount token={token} onAdded={load} />
    </div>
  );
}

function AddAccount({ token, onAdded }: { token: string; onAdded: () => void }) {
  const [name, setName] = useState("");
  const [role, setRole] = useState<Role>("member");
  const [secret, setSecret] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  async function submit(e: FormEvent) {
    e.preventDefault();
    const isService = role === "service";
    const problem = !name.trim() ? "Enter a name." : isService ? null : secretProblem(secret);
    if (problem) return setError(problem);
    setBusy(true);
    setError(null);
    try {
      await api.createAccount(token, { name: name.trim(), role, secret: isService ? null : secret });
      setName("");
      setSecret("");
      setRole("member");
      onAdded();
    } catch (err) {
      setError(message(err));
    } finally {
      setBusy(false);
    }
  }

  return (
    <form onSubmit={submit} className="flex flex-col gap-4 rounded-2xl border border-line bg-surface p-5" noValidate>
      <h3 className="font-medium">Add an account</h3>
      <div className="grid grid-cols-1 gap-4 md:grid-cols-2">
        <Field label="Name" value={name} onChange={(e) => setName(e.target.value)} maxLength={40} />
        {role !== "service" && (
          <Field
            label="Their PIN or password"
            type="password"
            value={secret}
            onChange={(e) => setSecret(e.target.value)}
            hint="They can change it later."
          />
        )}
      </div>
      <fieldset className="flex flex-col gap-2">
        <legend className="mb-2 text-sm text-muted">Role</legend>
        <div className="grid grid-cols-1 gap-2 md:grid-cols-2">
          {ROLES.map((r) => (
            <label
              key={r.value}
              className={`flex cursor-pointer flex-col gap-1 rounded-xl border p-3 text-sm ${
                role === r.value ? "border-ember bg-raised" : "border-line"
              }`}
            >
              <span className="flex items-center gap-2 font-medium">
                <input
                  type="radio"
                  name="role"
                  value={r.value}
                  checked={role === r.value}
                  onChange={() => setRole(r.value)}
                  className="accent-ember"
                />
                {r.label}
              </span>
              <span className="text-xs text-muted">{r.hint}</span>
            </label>
          ))}
        </div>
      </fieldset>
      <ErrorNote>{error}</ErrorNote>
      <Button type="submit" disabled={busy} className="self-start">
        {busy ? "Adding…" : "Add account"}
      </Button>
    </form>
  );
}

function ResetSecret({ token, account, onClose }: { token: string; account: Account; onClose: () => void }) {
  const [secret, setSecret] = useState("");
  const [error, setError] = useState<string | null>(null);

  async function submit(e: FormEvent) {
    e.preventDefault();
    const problem = secretProblem(secret);
    if (problem) return setError(problem);
    try {
      await api.resetSecret(token, account.id, secret);
      onClose();
    } catch (err) {
      setError(message(err));
    }
  }

  return (
    <form onSubmit={submit} className="flex flex-col gap-4 rounded-2xl border border-ember bg-surface p-5" noValidate>
      <h3 className="font-medium">New PIN or password for {account.name}</h3>
      <p className="text-sm text-muted">This signs {account.name} out everywhere.</p>
      <Field label="New PIN or password" type="password" value={secret} onChange={(e) => setSecret(e.target.value)} autoFocus />
      <ErrorNote>{error}</ErrorNote>
      <div className="flex gap-2">
        <Button type="submit">Save</Button>
        <Button variant="ghost" onClick={onClose}>
          Cancel
        </Button>
      </div>
    </form>
  );
}

function ChangeMySecret({ token, onChanged }: { token: string; onChanged: () => void }) {
  const [current, setCurrent] = useState("");
  const [next, setNext] = useState("");
  const [error, setError] = useState<string | null>(null);

  async function submit(e: FormEvent) {
    e.preventDefault();
    const problem = secretProblem(next);
    if (problem) return setError(problem);
    try {
      await api.changeMySecret(token, current, next);
      onChanged(); // every session for this account was signed out, including this one
    } catch (err) {
      setError(message(err));
    }
  }

  return (
    <form onSubmit={submit} className="flex flex-col gap-4" noValidate>
      <h2 className="text-lg font-semibold">My PIN or password</h2>
      <div className="grid grid-cols-1 gap-4 md:grid-cols-2">
        <Field label="Current" type="password" value={current} onChange={(e) => setCurrent(e.target.value)} />
        <Field label="New" type="password" value={next} onChange={(e) => setNext(e.target.value)} hint="You'll sign in again after changing it." />
      </div>
      <ErrorNote>{error}</ErrorNote>
      <Button type="submit" variant="ghost" className="self-start">
        Change
      </Button>
    </form>
  );
}
