import { useEffect, useMemo, useState, type ReactNode } from "react";
import {
  api,
  GATEWAY,
  SERVER,
  integrationsApi,
  type McpTool,
  type NetworkStatus,
  type OpenApiDoc,
  type Persona,
  type SignInResponse,
} from "../api";
import { Button } from "../components/ui";

/** A block of text to copy, with a Copy button. */
function Copy({ text, label }: { text: string; label: string }) {
  const [done, setDone] = useState(false);
  return (
    <div className="relative">
      <pre
        aria-label={label}
        className="overflow-auto rounded-xl border border-line bg-deep p-3 pr-20 font-mono text-xs leading-relaxed whitespace-pre text-ink"
      >
        {text}
      </pre>
      <button
        type="button"
        onClick={() => {
          void navigator.clipboard.writeText(text).then(() => {
            setDone(true);
            window.setTimeout(() => setDone(false), 1500);
          });
        }}
        className="absolute top-2 right-2 rounded-lg border border-line bg-surface px-2.5 py-1 text-xs hover:bg-raised"
      >
        {done ? "Copied" : "Copy"}
      </button>
    </div>
  );
}

function Section({ title, children, blurb }: { title: string; blurb?: string; children: ReactNode }) {
  return (
    <section className="flex flex-col gap-3">
      <div>
        <h2 className="text-lg font-semibold">{title}</h2>
        {blurb && <p className="text-sm text-muted">{blurb}</p>}
      </div>
      {children}
    </section>
  );
}

const METHOD_STYLE: Record<string, string> = {
  get: "text-ok",
  post: "text-ember",
  put: "text-pick",
  patch: "text-pick",
  delete: "text-danger",
};

/** Everything an app needs to connect: addresses, keys, OpenAI and MCP setup, and every endpoint. */
export default function Api({ auth, onOpenConnections }: { auth: SignInResponse; onOpenConnections: () => void }) {
  const token = auth.token;
  const [network, setNetwork] = useState<NetworkStatus | null>(null);
  const [context, setContext] = useState(65536);
  const [port, setPort] = useState(7420);
  const [bridge, setBridge] = useState<string | null>(null);
  const [personas, setPersonas] = useState<Persona[]>([]);
  const [tools, setTools] = useState<McpTool[]>([]);
  const [doc, setDoc] = useState<OpenApiDoc | null>(null);
  const [filter, setFilter] = useState("");

  useEffect(() => {
    api.network(token).then(setNetwork, () => undefined);
    integrationsApi.get(token).then(
      (r) => {
        setPort(r.port);
        setBridge(r.bridge_path);
      },
      () => undefined,
    );
    api.personas(token).then(setPersonas, () => undefined);
    integrationsApi.mcpTools(token).then(setTools, () => undefined);
    integrationsApi.openapi().then(setDoc, () => undefined);
    // The window Ollama runs with here (smaller on smaller cards).
    api.models(token).then((r) => {
      const c = r.data.find((m) => m.context_length)?.context_length;
      if (c) setContext(c);
    }, () => undefined);
  }, [token]);

  // Where other apps reach Outpost: this PC, the home network, Tailscale.
  const bases = useMemo(() => {
    const out: { where: string; url: string }[] = [{ where: SERVER ? "Apps on the server itself" : "Apps on this PC", url: `http://127.0.0.1:${port}` }];
    if (network?.lan_access && network.lan_address) out.push({ where: "Your home network", url: `http://${network.lan_address}:${port}` });
    if (network?.tailscale_running && network.tailscale)
      out.push({ where: "Your Tailscale devices", url: `http://${network.tailscale.dns_name ?? network.tailscale.ip}:${port}` });
    return out;
  }, [network, port]);
  const base = bases[bases.length > 1 ? 1 : 0].url;
  const slug = personas[0]?.slug ?? "assistant";

  const groups = useMemo(() => {
    if (!doc) return [];
    const byTag = new Map<string, { method: string; path: string; summary: string; description: string; params: string[]; body: boolean }[]>();
    for (const [path, ops] of Object.entries(doc.paths)) {
      for (const [method, op] of Object.entries(ops)) {
        const tag = op.tags?.[0] ?? "other";
        const text = `${method} ${path} ${op.summary ?? ""} ${op.description ?? ""}`.toLowerCase();
        if (filter && !text.includes(filter.toLowerCase())) continue;
        const list = byTag.get(tag) ?? [];
        list.push({
          method,
          path,
          summary: op.summary ?? op.description?.split("\n")[0] ?? "",
          description: op.description ?? "",
          params: (op.parameters ?? []).map((p) => `${p.name} (${p.in}${p.required ? ", required" : ""})${p.description ? `: ${p.description}` : ""}`),
          body: !!op.requestBody,
        });
        byTag.set(tag, list);
      }
    }
    const order = (doc.tags ?? []).map((t) => t.name);
    return [...byTag.entries()]
      .sort((a, b) => (order.indexOf(a[0]) + 1 || 99) - (order.indexOf(b[0]) + 1 || 99))
      .map(([tag, ops]) => ({ tag, about: doc.tags?.find((t) => t.name === tag)?.description, ops }));
  }, [doc, filter]);

  const claudeConfig = JSON.stringify(
    {
      mcpServers: {
        outpost: {
          command: bridge ?? "C:\\path\\to\\outpost-gateway.exe",
          args: ["--mcp-stdio"],
          env: { OUTPOST_KEY: "op_your_app_key" },
        },
      },
    },
    null,
    2,
  );

  return (
    <section className="mx-auto flex max-w-4xl flex-col gap-10">
      <div>
        <h1 className="text-2xl font-semibold">API</h1>
        <p className="text-sm text-muted">
          Everything in Outpost has an endpoint, so other apps can use it too: Hermes, Open WebUI, Claude Desktop, n8n or
          your own scripts. They sign in with an app key.
        </p>
      </div>

      <Section title="Where apps connect" blurb="Turn on network or Tailscale access in Connections for apps on other machines.">
        <ul className="flex flex-col divide-y divide-line rounded-2xl border border-line bg-surface">
          {bases.map((b) => (
            <li key={b.url} className="flex flex-wrap items-center gap-3 px-4 py-3">
              <span className="w-48 text-sm text-muted">{b.where}</span>
              <span className="flex-1 font-mono text-sm">{b.url}</span>
            </li>
          ))}
        </ul>
        <div className="flex flex-wrap items-center gap-3 text-sm">
          <span className="text-muted">Apps need an app key (it starts with op_).</span>
          <Button variant="ghost" onClick={onOpenConnections}>
            Make one in Connections
          </Button>
        </div>
      </Section>

      <Section
        title="Chat apps (OpenAI-compatible)"
        blurb={`Point any app that speaks the OpenAI API at ${base}/v1. Use persona:<id> as the model to get that persona's model, prompt and memory.`}
      >
        <Copy
          label="Hermes config"
          text={`# Hermes config.yaml\nproviders:\n  outpost:\n    api: ${base}/v1\n    key: op_your_app_key\n    model: persona:${slug}\n    context_length: ${context}`}
        />
        <Copy
          label="curl chat example"
          text={`curl ${base}/v1/chat/completions \\\n  -H "Authorization: Bearer op_your_app_key" \\\n  -H "Content-Type: application/json" \\\n  -d '{"model": "persona:${slug}", "messages": [{"role": "user", "content": "Hello"}]}'`}
        />
        {personas.length > 0 && (
          <p className="text-xs text-muted">
            Your personas: {personas.map((p) => `persona:${p.slug}`).join(", ")}
          </p>
        )}
      </Section>

      <Section
        title="MCP (Claude Desktop and other assistants)"
        blurb="Outpost is an MCP server: assistants can ask your personas, draw pictures (your characters too) and save memories, all on this PC."
      >
        <p className="text-sm">
          <b>Apps that connect over HTTP</b> (Hermes, many agents): the address and your app key.
        </p>
        <Copy label="MCP over HTTP" text={`URL:     ${base}/mcp\nHeader:  Authorization: Bearer op_your_app_key`} />
        <p className="text-sm">
          <b>Claude Desktop</b> starts local programs, so Outpost's own program acts as the bridge. Add this to{" "}
          <span className="font-mono">claude_desktop_config.json</span> (Settings › Developer › Edit config), with your key, then restart
          Claude Desktop. Outpost must be running.
        </p>
        <Copy label="Claude Desktop config" text={claudeConfig} />
        {!bridge && (
          <p className="text-xs text-warn">
            Outpost's program wasn't found next to this app. Build it with <span className="font-mono">cargo build --release -p outpost-gateway</span>,
            then use <span className="font-mono">target\release\outpost-gateway.exe</span>.
          </p>
        )}
        {tools.length > 0 && (
          <ul className="grid gap-2 sm:grid-cols-2">
            {tools.map((t) => (
              <li key={t.name} className="rounded-xl border border-line bg-surface px-3 py-2">
                <span className="font-mono text-sm text-ember">{t.name}</span>
                <span className="block text-xs text-muted">{t.description}</span>
              </li>
            ))}
          </ul>
        )}
      </Section>

      <Section title="Every endpoint" blurb={`From the live OpenAPI document at ${SERVER ? `${SERVER.name}'s /openapi.json` : `${GATEWAY}/openapi.json`}.`}>
        <input
          aria-label="Find an endpoint"
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
          placeholder="Find an endpoint, e.g. gallery or train"
          className="min-h-11 rounded-xl border border-line bg-surface px-3.5 text-ink placeholder:text-muted/60 focus:border-ember focus:outline-none"
        />
        {!doc && <p className="text-sm text-muted">Loading…</p>}
        {groups.map((g) => (
          <div key={g.tag} className="flex flex-col gap-1">
            <h3 className="mt-2 text-sm font-semibold capitalize">{g.tag}</h3>
            {g.about && <p className="text-xs text-muted">{g.about}</p>}
            <ul className="flex flex-col divide-y divide-line rounded-2xl border border-line bg-surface">
              {g.ops.map((op) => (
                <li key={op.method + op.path}>
                  <details className="group px-4 py-2">
                    <summary className="flex cursor-pointer items-baseline gap-3">
                      <span className={`w-14 shrink-0 font-mono text-xs font-semibold uppercase ${METHOD_STYLE[op.method] ?? ""}`}>{op.method}</span>
                      <span className="font-mono text-sm">{op.path}</span>
                      <span className="truncate text-xs text-muted">{op.summary}</span>
                    </summary>
                    <div className="mt-2 flex flex-col gap-2 pb-2 text-sm">
                      {op.description && op.description !== op.summary && <p className="whitespace-pre-wrap text-muted">{op.description}</p>}
                      {op.params.length > 0 && (
                        <ul className="list-disc pl-5 text-xs text-muted">
                          {op.params.map((p) => (
                            <li key={p}>{p}</li>
                          ))}
                        </ul>
                      )}
                      <Copy
                        label="curl"
                        text={`curl -X ${op.method.toUpperCase()} "${base}${op.path}" \\\n  -H "Authorization: Bearer op_your_app_key"${
                          op.body ? ` \\\n  -H "Content-Type: application/json" \\\n  -d '{ }'` : ""
                        }`}
                      />
                    </div>
                  </details>
                </li>
              ))}
            </ul>
          </div>
        ))}
      </Section>
    </section>
  );
}
