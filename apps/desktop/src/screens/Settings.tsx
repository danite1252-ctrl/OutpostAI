import { useCallback, useEffect, useState, type FormEvent } from "react";
import { api, ApiError, SERVER, secretProblem, type Account, type EnginesStatus, type Role, type SignInResponse } from "../api";
import { EnginesPanel } from "../components/engines";
import { DiagnosticsPanel, UpdatesPanel, WindowPanel } from "../components/system";
import { Avatar, Button, ErrorNote, Field, Section, ToggleRow } from "../components/ui";
import { CopyGallery, ServerPanel } from "../components/server";
import { HelperPanel } from "../components/helper";
import { ListeningPanel } from "../components/listening";
import { PairDialog } from "./Connections";

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
  onUseServer,
  onAddPerson,
}: {
  auth: SignInResponse;
  version: string;
  engines: EnginesStatus | null;
  onEnginesChanged: () => void;
  onSignedOut: () => void;
  onUseServer: () => void;
  onAddPerson?: () => void;
}) {
  const isOwner = auth.account.role === "owner";
  const running = engines?.engines.filter((e) => e.state === "running" || e.state === "external").length ?? 0;
  return (
    <section className="mx-auto flex max-w-4xl flex-col gap-4">
      <h1 className="text-2xl font-semibold">Settings</h1>
      <p className="mb-2 text-sm text-muted">
        App keys, paired phones and network access are under <b className="text-ink">Connections</b> in the side bar.
      </p>

      <Section
        id="server"
        title="Server"
        defaultOpen
        summary={SERVER ? `Using ${SERVER.name}` : "Outpost runs on this PC"}
      >
        <ServerPanel token={auth.token} isOwner={isOwner} onConnect={onUseServer} onAddPerson={onAddPerson} />
        {SERVER && <CopyGallery token={auth.token} />}
      </Section>

      {SERVER && (
        <Section id="helper" title="Lend this PC's graphics card" summary="Make pictures too big for the server's card here">
          <HelperPanel token={auth.token} isOwner={isOwner} />
        </Section>
      )}

      {isOwner && (
        <Section id="accounts" title="Accounts" summary="People, invites and PIN resets">
          <AccountsPanel token={auth.token} selfId={auth.account.id} />
        </Section>
      )}

      <Section id="pin" title="My PIN or password" summary={`Signed in as ${auth.account.name}`}>
        <ChangeMySecret token={auth.token} onChanged={onSignedOut} />
      </Section>

      <Section
        id="engines"
        title="Engines"
        summary={
          engines
            ? `${running} of ${engines.engines.length} running${SERVER ? ` on ${SERVER.name}` : ""}`
            : "Ollama, ComfyUI and voice"
        }
      >
        <EnginesPanel token={auth.token} canManage={isOwner} status={engines} onChanged={onEnginesChanged} />
      </Section>

      <Section id="voice" title="Voice" summary="How Outpost listens">
        <ListeningPanel token={auth.token} canManage={isOwner} />
      </Section>

      <Section id="app" title="Updates and this window" summary={`Outpost ${version}`}>
        <UpdatesPanel token={auth.token} canManage={isOwner} />
        <WindowPanel />
      </Section>

      {isOwner && (
        <Section id="diagnostics" title="Diagnostics" summary="Crash reports and logs, kept on this PC">
          <DiagnosticsPanel token={auth.token} />
        </Section>
      )}

      <p className="mt-4 text-xs text-muted">
        Outpost {version} · MIT licence · {SERVER ? `runs on ${SERVER.name}, your own server` : "runs entirely on this PC"}
      </p>
    </section>
  );
}

function AccountsPanel({ token, selfId }: { token: string; selfId: string }) {
  const [accounts, setAccounts] = useState<Account[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [resetFor, setResetFor] = useState<Account | null>(null);
  // An invite to show: someone new, or a PIN reset they do on their own device.
  const [invite, setInvite] = useState<{ title: string; body: { name?: string; role?: Role; account_id?: string } } | null>(null);

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
              <Button
                variant="ghost"
                title="A code they use on their own device to pick a new PIN"
                onClick={() => setInvite({ title: `PIN reset for ${a.name}`, body: { account_id: a.id } })}
              >
                Send PIN reset
              </Button>
            )}
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
      <AddAccount token={token} onAdded={load} onInvite={(name, role) => setInvite({ title: `Invite ${name}`, body: { name, role } })} />
      {invite && (
        <PairDialog
          token={token}
          title={invite.title}
          make={() => api.createInvite(token, invite.body)}
          onClose={() => {
            setInvite(null);
            load();
          }}
        />
      )}
    </div>
  );
}

function AddAccount({
  token,
  onAdded,
  onInvite,
}: {
  token: string;
  onAdded: () => void;
  onInvite: (name: string, role: Role) => void;
}) {
  const [name, setName] = useState("");
  const [role, setRole] = useState<Role>("member");
  const [secret, setSecret] = useState("");
  // Most people pick their own PIN on their own device (an invite).
  const [byInvite, setByInvite] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  async function submit(e: FormEvent) {
    e.preventDefault();
    const isService = role === "service";
    if (byInvite && !isService) {
      if (!name.trim()) return setError("Enter a name.");
      setError(null);
      onInvite(name.trim(), role);
      setName("");
      setRole("member");
      return;
    }
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
        {role !== "service" && !byInvite && (
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
      {role !== "service" && (
        <ToggleRow
          title="Send them an invite"
          hint="They pick their own PIN on their phone or PC."
          checked={byInvite}
          onChange={setByInvite}
        />
      )}
      <ErrorNote>{error}</ErrorNote>
      <Button type="submit" disabled={busy} className="self-start">
        {busy ? "Adding…" : byInvite && role !== "service" ? "Make an invite" : "Add account"}
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
    <form onSubmit={submit} className="flex flex-col gap-4 rounded-2xl border border-line bg-surface p-5" noValidate>
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
