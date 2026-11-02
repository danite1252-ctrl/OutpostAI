// Tools on this PC for chats: MCP servers (web search, GitHub, a browser...) that the app runs
// here. A chat uses its persona's tools unless it's given its own set. The library lists ones
// that are known to work; any other MCP server can be added by hand.

import { useCallback, useEffect, useMemo, useState, type ReactNode } from "react";
import { pcTools, type McpServer, type McpTool, type MessageTools, type Runtimes } from "../api";
import { CATALOG, CATEGORIES, fromCatalog, missingInputs, runtimeFor, type CatalogEntry } from "../mcpCatalog";
import { Button, ErrorNote, Field, Toggle } from "./ui";

/** Tools work only in the desktop app (they run on this PC). */
export const toolsAvailable = () => typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

function message(e: unknown) {
  if (typeof e === "string") return e;
  if (e instanceof Error) return e.message;
  return "Something went wrong.";
}

/** Strip the "needs node:" marker the app uses to say what to install. */
function readable(e: string) {
  return e.replace(/^needs (node|uv): /, "");
}

export function useChatTools(chatId: string | null, personaId: string | null) {
  const [servers, setServers] = useState<McpServer[]>([]);
  const [selected, setSelected] = useState<string[]>([]);
  const reload = useCallback(async () => {
    if (!toolsAvailable()) return;
    const [list, sel] = await Promise.all([pcTools.list().catch(() => []), pcTools.selection(chatId, personaId).catch(() => [])]);
    setServers(list);
    setSelected(sel);
  }, [chatId, personaId]);
  useEffect(() => {
    void reload();
  }, [reload]);
  const active = servers.filter((s) => s.enabled && selected.includes(s.id));
  /** The selected servers' tools, to send with a message (they start if need be). */
  const forMessage = useCallback(async (): Promise<MessageTools[]> => {
    if (!toolsAvailable() || active.length === 0) return [];
    return pcTools.messageTools(active.map((s) => s.id));
  }, [active]);
  return { servers, selected, setSelected, active, reload, forMessage };
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
  tools,
  chatId,
  personaName,
  ensureChat,
  onClose,
}: {
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

  return (
    <Drawer label="Tools for this chat" onClose={onClose}>
      <p className="text-sm text-muted">
        Tools run on this PC, while this app is open. The chat's model needs the tools badge to use them.
        {personaName ? ` New chats with ${personaName} start with the tools set in Personas.` : ""}
      </p>
      {tools.servers.length === 0 ? (
        <p className="text-sm">No tools on this PC yet. Add some from the library: web search, GitHub, a browser and more.</p>
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
        {chatId && tools.servers.length > 0 && (
          <Button variant="ghost" onClick={() => void choose(null)} title="Use the persona's tools again">
            Use the persona's tools
          </Button>
        )}
      </div>
      <ErrorNote>{error}</ErrorNote>
      {library && (
        <ToolLibrary
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
export function ToolLibrary({ onClose }: { onClose: () => void }) {
  const [tab, setTab] = useState<"library" | "mine" | "custom">("library");
  const [servers, setServers] = useState<McpServer[]>([]);
  const [error, setError] = useState<string | null>(null);
  const rt = useRuntimes();
  const reload = useCallback(() => pcTools.list().then(setServers, (e) => setError(message(e))), []);
  useEffect(() => {
    void reload();
  }, [reload]);

  return (
    <Drawer label="Tool library" onClose={onClose} wide>
      <p className="text-sm text-muted">
        Tools let chats search the web, read pages, work with GitHub, drive a browser and more. They run on this PC; keys you
        enter stay on this PC.
      </p>
      <div className="flex gap-2" role="tablist">
        {(
          [
            ["library", "Library"],
            ["mine", `On this PC${servers.length ? ` (${servers.length})` : ""}`],
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
                  entry={e}
                  added={servers.filter((s) => s.catalog_id === e.id).length}
                  runtimes={rt.runtimes}
                  onInstall={rt.install}
                  onAdded={() => {
                    void reload();
                  }}
                />
              ))}
            </section>
          ))}
        </div>
      )}
      {tab === "mine" &&
        (servers.length === 0 ? (
          <p className="text-sm">Nothing yet: add tools from the Library tab.</p>
        ) : (
          <ul className="flex flex-col gap-3">
            {servers.map((s) => (
              <ServerRow key={s.id} server={s} runtimes={rt.runtimes} onInstall={rt.install} onChanged={reload} />
            ))}
          </ul>
        ))}
      {tab === "custom" && (
        <CustomServer
          onAdded={() => {
            void reload();
            setTab("mine");
          }}
        />
      )}
    </Drawer>
  );
}

function CatalogCard({
  entry,
  added,
  runtimes,
  onInstall,
  onAdded,
}: {
  entry: CatalogEntry;
  added: number;
  runtimes: Runtimes | null;
  onInstall: (w: "node" | "uv") => void;
  onAdded: () => void;
}) {
  const [open, setOpen] = useState(false);
  const [values, setValues] = useState<Record<string, string>>({});
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const need = runtimeFor(entry);
  const missing = missingInputs(entry, values);

  async function add() {
    setBusy(true);
    setError(null);
    setResult(null);
    try {
      const saved = await pcTools.save(fromCatalog(entry, values) as McpServer);
      onAdded();
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
          {entry.inputs.map((i) => (
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
              {busy ? "Starting it…" : "Add and test"}
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
  server,
  runtimes,
  onInstall,
  onChanged,
}: {
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
  async function remove() {
    if (!window.confirm(`Remove ${server.name}? Its settings and keys on this PC are deleted.`)) return;
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
          <span className="text-sm font-medium">{server.name}</span>
          <span className="truncate font-mono text-xs text-muted">
            {server.transport === "http" ? server.url : [server.command, ...server.args].join(" ")}
          </span>
        </span>
        <Toggle checked={server.enabled} label={`${server.name} on`} onChange={(on) => void save({ ...server, enabled: on })} />
        <Button variant="ghost" onClick={() => void test()} disabled={busy}>
          {busy ? "Starting…" : "Test"}
        </Button>
        <Button variant="ghost" onClick={() => setEditing((e) => !e)}>
          Edit
        </Button>
        <Button variant="danger" onClick={() => void remove()}>
          Remove
        </Button>
      </div>
      {need && <RuntimeNote need={need} runtimes={runtimes} onInstall={() => onInstall(need)} />}
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
function CustomServer({ onAdded }: { onAdded: () => void }) {
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
        Node and uv. Servers that need a sign-in page (OAuth) aren't supported yet; use one that takes a key.
      </p>
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
export function PersonaTools({ personaId }: { personaId: string }) {
  const [servers, setServers] = useState<McpServer[]>([]);
  const [selected, setSelected] = useState<string[]>([]);
  const [library, setLibrary] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const load = useCallback(() => {
    if (!toolsAvailable()) return;
    pcTools.list().then(setServers, () => undefined);
    pcTools.selection(null, personaId).then(setSelected, () => undefined);
  }, [personaId]);
  useEffect(load, [load]);
  if (!toolsAvailable()) return null;
  const set = (ids: string[]) => {
    setSelected(ids);
    pcTools.selectPersona(personaId, ids).catch((e) => setError(message(e)));
  };
  return (
    <div className="flex flex-col gap-2">
      <span className="text-sm text-muted">Tools on this PC (new chats with this persona start with these)</span>
      {servers.length === 0 ? (
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
          onClose={() => {
            setLibrary(false);
            load();
          }}
        />
      )}
    </div>
  );
}
