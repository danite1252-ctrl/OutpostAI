// Tools on the Outpost server (per person): they work from every device (this PC, a laptop,
// the phone) with no PC left on. Their keys and sign-ins are kept on the server, sealed; a
// sign-in happens here (the browser comes back to this PC) and is handed to the server.

import { useCallback, useEffect, useState } from "react";
import {
  ApiError,
  pcTools,
  serverTools,
  signInServerTool,
  type McpServer,
  type McpTool,
  type PersonAccess,
  type Runtimes,
  type ToolAccess,
  type ServerTool,
  type ServerToolInput,
} from "../api";
import { runtimeFor } from "../mcpCatalog";
import { Button, ErrorNote, Field, Toggle, areYouSure } from "./ui";

function message(e: unknown) {
  if (typeof e === "string") return e;
  if (e instanceof ApiError) return e.message;
  if (e instanceof Error) return e.message;
  return "Something went wrong.";
}

export const readable = (e: string) => e.replace(/^needs (node|uv|sign-in|client): /, "");
const needsSignIn = (e: string | null) => !!e && e.startsWith("needs sign-in:");
const needsClient = (e: string | null) => !!e && e.startsWith("needs client:");

export type Where = "server" | "pc";

/** Run it on the server (every device) or this PC. */
export function WhereChoice({ where, onChange, pcOnly }: { where: Where; onChange: (w: Where) => void; pcOnly?: boolean }) {
  return (
    <div className="flex flex-col gap-1.5 text-sm">
      <span className="text-muted">Runs on</span>
      <div className="flex flex-wrap gap-2">
        {(
          [
            ["server", "The Outpost server (all your devices)"],
            ["pc", "This PC only"],
          ] as const
        ).map(([id, label]) => (
          <button
            key={id}
            type="button"
            aria-pressed={where === id}
            onClick={() => onChange(id)}
            className={`min-h-9 rounded-full border px-3.5 ${where === id ? "border-ember bg-raised text-ink" : "border-line text-muted hover:text-ink"}`}
          >
            {label}
          </button>
        ))}
      </div>
      {pcOnly && where === "server" && (
        <span className="text-xs text-amber">This one works with a PC's own files or browser: it's usually best on this PC.</span>
      )}
    </div>
  );
}

export function useServerTools(token: string) {
  const [tools, setTools] = useState<ServerTool[]>([]);
  const reload = useCallback(() => serverTools.list(token).then(setTools, () => undefined), [token]);
  useEffect(() => {
    void reload();
  }, [reload]);
  return { tools, reload };
}

/** Add a tool on the server from a server-shaped description; signs in if asked. */
export async function addServerTool(
  token: string,
  s: Pick<McpServer, "name" | "catalog_id" | "transport" | "command" | "args" | "env" | "url" | "headers">,
  opts: { signIn?: boolean; client?: { id?: string; secret?: string }; onSigningIn?: () => void } = {},
): Promise<{ tool: ServerTool; tools?: McpTool[]; problem?: string }> {
  const tool = await serverTools.create(token, {
    name: s.name,
    catalog_id: s.catalog_id,
    transport: s.transport,
    command: s.command,
    args: s.args,
    url: s.url,
    secrets: { env: s.env, headers: s.headers },
  });
  if (opts.signIn) {
    // An owner's Google client is saved for the household (others sign in through it too).
    if (tool.transport === "builtin" && opts.client?.id) {
      await serverTools.setGoogleClient(token, opts.client.id, opts.client.secret).catch(() => undefined);
    }
    opts.onSigningIn?.();
    try {
      await signInServerTool(token, tool.id, opts.client);
    } catch (e) {
      return { tool, problem: `Added, but not signed in: ${readable(message(e))} Sign in from Your tools.` };
    }
  }
  try {
    return { tool, tools: await serverTools.test(token, tool.id) };
  } catch (e) {
    return { tool, problem: `Added, but it didn't start: ${readable(message(e))}` };
  }
}

/** Move a PC tool to the server: its settings, keys, sign-in and chats' choices go too. */
export async function moveToServer(token: string, pc: McpServer): Promise<ServerTool> {
  const whole = await pcTools.export(pc.id);
  const tool = await serverTools.create(token, {
    name: whole.name,
    catalog_id: whole.catalog_id,
    transport: whole.transport,
    command: whole.command,
    args: whole.args,
    url: whole.url,
    enabled: whole.enabled,
    disabled_tools: whole.disabled_tools,
    secrets: { env: whole.env, headers: whole.headers },
  });
  const oauth = whole.oauth as { access_token?: string } | null | undefined;
  if (oauth?.access_token) await serverTools.signInSave(token, tool.id, oauth).catch(() => undefined);
  if (whole.transport === "builtin" && oauth && (oauth as { client_id?: string }).client_id) {
    const o = oauth as { client_id: string; client_secret?: string };
    await serverTools.setGoogleClient(token, o.client_id, o.client_secret).catch(() => undefined);
  }
  // Chats and personas that used it keep it.
  const sel = await pcTools.allSelections();
  for (const [chatId, ids] of Object.entries(sel.chats)) {
    if (!ids.includes(pc.id)) continue;
    const now = await serverTools.chatTools(token, chatId).catch(() => null);
    if (now) await serverTools.setChatTools(token, chatId, [...now, tool.id]).catch(() => undefined);
  }
  for (const [personaId, ids] of Object.entries(sel.personas)) {
    if (!ids.includes(pc.id)) continue;
    const now = await serverTools.personaTools(token, personaId).catch(() => null);
    if (now) await serverTools.setPersonaTools(token, personaId, [...now, tool.id]).catch(() => undefined);
  }
  await pcTools.remove(pc.id);
  return tool;
}

function useServerRuntimes(token: string) {
  const [runtimes, setRuntimes] = useState<Runtimes | null>(null);
  const refresh = useCallback(() => serverTools.runtimes(token).then(setRuntimes, () => undefined), [token]);
  useEffect(() => {
    void refresh();
    const t = window.setInterval(() => void refresh(), runtimes?.installing ? 1000 : 6000);
    return () => window.clearInterval(t);
  }, [refresh, runtimes?.installing]);
  return { runtimes, refresh };
}

const GOOGLE_PARTS: [string, string][] = [
  ["drive", "Drive"],
  ["docs", "Docs"],
  ["sheets", "Sheets"],
  ["gmail", "Gmail"],
  ["calendar", "Calendar"],
];

/** One tool on the server, as listed under Your tools. */
export function ServerToolRow({ token, tool, onChanged }: { token: string; tool: ServerTool; onChanged: () => void }) {
  const [tools, setTools] = useState<McpTool[] | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [editing, setEditing] = useState(false);
  const [clientId, setClientId] = useState("");
  const [clientSecret, setClientSecret] = useState("");
  const { runtimes, refresh: refreshRuntimes } = useServerRuntimes(token);
  const need = runtimeFor(tool);
  const google = tool.transport === "builtin" && tool.command === "google";

  async function change(body: ServerToolInput) {
    setError(null);
    try {
      await serverTools.update(token, tool.id, body);
      onChanged();
    } catch (e) {
      setError(message(e));
    }
  }
  async function test() {
    setBusy("Starting…");
    setError(null);
    try {
      setTools(await serverTools.test(token, tool.id));
    } catch (e) {
      setTools(null);
      setError(message(e));
    } finally {
      setBusy(null);
    }
  }
  async function signIn() {
    setBusy("Waiting for the browser…");
    setError(null);
    try {
      if (google && clientId.trim()) await serverTools.setGoogleClient(token, clientId.trim(), clientSecret.trim()).catch(() => undefined);
      await signInServerTool(token, tool.id, { id: clientId.trim() || undefined, secret: clientSecret.trim() || undefined });
      onChanged();
      void test();
    } catch (e) {
      setError(message(e));
    } finally {
      setBusy(null);
    }
  }
  async function remove() {
    if (!await areYouSure(`Remove ${tool.name}? Its settings and sign-in on the server are deleted.`)) return;
    try {
      await serverTools.remove(token, tool.id);
      onChanged();
    } catch (e) {
      setError(message(e));
    }
  }

  const where = google
    ? `Google${tool.signed_in_as ? ` · ${tool.signed_in_as}` : ""}`
    : tool.transport === "http"
      ? tool.url
      : [tool.command, ...tool.args].join(" ");
  const canSignIn = tool.mine && (tool.transport === "http" || google);
  const showSignIn = canSignIn && (tool.signed_in || google || needsSignIn(error) || needsClient(error));

  return (
    <li className="flex flex-col gap-2 rounded-xl border border-line px-4 py-3">
      <div className="flex flex-wrap items-center gap-3">
        <span className="flex min-w-0 flex-1 flex-col">
          <span className="text-sm font-medium">
            {tool.name}
            <span className="ml-2 text-xs text-muted">
              Outpost server{tool.shared ? (tool.mine ? " · shared with everyone" : ` · shared by ${tool.owner_name ?? "an owner"}`) : ""}
            </span>
          </span>
          <span className="truncate font-mono text-xs text-muted">{where}</span>
        </span>
        {tool.mine && <Toggle checked={tool.enabled} label={`${tool.name} on`} onChange={(on) => void change({ enabled: on })} />}
        <Button variant="ghost" onClick={() => void test()} disabled={!!busy}>
          {busy === "Starting…" ? busy : "Test"}
        </Button>
        {tool.mine && (
          <>
            <Button variant="ghost" onClick={() => setEditing((e) => !e)}>
              Edit
            </Button>
            <Button variant="danger" onClick={() => void remove()}>
              Remove
            </Button>
          </>
        )}
      </div>
      {tool.mine && (
        <div className="flex flex-wrap items-center gap-4 text-xs text-muted">
          <label className="flex items-center gap-2">
            <Toggle checked={tool.shared} label="Share with everyone" onChange={(on) => void change({ shared: on })} />
            Share with everyone (they use it with your settings; owners only)
          </label>
          {tool.shared && (
            <label className="flex items-center gap-2">
              <Toggle checked={tool.restricted_ok} label="Children's accounts too" onChange={(on) => void change({ restricted_ok: on })} />
              Restricted accounts too
            </label>
          )}
        </div>
      )}
      {tool.mine && tool.shared && <PeoplePanel token={token} tool={tool} actions={tools} onTest={() => void test()} />}
      {need && runtimes && !runtimes[need] && (
        <div className="flex flex-wrap items-center gap-2 text-xs">
          <span className="text-amber">
            {runtimes.installing === need
              ? `Downloading ${need === "node" ? "Node.js" : "uv"} on the server${runtimes.percent != null ? `: ${Math.round(runtimes.percent)}%` : "…"}`
              : `Needs ${need === "node" ? "Node.js" : "Python (uv)"} on the server.`}
          </span>
          {!runtimes.installing && (
            <Button
              variant="ghost"
              onClick={() =>
                void serverTools
                  .installRuntime(token, need)
                  .then(refreshRuntimes)
                  .catch((e) => setError(message(e)))
              }
            >
              Get it on the server
            </Button>
          )}
        </div>
      )}
      {google && tool.mine && (
        <div className="flex flex-wrap gap-2 text-xs">
          {GOOGLE_PARTS.map(([k, label]) => {
            const on = tool.args.includes(k);
            return (
              <button
                key={k}
                type="button"
                aria-pressed={on}
                onClick={() => void change({ args: on ? tool.args.filter((x) => x !== k) : [...tool.args, k] })}
                className={`min-h-8 rounded-full border px-3 ${on ? "border-ember bg-raised text-ink" : "border-line text-muted hover:text-ink"}`}
              >
                {label}
              </button>
            );
          })}
          <label className="flex items-center gap-2 text-muted">
            <Toggle
              checked={tool.args.includes("write")}
              label="Allow changes"
              onChange={(v) => void change({ args: v ? [...tool.args, "write"] : tool.args.filter((x) => x !== "write") })}
            />
            Allow changes (edit Docs and Sheets, add events, write drafts). Turning on more needs Sign in again.
          </label>
        </div>
      )}
      {showSignIn && (
        <div className="flex flex-col gap-2 rounded-xl border border-line px-3 py-2 text-xs">
          {tool.signed_in ? (
            <div className="flex flex-wrap items-center gap-2">
              <span className="text-ok">Signed in{tool.signed_in_as ? ` as ${tool.signed_in_as}` : ""}</span>
              <Button variant="ghost" disabled={!!busy} onClick={() => void signIn()}>
                Sign in again
              </Button>
              <Button
                variant="ghost"
                onClick={() =>
                  void serverTools
                    .signOut(token, tool.id)
                    .then(onChanged)
                    .catch((e) => setError(message(e)))
                }
              >
                Sign out
              </Button>
            </div>
          ) : (
            <>
              <span>
                Sign in to {tool.name}: your browser opens here, and the sign-in is kept on the server, for all your devices.
              </span>
              {needsClient(error) && (
                <>
                  <Field label="OAuth client ID" value={clientId} onChange={(e) => setClientId(e.target.value)} autoComplete="off" />
                  <Field
                    label="Client secret (if it gave one)"
                    type="password"
                    value={clientSecret}
                    onChange={(e) => setClientSecret(e.target.value)}
                    autoComplete="off"
                  />
                </>
              )}
              <div>
                <Button disabled={!!busy} onClick={() => void signIn()}>
                  {busy === "Waiting for the browser…" ? busy : "Sign in"}
                </Button>
              </div>
            </>
          )}
        </div>
      )}
      {tools && (
        <div className="flex flex-col gap-1">
          <p className="text-xs text-ok">
            Working: {tools.length} tools.
            {tool.mine
              ? " Switch off any you don't want chats to use; Ask first makes everyone OK it each time (locks, alarms, doors and covers always ask)."
              : ""}
          </p>
          <ul className="flex flex-col gap-1">
            {tools.map((t) => {
              const off = tool.disabled_tools.includes(t.name);
              return (
                <li key={t.name} className="flex items-start gap-3 text-xs">
                  {tool.mine && (
                    <Toggle
                      checked={!off}
                      label={`Allow ${t.name}`}
                      onChange={(on) =>
                        void change({
                          disabled_tools: on ? tool.disabled_tools.filter((x) => x !== t.name) : [...tool.disabled_tools, t.name],
                        })
                      }
                    />
                  )}
                  <span className="flex min-w-0 flex-1 flex-col">
                    <span className="font-mono">
                      {t.name}
                      {t.read_only === true && <span className="ml-2 text-muted">reads only</span>}
                    </span>
                    <span className="line-clamp-2 text-muted">{t.description}</span>
                  </span>
                  {tool.mine && !off && t.read_only !== true && (
                    <label className="flex shrink-0 items-center gap-1.5 text-muted">
                      <Toggle
                        checked={(tool.ask_tools ?? []).includes(t.name)}
                        label={`Ask first before ${t.name}`}
                        onChange={(on) =>
                          void change({
                            ask_tools: on ? [...(tool.ask_tools ?? []), t.name] : (tool.ask_tools ?? []).filter((x) => x !== t.name),
                          })
                        }
                      />
                      Ask first
                    </label>
                  )}
                </li>
              );
            })}
          </ul>
        </div>
      )}
      {editing && tool.mine && <EditServerTool tool={tool} onSave={(body) => void change(body).then(() => setEditing(false))} />}
      <ErrorNote>{error && readable(error)}</ErrorNote>
    </li>
  );
}

function pairs(text: string, sep: string): Record<string, string> {
  const out: Record<string, string> = {};
  for (const line of text.split("\n")) {
    const at = line.indexOf(sep);
    if (at <= 0) continue;
    out[line.slice(0, at).trim()] = line.slice(at + sep.length).trim();
  }
  return out;
}

/** Change a server tool: its name, how it starts, and its keys (kept values aren't shown). */
function EditServerTool({ tool, onSave }: { tool: ServerTool; onSave: (body: ServerToolInput) => void }) {
  const [name, setName] = useState(tool.name);
  const [command, setCommand] = useState(tool.command);
  const [args, setArgs] = useState(tool.args.join("\n"));
  const [url, setUrl] = useState(tool.url);
  const http = tool.transport === "http";
  const sep = http ? ":" : "=";
  const [secrets, setSecrets] = useState(tool.secret_names.map((k) => `${k}${http ? ": " : "="}`).join("\n"));
  if (tool.transport === "builtin") return null;
  function save() {
    const given = pairs(secrets, sep);
    // Names left out are removed; names given without a value keep theirs.
    const body: Record<string, string> = {};
    for (const k of tool.secret_names) if (!(k in given)) body[k] = "";
    for (const [k, v] of Object.entries(given)) if (v) body[k] = v;
    onSave({
      name,
      ...(http ? { url } : { command, args: args.split("\n").map((a) => a.trim()).filter(Boolean) }),
      secrets: http ? { headers: body } : { env: body },
    });
  }
  return (
    <div className="flex flex-col gap-3 border-t border-line pt-3">
      <Field label="Name" value={name} onChange={(e) => setName(e.target.value)} />
      {http ? (
        <Field label="Address" value={url} onChange={(e) => setUrl(e.target.value)} />
      ) : (
        <>
          <Field label="Command" value={command} onChange={(e) => setCommand(e.target.value)} />
          <label className="flex flex-col gap-1.5 text-sm text-muted">
            Arguments (one per line)
            <textarea
              rows={3}
              value={args}
              onChange={(e) => setArgs(e.target.value)}
              className="rounded-xl border border-line bg-surface px-3.5 py-2 font-mono text-sm text-ink focus:border-ember focus:outline-none"
            />
          </label>
        </>
      )}
      <label className="flex flex-col gap-1.5 text-sm text-muted">
        {http ? "Headers (Name: value, one per line)" : "Settings (NAME=value, one per line)"}
        <textarea
          rows={3}
          spellCheck={false}
          value={secrets}
          onChange={(e) => setSecrets(e.target.value)}
          className="rounded-xl border border-line bg-surface px-3.5 py-2 font-mono text-sm text-ink focus:border-ember focus:outline-none"
        />
        <span className="text-xs">Saved values aren't shown: leave a value empty to keep it, or delete the line to remove it.</span>
      </label>
      <div>
        <Button onClick={save}>Save</Button>
      </div>
    </div>
  );
}

const ACCESS: { id: ToolAccess; label: string }[] = [
  { id: "all", label: "All of it" },
  { id: "some", label: "Only some" },
  { id: "none", label: "None" },
];

/** Who may use a shared tool, and which of its actions (its owner chooses, per person). */
function PeoplePanel({ token, tool, actions, onTest }: { token: string; tool: ServerTool; actions: McpTool[] | null; onTest: () => void }) {
  const [people, setPeople] = useState<PersonAccess[] | null>(null);
  const [open, setOpen] = useState(false);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    if (open && !people) serverTools.people(token, tool.id).then(setPeople, (e) => setError(message(e)));
  }, [open, people, token, tool.id]);
  async function set(p: PersonAccess, access: ToolAccess, list: string[]) {
    setError(null);
    try {
      setPeople(await serverTools.setPerson(token, tool.id, p.account_id, access, list));
    } catch (e) {
      setError(message(e));
    }
  }
  const usable = (actions ?? []).filter((a) => !tool.disabled_tools.includes(a.name));
  return (
    <details className="text-xs" onToggle={(e) => setOpen((e.target as HTMLDetailsElement).open)}>
      <summary className="cursor-pointer text-muted">Who can use it, and which actions</summary>
      <div className="mt-2 flex flex-col gap-3">
        {people === null ? (
          <p className="text-muted">Loading…</p>
        ) : people.length === 0 ? (
          <p className="text-muted">There's nobody else in the household yet.</p>
        ) : (
          people.map((p) => (
            <div key={p.account_id} className="flex flex-col gap-2 rounded-xl border border-line px-3 py-2">
              <div className="flex flex-wrap items-center gap-2">
                <span className="min-w-28 font-medium text-ink">
                  {p.name}
                  {p.role === "restricted" && <span className="ml-1 text-muted">(restricted)</span>}
                </span>
                {ACCESS.map((a) => (
                  <button
                    key={a.id}
                    type="button"
                    aria-pressed={p.access === a.id}
                    onClick={() => void set(p, a.id, a.id === "some" ? (p.actions.length ? p.actions : []) : [])}
                    className={`min-h-8 rounded-full border px-3 ${p.access === a.id ? "border-ember bg-raised text-ink" : "border-line text-muted hover:text-ink"}`}
                  >
                    {a.label}
                  </button>
                ))}
                {!p.chosen && <span className="text-muted">(the default)</span>}
              </div>
              {p.access === "some" &&
                (actions === null ? (
                  <div className="flex items-center gap-2 text-muted">
                    Press Test to list its actions, then pick theirs.
                    <Button variant="ghost" onClick={onTest}>
                      Test
                    </Button>
                  </div>
                ) : (
                  <div className="flex flex-wrap gap-2">
                    {usable.map((a) => {
                      const on = p.actions.includes(a.name);
                      return (
                        <button
                          key={a.name}
                          type="button"
                          aria-pressed={on}
                          title={a.description}
                          onClick={() => void set(p, "some", on ? p.actions.filter((x) => x !== a.name) : [...p.actions, a.name])}
                          className={`min-h-8 rounded-full border px-3 font-mono ${on ? "border-ember bg-raised text-ink" : "border-line text-muted hover:text-ink"}`}
                        >
                          {a.name}
                          {a.read_only === true ? " ·reads" : ""}
                        </button>
                      );
                    })}
                  </div>
                ))}
            </div>
          ))
        )}
        <p className="text-muted">
          Outpost checks this on every call, so a model can't use an action someone isn't allowed, and it tells them to ask you.
          Restricted accounts get none of it unless you pick for them (or turn on Restricted accounts too).
        </p>
        <ErrorNote>{error}</ErrorNote>
      </div>
    </details>
  );
}
