import entries from "../../../crates/outpost-gateway/tool-catalog.json";

// The tool library: MCP servers that work with Outpost's chats, checked November 2026.
// Each runs on this PC: `npx` ones with Node, `uvx` ones with Python (through uv); Outpost
// downloads either if the PC hasn't got it. Address ones (http) are services on the internet
// or the home network.

export interface CatalogInput {
  key: string;
  label: string;
  /** Where the value goes: an environment variable, a header, command-line arguments, the
   * address, or (`client_id`/`client_secret`) the OAuth client used to sign in. */
  kind: "env" | "header" | "arg" | "url" | "client_id" | "client_secret";
  /** env / header name. */
  name?: string;
  /** For a header: how the value is written, e.g. "Bearer {value}". */
  format?: string;
  /** For arguments: what's added when it's filled in, e.g. ["--repository", "{value}"]. */
  args?: string[];
  secret?: boolean;
  optional?: boolean;
  placeholder?: string;
  help?: string;
  /** Where to get it. */
  link?: string;
  /** Filled in to start with. */
  value?: string;
}

export interface CatalogEntry {
  id: string;
  name: string;
  category: "Search & the web" | "Office & files" | "Coding" | "Knowledge" | "Home & tasks" | "Helpers";
  description: string;
  transport: "stdio" | "http" | "builtin";
  command?: string;
  args?: string[];
  url?: string;
  inputs: CatalogInput[];
  /** Free, needs a key, etc. */
  cost: string;
  page: string;
  /** Something to know before relying on it. */
  note?: string;
  /** Picked as a good first tool. */
  starter?: boolean;
  /** Signs in with the service's own page when added (OAuth). */
  signIn?: boolean;
  /** Steps to do first (e.g. making an OAuth client). */
  setup?: string[];
  /** Scopes to ask for when signing in (services that don't publish theirs). */
  oauthScope?: string;
}

type RawEntry = CatalogEntry & { windows?: { args?: string[]; note?: string; placeholders?: Record<string, string> } };

const windows = typeof navigator !== "undefined" && /Windows/i.test(navigator.userAgent);

/** The library, shared with the server (which hands it to the phone): crates/outpost-gateway/tool-catalog.json.
 *  Entries with a `windows` part use it on Windows (another browser, Windows-style example paths). */
export const CATALOG: CatalogEntry[] = (entries as unknown as RawEntry[]).map(({ windows: w, ...e }) =>
  !windows || !w
    ? e
    : {
        ...e,
        args: w.args ?? e.args,
        note: w.note ?? e.note,
        inputs: e.inputs.map((i) => (w.placeholders?.[i.key] ? { ...i, placeholder: w.placeholders[i.key] } : i)),
      },
);

export const CATEGORIES: CatalogEntry["category"][] = ["Search & the web", "Office & files", "Coding", "Knowledge", "Home & tasks", "Helpers"];

/** The runtime an entry needs on this PC. */
export function runtimeFor(e: { transport: string; command?: string }): "node" | "uv" | null {
  if (e.transport !== "stdio") return null;
  if (e.command === "npx" || e.command === "node") return "node";
  if (e.command === "uvx" || e.command === "uv") return "uv";
  return null;
}

/** A library entry, filled in, as a server to save. */
export function fromCatalog(e: CatalogEntry, values: Record<string, string>) {
  const env: Record<string, string> = {};
  const headers: Record<string, string> = {};
  const args = [...(e.args ?? [])];
  let url = e.url ?? "";
  for (const input of e.inputs) {
    const v = (values[input.key] ?? "").trim();
    if (!v) continue;
    if (input.kind === "env" && input.name) env[input.name] = v;
    else if (input.kind === "header" && input.name) headers[input.name] = (input.format ?? "{value}").replace("{value}", v);
    else if (input.kind === "arg") args.push(...(input.args ?? ["{value}"]).map((a) => a.replace("{value}", v)));
    else if (input.kind === "url") url = url.replace(`{${input.key}}`, v.replace(/\/+$/, ""));
    // client_id / client_secret go to the sign-in, not the server's settings.
  }
  return {
    id: "",
    name: e.name,
    catalog_id: e.id,
    transport: e.transport,
    command: e.command ?? "",
    args,
    env,
    url,
    headers,
    enabled: true,
    disabled_tools: [] as string[],
  };
}

/** What's missing before an entry can be added. */
export function missingInputs(e: CatalogEntry, values: Record<string, string>): string[] {
  return e.inputs.filter((i) => !i.optional && !(values[i.key] ?? "").trim()).map((i) => i.label);
}
