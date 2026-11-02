// Tools for chats: MCP servers (web search, GitHub, a browser...) and Outpost's own Google tools.
// Each runs on the Outpost server (with your own keys and sign-ins, from every device) or on
// this PC (while this app is open). A chat uses its persona's tools unless it's given its own
// set. The library lists ones that are known to work; any other MCP server can be added by hand.

import { useCallback, useEffect, useMemo, useState, type ReactNode } from "react";
import { pcTools, serverTools, type McpServer, type McpTool, type MessageTools, type Runtimes, type ServerTool } from "../api";
import { addServerTool, moveToServer, ServerToolRow, useServerTools, WhereChoice, type Where } from "./servertools";
import { CATALOG, CATEGORIES, fromCatalog, missingInputs, runtimeFor, type CatalogEntry } from "../mcpCatalog";
import { Button, ErrorNote, Field, Toggle, areYouSure } from "./ui";

/** Tools work only in the desktop app (they run on this PC). */
export const toolsAvailable = () => typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

function message(e: unknown) {
  if (typeof e === "string") return e;
  if (e instanceof Error) return e.message;
  return "Something went wrong.";
}

/** Strip the "needs node:" marker the app uses to say what to install. */
function readable(e: string) {
  return e.replace(/^needs (node|uv|sign-in|client): /, "");
}

const needsSignIn = (e: string | null) => !!e && e.startsWith("needs sign-in:");
const needsClient = (e: string | null) => !!e && e.startsWith("needs client:");

export function useChatTools(token: string, chatId: string | null, personaId: string | null) {
  const [servers, setServers] = useState<McpServer[]>([]);
  const [selected, setSelected] = useState<string[]>([]);
  // Tools on the server: the chat's (or, before it exists, its persona's).
  const [onServer, setOnServer] = useState<ServerTool[]>([]);
  const [serverSelected, setServerSelected] = useState<string[]>([]);
  const reload = useCallback(async () => {
    const [st, ssel] = await Promise.all([
      serverTools.list(token).catch(() => [] as ServerTool[]),
      chatId
        ? serverTools.chatTools(token, chatId).catch(() => [] as string[])
        : personaId
          ? serverTools.personaTools(token, personaId).catch(() => [] as string[])
          : Promise.resolve([] as string[]),
    ]);
    setOnServer(st);
    setServerSelected(ssel);
    if (!toolsAvailable()) return;
    const [list, sel] = await Promise.all([pcTools.list().catch(() => []), pcTools.selection(chatId, personaId).catch(() => [])]);
    setServers(list);
    setSelected(sel);
  }, [token, chatId, personaId]);
  useEffect(() => {
    void reload();
  }, [reload]);
  const active = servers.filter((s) => s.enabled && selected.includes(s.id));
  /** The selected servers' tools, to send with a message (they start if need be). */
  const forMessage = useCallback(async (): Promise<MessageTools[]> => {
    if (!toolsAvailable() || active.length === 0) return [];
    return pcTools.messageTools(active.map((s) => s.id));
  }, [active]);
  const serverActive = onServer.filter((s) => s.enabled && serverSelected.includes(s.id));
  return { servers, selected, setSelected, active, reload, forMessage, onServer, serverSelected, serverActive };
}

export type ChatTools = ReturnType<typeof useChatTools>;

function Drawer({ label, onClose, children, wide }: { label: string; onClose: () => void; children: ReactNode; wide?: boolean }) {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);
  return (
    <div role="dialog" aria-modal="true" aria-label={label} className="fixed inset-0 z-40 flex justify-end bg-black/60" onClick={onClose}>
      <div
        className={`flex h-full w-full ${wide ? "max-w-2xl" : "max-w-md"} flex-col gap-4 overflow-auto border-l border-line bg-surface p-5`}
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex items-center gap-3">
          <h2 className="flex-1 text-lg font-semibold">{label}</h2>
          <Button variant="ghost" onClick={onClose}>
            Close
          </Button>
        </div>
        {children}
      </div>
    </div>
  );
}

/** Which tools this chat uses. */
export function ToolsPanel({
  token,
  tools,
  chatId,
  personaName,
  ensureChat,
  onClose,
}: {
  token: string;
  tools: ChatTools;
  chatId: string | null;
  personaName: string | null;
  ensureChat: () => Promise<string>;
  onClose: () => void;
}) {
  const [error, setError] = useState<string | null>(null);
  const [library, setLibrary] = useState(false);

  async function choose(ids: string[] | null) {
    setError(null);
    try {
      const id = chatId ?? (await ensureChat());
      await pcTools.selectChat(id, ids);
      await tools.reload();
    } catch (e) {
      setError(message(e));
    }
  }

  async function chooseServer(ids: string[] | null) {
    setError(null);
    try {
      const id = chatId ?? (await ensureChat());
      if (ids) await serverTools.setChatTools(token, id, ids);
      else await serverTools.resetChatTools(token, id);
      await tools.reload();
    } catch (e) {
      setError(message(e));
    }
  }

  return (
    <Drawer label="Tools for this chat" onClose={onClose}>
      <p className="text-sm text-muted">
        The chat's model needs the tools badge to use them.
        {personaName ? ` New chats with ${personaName} start with the tools set in Personas.` : ""}
      </p>
      {tools.onServer.length > 0 && (
        <>
          <h3 className="text-sm font-semibold text-muted">On the Outpost server (every device)</h3>
          <ul className="flex flex-col gap-2">
            {tools.onServer.map((s) => {
              const on = tools.serverSelected.includes(s.id);
              return (
                <li key={s.id} className="flex items-center gap-3 rounded-xl border border-line px-3 py-2">
                  <span className="flex min-w-0 flex-1 flex-col">
                    <span className="text-sm">{s.name}</span>
                    <span className="text-xs text-muted">
                      {!s.enabled ? "Switched off" : s.mine ? (s.signed_in_as ?? "") : `Shared by ${s.owner_name ?? "an owner"}`}
                    </span>
                  </span>
                  <Toggle
                    checked={on}
                    disabled={!s.enabled}
                    label={`Use ${s.name} in this chat`}
                    onChange={(v) =>
                      void chooseServer(v ? [...tools.serverSelected, s.id] : tools.serverSelected.filter((x) => x !== s.id))
                    }
                  />
                </li>
              );
            })}
          </ul>
        </>
      )}
      {toolsAvailable() && tools.servers.length > 0 && <h3 className="text-sm font-semibold text-muted">On this PC (while it's open)</h3>}
      {tools.servers.length === 0 && tools.onServer.length === 0 ? (
        <p className="text-sm">No tools yet. Add some from the library: web search, Google, Home Assistant, GitHub and more.</p>
      ) : (
        <ul className="flex flex-col gap-2">
          {tools.servers.map((s) => {
            const on = tools.selected.includes(s.id);
            return (
              <li key={s.id} className="flex items-center gap-3 rounded-xl border border-line px-3 py-2">
                <span className="flex min-w-0 flex-1 flex-col">
                  <span className="text-sm">{s.name}</span>
                  {!s.enabled && <span className="text-xs text-muted">Switched off in the library</span>}
                </span>
                <Toggle
                  checked={on}
                  disabled={!s.enabled}
                  label={`Use ${s.name} in this chat`}
                  onChange={(v) => void choose(v ? [...tools.selected, s.id] : tools.selected.filter((x) => x !== s.id))}
                />
              </li>
            );
          })}
        </ul>
      )}
      <div className="flex flex-wrap gap-2">
        <Button onClick={() => setLibrary(true)}>Tool library</Button>
        {chatId && (tools.servers.length > 0 || tools.onServer.length > 0) && (
          <Button
            variant="ghost"
            onClick={() => {
              void chooseServer(null);
              if (toolsAvailable()) void choose(null);
            }}
            title="Use the persona's tools again"
          >
            Use the persona's tools
          </Button>
        )}
      </div>
      <ErrorNote>{error}</ErrorNote>
      {library && (
        <ToolLibrary
          token={token}
          onClose={() => {
            setLibrary(false);
            void tools.reload();
          }}
        />
      )}
    </Drawer>
  );
}

/** Node and uv on this PC: what a tool needs, with a button to get it. */
function RuntimeNote({ need, runtimes, onInstall }: { need: "node" | "uv"; runtimes: Runtimes | null; onInstall: () => void }) {
  if (!runtimes || runtimes[need]) return null;
  const label = need === "node" ? "Node.js" : "Python (through uv)";
  if (runtimes.installing === need)
    return <p className="text-xs text-muted">Downloading {label}{runtimes.percent != null ? `: ${Math.round(runtimes.percent)}%` : "…"}</p>;
  return (
    <div className="flex flex-wrap items-center gap-2 text-xs">
      <span className="text-amber">Needs {label}, which isn't on this PC yet.</span>
      <Button variant="ghost" onClick={onInstall} disabled={!!runtimes.installing}>
        Get {need === "node" ? "Node.js" : "uv"} ({need === "node" ? "about 30 MB" : "about 20 MB"})
      </Button>
    </div>
  );
}

function useRuntimes() {
  const [runtimes, setRuntimes] = useState<Runtimes | null>(null);
  const [error, setError] = useState<string | null>(null);
  const refresh = useCallback(() => {
    pcTools.runtimes().then(setRuntimes, () => undefined);
  }, []);
  useEffect(() => {
    refresh();
    const t = window.setInterval(refresh, runtimes?.installing ? 800 : 4000);
    return () => window.clearInterval(t);
  }, [refresh, runtimes?.installing]);
  const install = (which: "node" | "uv") => {
    setError(null);
    pcTools.installRuntime(which).then(refresh, (e) => setError(message(e)));
  };
  return { runtimes, install, error: error ?? runtimes?.error ?? null };
}

/** The tool library: what's on this PC, what can be added, and adding your own. */
export function ToolLibrary({ token, onClose }: { token: string; onClose: () => void }) {
  const [tab, setTab] = useState<"library" | "mine" | "custom">("library");
  const server = useServerTools(token);
  const [servers, setServers] = useState<McpServer[]>([]);
  const [error, setError] = useState<string | null>(null);
  const rt = useRuntimes();
  const reload = useCallback(
    () =>
      pcTools.list().then(async (list) => {
        // Library entries fixed since they were added (Weather's start command).
        for (const s of list) {
          if (s.catalog_id === "weather" && s.args.length === 1 && s.args[0] === "mcp_weather_server") {
            const fixed = { ...s, args: ["--with", "mcp<2", "mcp_weather_server"] };
            await pcTools.save(fixed).catch(() => undefined);
            Object.assign(s, fixed);
          }
        }
        setServers(list);
      }, (e) => setError(message(e))),
    [],
  );
  useEffect(() => {
    void reload();
  }, [reload]);

  return (
    <Drawer label="Tool library" onClose={onClose} wide>
      <p className="text-sm text-muted">
        Tools let chats search the web, read pages, use Google and Home Assistant, work with GitHub and more. On the Outpost
        server they work from all your devices (keys kept there, encrypted); on this PC, only while this app is open.
      </p>
      <div className="flex gap-2" role="tablist">
        {(
          [
            ["library", "Library"],
            ["mine", `Your tools${servers.length + server.tools.length ? ` (${servers.length + server.tools.length})` : ""}`],
            ["custom", "Add your own"],
          ] as const
        ).map(([id, label]) => (
          <button
            key={id}
            type="button"
            role="tab"
            aria-selected={tab === id}
            onClick={() => setTab(id)}
            className={`min-h-9 rounded-full border px-3.5 text-sm ${tab === id ? "border-ember bg-raised text-ink" : "border-line text-muted hover:text-ink"}`}
          >
            {label}
          </button>
        ))}
      </div>
      <ErrorNote>{error ?? rt.error}</ErrorNote>
      {tab === "library" && (
        <div className="flex flex-col gap-5">
          {CATEGORIES.map((c) => (
            <section key={c} className="flex flex-col gap-2">
              <h3 className="text-sm font-semibold text-muted">{c}</h3>
              {CATALOG.filter((e) => e.category === c).map((e) => (
                <CatalogCard
                  key={e.id}
                  token={token}
                  entry={e}
                  added={servers.filter((s) => s.catalog_id === e.id).length + server.tools.filter((s) => s.mine && s.catalog_id === e.id).length}
                  runtimes={rt.runtimes}
                  onInstall={rt.install}
                  onAdded={() => {
                    void reload();
                    void server.reload();
                  }}
                />
              ))}
            </section>
          ))}
        </div>
      )}
      {tab === "mine" &&
        (servers.length === 0 && server.tools.length === 0 ? (
          <p className="text-sm">Nothing yet: add tools from the Library tab.</p>
        ) : (
          <ul className="flex flex-col gap-3">
            {server.tools.map((s) => (
              <ServerToolRow key={s.id} token={token} tool={s} onChanged={() => void server.reload()} />
            ))}
            {servers.map((s) => (
              <ServerRow
                key={s.id}
                token={token}
                server={s}
                runtimes={rt.runtimes}
                onInstall={rt.install}
                onChanged={() => {
                  void reload();
                  void server.reload();
                }}
              />
            ))}
          </ul>
        ))}
      {tab === "custom" && (
        <CustomServer
          token={token}
          onAdded={() => {
            void reload();
            void server.reload();
            setTab("mine");
          }}
        />
      )}
    </Drawer>
  );
}

/** Entries that work on a PC's own files or browser: best on that PC. */
const PC_ONLY = new Set(["filesystem", "git", "playwright"]);

function CatalogCard({
  token,
  entry,
  added,
  runtimes,
  onInstall,
  onAdded,
}: {
  token: string;
  entry: CatalogEntry;
  added: number;
  runtimes: Runtimes | null;
  onInstall: (w: "node" | "uv") => void;
  onAdded: () => void;
}) {
  const [open, setOpen] = useState(false);
  const [values, setValues] = useState<Record<string, string>>(() =>
    Object.fromEntries(entry.inputs.filter((i) => i.value).map((i) => [i.key, i.value as string])),
  );
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const pcOnly = PC_ONLY.has(entry.id);
  const [where, setWhere] = useState<Where>(!toolsAvailable() ? "server" : pcOnly ? "pc" : "server");
  // A Google client the household has already: no need to enter it again.
  const [googleClient, setGoogleClient] = useState(false);
  useEffect(() => {
    if (entry.id === "google") serverTools.googleClient(token).then((c) => setGoogleClient(!!c.client_id), () => undefined);
  }, [entry.id, token]);
  const hideClient = where === "server" && googleClient;
  const inputs = entry.inputs.filter((i) => !(hideClient && (i.kind === "client_id" || i.kind === "client_secret")));
  const need = where === "pc" ? runtimeFor(entry) : null;
  const missing = missingInputs({ ...entry, inputs }, values);

  async function addOnServer() {
    setBusy(true);
    setError(null);
    setResult(null);
    try {
      const r = await addServerTool(token, fromCatalog(entry, values) as McpServer, {
        signIn: entry.signIn,
        client: { id: values.client_id, secret: values.client_secret },
        onSigningIn: () => setResult("Finish signing in in your browser, then come back here…"),
      });
      onAdded();
      if (r.problem) {
        setResult(null);
        setError(r.problem);
      } else {
        setResult(
          `Ready on the server: ${r.tools?.length ?? 0} tools, on all your devices. Turn it on for a chat from the + menu, or for a persona in Personas.`,
        );
        setOpen(false);
      }
    } catch (e) {
      setResult(null);
      setError(message(e));
    } finally {
      setBusy(false);
    }
  }

  async function add() {
    if (where === "server") return addOnServer();
    setBusy(true);
    setError(null);
    setResult(null);
    try {
      const saved = await pcTools.save(fromCatalog(entry, values) as McpServer);
      onAdded();
      if (entry.signIn) {
        setResult("Finish signing in in your browser, then come back here…");
        try {
          await pcTools.signIn(saved.id, values.client_id, values.client_secret, entry.oauthScope);
          onAdded();
        } catch (e) {
          setResult(null);
          setError(`Added, but not signed in: ${readable(message(e))} Sign in from On this PC.`);
          return;
        }
      }
      // Start it now, so problems show here rather than in a chat.
      try {
        const tools = await pcTools.tools(saved.id);
        setResult(`Ready: ${tools.length} tool${tools.length === 1 ? "" : "s"}. Turn it on for a chat from the chat's + menu, or for a persona in Personas.`);
        setOpen(false);
      } catch (e) {
        setError(`Added, but it didn't start: ${readable(message(e))}`);
      }
    } catch (e) {
      setError(message(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="flex flex-col gap-2 rounded-xl border border-line px-4 py-3">
      <div className="flex flex-wrap items-start gap-3">
        <span className="flex min-w-0 flex-1 flex-col gap-0.5">
          <span className="text-sm font-medium">
            {entry.name}
            {entry.starter && <span className="ml-2 text-xs text-ember">Good start</span>}
            {added > 0 && <span className="ml-2 text-xs text-ok">Added</span>}
          </span>
          <span className="text-sm text-muted">{entry.description}</span>
          <span className="text-xs text-muted">
            {entry.cost} ·{" "}
            <a href={entry.page} target="_blank" rel="noreferrer" className="text-ember hover:text-ember-hover">
              About it
            </a>
          </span>
        </span>
        {!open && (
          <Button variant={added ? "ghost" : "primary"} onClick={() => setOpen(true)}>
            {added ? "Add another" : "Add"}
          </Button>
        )}
      </div>
      {need && <RuntimeNote need={need} runtimes={runtimes} onInstall={() => onInstall(need)} />}
      {open && (
        <div className="flex flex-col gap-3 border-t border-line pt-3">
          {entry.note && <p className="text-xs text-amber">{entry.note}</p>}
          {entry.setup && (
            <ol className="ml-5 list-decimal space-y-1 text-xs text-muted">
              {entry.setup.map((step, i) => (
                <li key={i}>{step}</li>
              ))}
            </ol>
          )}
          {toolsAvailable() && <WhereChoice where={where} onChange={setWhere} pcOnly={pcOnly} />}
          {hideClient && <p className="text-xs text-muted">Your household's Google client is set up already: just sign in.</p>}
          {inputs.map((i) => (
            <Field
              key={i.key}
              label={`${i.label}${i.optional ? " (optional)" : ""}`}
              type={i.secret ? "password" : "text"}
              autoComplete="off"
              placeholder={i.placeholder}
              value={values[i.key] ?? ""}
              onChange={(e) => setValues((v) => ({ ...v, [i.key]: e.target.value }))}
              hint={
                i.help || i.link ? (
                  <>
                    {i.help}{" "}
                    {i.link && (
                      <a href={i.link} target="_blank" rel="noreferrer" className="text-ember hover:text-ember-hover">
                        Get one
                      </a>
                    )}
                  </>
                ) : undefined
              }
            />
          ))}
          <div className="flex flex-wrap items-center gap-2">
            <Button onClick={() => void add()} disabled={busy || missing.length > 0 || (!!need && !runtimes?.[need])}>
              {busy ? (entry.signIn ? "Signing in…" : "Starting it…") : entry.signIn ? "Add and sign in" : "Add and test"}
            </Button>
            <Button variant="ghost" onClick={() => setOpen(false)}>
              Cancel
            </Button>
            {missing.length > 0 && <span className="text-xs text-muted">Needs: {missing.join(", ")}</span>}
          </div>
        </div>
      )}
      {result && <p className="text-xs text-ok">{result}</p>}
      <ErrorNote>{error}</ErrorNote>
    </div>
  );
}

function ServerRow({
  token,
  server,
  runtimes,
  onInstall,
  onChanged,
}: {
  token: string;
  server: McpServer;
  runtimes: Runtimes | null;
  onInstall: (w: "node" | "uv") => void;
  onChanged: () => void;
}) {
  const [tools, setTools] = useState<McpTool[] | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [editing, setEditing] = useState(false);
  const need = runtimeFor(server);

  async function save(next: McpServer) {
    setError(null);
    try {
      await pcTools.save(next);
      onChanged();
    } catch (e) {
      setError(message(e));
    }
  }
  async function test() {
    setBusy(true);
    setError(null);
    try {
      setTools(await pcTools.tools(server.id));
    } catch (e) {
      setTools(null);
      setError(readable(message(e)));
    } finally {
      setBusy(false);
    }
  }
  async function move() {
    if (!await areYouSure(`Move ${server.name} to the Outpost server? It then works from all your devices (its keys and sign-in go with it, kept encrypted), and leaves this PC.`)) return;
    setError(null);
    try {
      await moveToServer(token, server);
      onChanged();
    } catch (e) {
      setError(`Couldn't move it: ${message(e)}`);
    }
  }
  async function remove() {
    if (!await areYouSure(`Remove ${server.name}? Its settings and keys on this PC are deleted.`)) return;
    try {
      await pcTools.remove(server.id);
      onChanged();
    } catch (e) {
      setError(message(e));
    }
  }

  return (
    <li className="flex flex-col gap-2 rounded-xl border border-line px-4 py-3">
      <div className="flex flex-wrap items-center gap-3">
        <span className="flex min-w-0 flex-1 flex-col">
          <span className="text-sm font-medium">
            {server.name}
            <span className="ml-2 text-xs text-muted">This PC</span>
          </span>
          <span className="truncate font-mono text-xs text-muted">
            {server.transport === "http"
              ? server.url
              : server.transport === "builtin"
                ? `Built into Outpost${server.env.account ? ` · ${server.env.account}` : ""}`
                : [server.command, ...server.args].join(" ")}
          </span>
        </span>
        <Toggle checked={server.enabled} label={`${server.name} on`} onChange={(on) => void save({ ...server, enabled: on })} />
        <Button variant="ghost" onClick={() => void test()} disabled={busy}>
          {busy ? "Starting…" : "Test"}
        </Button>
        <Button variant="ghost" onClick={() => setEditing((e) => !e)}>
          Edit
        </Button>
        <Button variant="ghost" onClick={() => void move()} title="Run it on the Outpost server, for all your devices">
          Move to the server
        </Button>
        <Button variant="danger" onClick={() => void remove()}>
          Remove
        </Button>
      </div>
      {need && <RuntimeNote need={need} runtimes={runtimes} onInstall={() => onInstall(need)} />}
      {server.transport === "builtin" && server.command === "google" && (
        <GoogleSettings server={server} onSave={(s) => void save(s)} />
      )}
      {(server.transport === "http" || server.transport === "builtin") &&
        (server.signed_in || server.oauth_client || needsSignIn(error) || needsClient(error)) && (
        <SignIn
          server={server}
          askClient={needsClient(error)}
          onDone={() => {
            setError(null);
            onChanged();
            void test();
          }}
          onSignedOut={onChanged}
        />
      )}
      {tools && (
        <div className="flex flex-col gap-1">
          <p className="text-xs text-ok">Working: {tools.length} tools. Switch off any you don't want chats to use.</p>
          <ul className="flex flex-col gap-1">
            {tools.map((t) => {
              const off = server.disabled_tools.includes(t.name);
              return (
                <li key={t.name} className="flex items-start gap-3 text-xs">
                  <Toggle
                    checked={!off}
                    label={`Allow ${t.name}`}
                    onChange={(on) =>
                      void save({
                        ...server,
                        disabled_tools: on ? server.disabled_tools.filter((x) => x !== t.name) : [...server.disabled_tools, t.name],
                      })
                    }
                  />
                  <span className="flex min-w-0 flex-1 flex-col">
                    <span className="font-mono">
                      {t.name}
                      {t.read_only === true && <span className="ml-2 text-muted">reads only</span>}
                    </span>
                    <span className="line-clamp-2 text-muted">{t.description}</span>
                  </span>
                </li>
              );
            })}
          </ul>
        </div>
      )}
      {editing && <EditServer server={server} onSave={(s) => void save(s).then(() => setEditing(false))} />}
      <ErrorNote>{error}</ErrorNote>
    </li>
  );
}

const GOOGLE_PARTS: [string, string][] = [
  ["drive", "Drive"],
  ["docs", "Docs"],
  ["sheets", "Sheets"],
  ["gmail", "Gmail"],
  ["calendar", "Calendar"],
];

/** What the built-in Google tools may use. Turning on more means signing in again. */
function GoogleSettings({ server, onSave }: { server: McpServer; onSave: (s: McpServer) => void }) {
  const on = (k: string) => server.args.includes(k);
  const flip = (k: string, v: boolean) => onSave({ ...server, args: v ? [...server.args, k] : server.args.filter((x) => x !== k) });
  return (
    <div className="flex flex-col gap-2 text-xs">
      <div className="flex flex-wrap gap-2">
        {GOOGLE_PARTS.map(([k, label]) => (
          <button
            key={k}
            type="button"
            aria-pressed={on(k)}
            onClick={() => flip(k, !on(k))}
            className={`min-h-8 rounded-full border px-3 ${on(k) ? "border-ember bg-raised text-ink" : "border-line text-muted hover:text-ink"}`}
          >
            {label}
          </button>
        ))}
      </div>
      <label className="flex items-center gap-2 text-muted">
        <Toggle checked={on("write")} label="Allow changes" onChange={(v) => flip("write", v)} />
        Allow changes (edit Docs and Sheets, create files, add events, write email drafts). Off: it only reads.
      </label>
      {server.signed_in && server.scope_ok === false && (
        <span className="text-amber">You've switched on more than your sign-in allows: sign in again below.</span>
      )}
    </div>
  );
}

/** Signing in to a service (OAuth): the browser opens at its page and comes back here. */
function SignIn({
  server,
  askClient,
  onDone,
  onSignedOut,
}: {
  server: McpServer;
  askClient: boolean;
  onDone: () => void;
  onSignedOut: () => void;
}) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [clientId, setClientId] = useState("");
  const [secret, setSecret] = useState("");
  const showClient = askClient || needsClient(error);
  async function go() {
    setBusy(true);
    setError(null);
    try {
      await pcTools.signIn(server.id, clientId.trim() || undefined, secret.trim() || undefined);
      onDone();
    } catch (e) {
      setError(message(e));
    } finally {
      setBusy(false);
    }
  }
  if (server.signed_in)
    return (
      <div className="flex flex-wrap items-center gap-2 text-xs">
        <span className="text-ok">Signed in</span>
        <Button variant="ghost" disabled={busy} onClick={() => void go()}>
          Sign in again
        </Button>
        <Button
          variant="ghost"
          onClick={() =>
            void pcTools
              .signOut(server.id)
              .then(onSignedOut)
              .catch((e) => setError(message(e)))
          }
        >
          Sign out
        </Button>
        <ErrorNote>{error && readable(error)}</ErrorNote>
      </div>
    );
  return (
    <div className="flex flex-col gap-2 rounded-xl border border-amber/60 px-3 py-2 text-xs">
      <span>
        {server.name} needs you to sign in. Your browser opens at its sign-in page; come back here once you've signed in.
      </span>
      {showClient && (
        <>
          <span className="text-muted">
            This service doesn't let apps register themselves: make an OAuth client for it (a "Desktop app" type, if asked)
            and enter it here.
          </span>
          <Field label="Client ID" value={clientId} onChange={(e) => setClientId(e.target.value)} autoComplete="off" />
          <Field label="Client secret (if it gave one)" type="password" value={secret} onChange={(e) => setSecret(e.target.value)} autoComplete="off" />
        </>
      )}
      <div>
        <Button disabled={busy || (showClient && !clientId.trim() && !server.oauth_client)} onClick={() => void go()}>
          {busy ? "Waiting for the browser…" : "Sign in"}
        </Button>
      </div>
      <ErrorNote>{error && !needsClient(error) ? readable(error) : null}</ErrorNote>
    </div>
  );
}

const lines = (o: Record<string, string>, sep: string) => Object.keys(o).map((k) => `${k}${sep}`).join("\n");

function parsePairs(text: string, sep: string): Record<string, string> {
  const out: Record<string, string> = {};
  for (const line of text.split("\n")) {
    const at = line.indexOf(sep);
    if (at <= 0) continue;
    out[line.slice(0, at).trim()] = line.slice(at + sep.length).trim();
  }
  return out;
}

/** Change a server: its name, how it starts, and its keys (kept ones aren't shown). */
function EditServer({ server, onSave }: { server: McpServer; onSave: (s: McpServer) => void }) {
  const [name, setName] = useState(server.name);
  const [command, setCommand] = useState(server.command);
  const [args, setArgs] = useState(server.args.join("\n"));
  const [url, setUrl] = useState(server.url);
  const [secrets, setSecrets] = useState(server.transport === "http" ? lines(server.headers, ": ") : lines(server.env, "="));
  const sep = server.transport === "http" ? ":" : "=";
  function save() {
    // A key left without a value keeps its saved value.
    const given = parsePairs(secrets, sep);
    const old = server.transport === "http" ? server.headers : server.env;
    const kept: Record<string, string> = {};
    for (const k of Object.keys(given)) kept[k] = given[k] || old[k] || "";
    for (const k of Object.keys(kept)) if (!kept[k]) delete kept[k];
    onSave({
      ...server,
      name,
      command,
      args: args.split("\n").map((a) => a.trim()).filter(Boolean),
      url,
      ...(server.transport === "http" ? { headers: kept } : { env: kept }),
    });
  }
  return (
    <div className="flex flex-col gap-3 border-t border-line pt-3">
      <Field label="Name" value={name} onChange={(e) => setName(e.target.value)} />
      {server.transport === "http" ? (
        <Field label="Address" value={url} onChange={(e) => setUrl(e.target.value)} />
      ) : (
        <>
          <Field label="Command" value={command} onChange={(e) => setCommand(e.target.value)} />
          <TextArea label="Arguments (one per line)" value={args} onChange={setArgs} />
        </>
      )}
      <TextArea
        label={server.transport === "http" ? "Headers (Name: value, one per line)" : "Settings (NAME=value, one per line)"}
        hint="Saved values aren't shown: leave a value empty to keep it, or delete the line to remove it."
        value={secrets}
        onChange={setSecrets}
      />
      <div>
        <Button onClick={save}>Save</Button>
      </div>
    </div>
  );
}

function TextArea({ label, hint, value, onChange }: { label: string; hint?: string; value: string; onChange: (v: string) => void }) {
  return (
    <label className="flex flex-col gap-1.5 text-sm text-muted">
      {label}
      <textarea
        rows={3}
        spellCheck={false}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        className="rounded-xl border border-line bg-surface px-3.5 py-2 font-mono text-sm text-ink focus:border-ember focus:outline-none"
      />
      {hint && <span className="text-xs">{hint}</span>}
    </label>
  );
}

/** Any other MCP server: a program to start, or an address. */
function CustomServer({ token, onAdded }: { token: string; onAdded: () => void }) {
  const [where, setWhere] = useState<Where>(toolsAvailable() ? "server" : "server");
  const [kind, setKind] = useState<"stdio" | "http">("stdio");
  const [name, setName] = useState("");
  const [command, setCommand] = useState("npx");
  const [args, setArgs] = useState("-y\n");
  const [env, setEnv] = useState("");
  const [url, setUrl] = useState("");
  const [headers, setHeaders] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [result, setResult] = useState<string | null>(null);
  const server = useMemo<McpServer>(
    () => ({
      id: "",
      name: name.trim(),
      catalog_id: null,
      transport: kind,
      command: kind === "stdio" ? command.trim() : "",
      args: kind === "stdio" ? args.split("\n").map((a) => a.trim()).filter(Boolean) : [],
      env: kind === "stdio" ? parsePairs(env, "=") : {},
      url: kind === "http" ? url.trim() : "",
      headers: kind === "http" ? parsePairs(headers, ":") : {},
      enabled: true,
      disabled_tools: [],
    }),
    [kind, name, command, args, env, url, headers],
  );
  async function add() {
    setBusy(true);
    setError(null);
    setResult(null);
    if (where === "server") {
      try {
        const r = await addServerTool(token, server);
        onAdded();
        if (r.problem) setError(r.problem);
        else setResult(`Added on the server: ${r.tools?.length ?? 0} tools.`);
      } catch (e) {
        setError(message(e));
      } finally {
        setBusy(false);
      }
      return;
    }
    try {
      const saved = await pcTools.save(server);
      try {
        const tools = await pcTools.tools(saved.id);
        setResult(`Added: ${tools.length} tools.`);
        onAdded();
      } catch (e) {
        setError(`Added, but it didn't start: ${readable(message(e))}`);
        onAdded();
      }
    } catch (e) {
      setError(message(e));
    } finally {
      setBusy(false);
    }
  }
  return (
    <div className="flex flex-col gap-3">
      <p className="text-sm text-muted">
        Any MCP server works: copy the command (or address) from its instructions. <code>npx</code> and <code>uvx</code> use Outpost's own
        Node and uv. One that signs in on its own page shows Sign in under Your tools once added. Programs on the server are
        for owners.
      </p>
      {toolsAvailable() && <WhereChoice where={where} onChange={setWhere} />}
      <div className="flex gap-2">
        {(
          [
            ["stdio", "A program on this PC"],
            ["http", "An address (http)"],
          ] as const
        ).map(([id, label]) => (
          <button
            key={id}
            type="button"
            aria-pressed={kind === id}
            onClick={() => setKind(id)}
            className={`min-h-9 rounded-full border px-3.5 text-sm ${kind === id ? "border-ember bg-raised text-ink" : "border-line text-muted hover:text-ink"}`}
          >
            {label}
          </button>
        ))}
      </div>
      <Field label="Name" value={name} placeholder="e.g. My notes" onChange={(e) => setName(e.target.value)} />
      {kind === "stdio" ? (
        <>
          <Field label="Command" value={command} placeholder="npx, uvx, or a program's path" onChange={(e) => setCommand(e.target.value)} />
          <TextArea label="Arguments (one per line)" value={args} onChange={setArgs} />
          <TextArea label="Settings (NAME=value, one per line)" value={env} onChange={setEnv} />
        </>
      ) : (
        <>
          <Field label="Address" value={url} placeholder="https://example.com/mcp" onChange={(e) => setUrl(e.target.value)} />
          <TextArea label="Headers (Name: value, one per line)" value={headers} onChange={setHeaders} />
        </>
      )}
      <div>
        <Button onClick={() => void add()} disabled={busy || !server.name || (kind === "stdio" ? !server.command : !server.url)}>
          {busy ? "Starting it…" : "Add and test"}
        </Button>
      </div>
      {result && <p className="text-xs text-ok">{result}</p>}
      <ErrorNote>{error}</ErrorNote>
    </div>
  );
}

/** A persona's default tools (Personas screen). */
export function PersonaTools({ token, personaId }: { token: string; personaId: string }) {
  const [servers, setServers] = useState<McpServer[]>([]);
  const [selected, setSelected] = useState<string[]>([]);
  const [onServer, setOnServer] = useState<ServerTool[]>([]);
  const [serverSelected, setServerSelected] = useState<string[]>([]);
  const [library, setLibrary] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const load = useCallback(() => {
    serverTools.list(token).then(setOnServer, () => undefined);
    serverTools.personaTools(token, personaId).then(setServerSelected, () => undefined);
    if (!toolsAvailable()) return;
    pcTools.list().then(setServers, () => undefined);
    pcTools.selection(null, personaId).then(setSelected, () => undefined);
  }, [token, personaId]);
  useEffect(load, [load]);
  const set = (ids: string[]) => {
    setSelected(ids);
    pcTools.selectPersona(personaId, ids).catch((e) => setError(message(e)));
  };
  const setServer = (ids: string[]) => {
    setServerSelected(ids);
    serverTools.setPersonaTools(token, personaId, ids).then(setServerSelected, (e) => setError(message(e)));
  };
  const chip = (id: string, name: string, on: boolean, flip: () => void) => (
    <button
      key={id}
      type="button"
      aria-pressed={on}
      onClick={flip}
      className={`min-h-9 rounded-full border px-3.5 text-sm ${on ? "border-ember bg-raised text-ink" : "border-line text-muted hover:text-ink"}`}
    >
      {name}
    </button>
  );
  return (
    <div className="flex flex-col gap-2">
      <span className="text-sm text-muted">Tools (new chats with this persona start with these)</span>
      {onServer.length > 0 && (
        <div className="flex flex-wrap gap-2">
          {onServer.map((s) => {
            const on = serverSelected.includes(s.id);
            return chip(s.id, s.name, on, () => setServer(on ? serverSelected.filter((x) => x !== s.id) : [...serverSelected, s.id]));
          })}
        </div>
      )}
      {toolsAvailable() && servers.length > 0 && <span className="text-xs text-muted">On this PC:</span>}
      {servers.length === 0 && onServer.length === 0 ? (
        <span className="text-sm">None yet.</span>
      ) : (
        <div className="flex flex-wrap gap-2">
          {servers.map((s) => {
            const on = selected.includes(s.id);
            return (
              <button
                key={s.id}
                type="button"
                aria-pressed={on}
                onClick={() => set(on ? selected.filter((x) => x !== s.id) : [...selected, s.id])}
                className={`min-h-9 rounded-full border px-3.5 text-sm ${on ? "border-ember bg-raised text-ink" : "border-line text-muted hover:text-ink"}`}
              >
                {s.name}
              </button>
            );
          })}
        </div>
      )}
      <div>
        <Button variant="ghost" onClick={() => setLibrary(true)}>
          Tool library
        </Button>
      </div>
      <ErrorNote>{error}</ErrorNote>
      {library && (
        <ToolLibrary
          token={token}
          onClose={() => {
            setLibrary(false);
            load();
          }}
        />
      )}
    </div>
  );
}
