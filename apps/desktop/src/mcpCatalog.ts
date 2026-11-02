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

const GOOGLE_SETUP = [
  "Needs a Google Workspace account (Google's MCP servers are a developer preview; personal Gmail accounts can't enrol yet).",
  "In Google Cloud (console.cloud.google.com), make or pick a project, and enrol it in the Workspace Developer Preview (developers.google.com/workspace/preview). Google takes a couple of days.",
  "Enable the service's API and its MCP service in that project (e.g. Google Drive API and Drive MCP API).",
  "Google Auth Platform › Audience: Internal (or External with your accounts as test users). Data Access: add the scopes this tool asks for.",
  "Clients › Create client: type Web application, tick 'Use this client for an AI-powered agent', and add this redirect address exactly: http://127.0.0.1:33418/callback",
  "Copy its client ID and secret below (the secret is shown once). One client works for all the Google tools.",
];

const googleInputs: CatalogInput[] = [
  { key: "client_id", label: "OAuth client ID", kind: "client_id", placeholder: "…apps.googleusercontent.com" },
  { key: "client_secret", label: "Client secret", kind: "client_secret", secret: true },
];

function google(id: string, name: string, description: string, host: string, scopes: string[]): CatalogEntry {
  return {
    id: `google-${id}`,
    name,
    category: "Office & files",
    description,
    transport: "http",
    url: `https://${host}.googleapis.com/mcp/v1`,
    inputs: googleInputs,
    cost: "Free with Google Workspace (developer preview)",
    page: "https://developers.google.com/workspace/guides/configure-mcp-servers",
    signIn: true,
    oauthScope: scopes.map((s) => `https://www.googleapis.com/auth/${s}`).join(" "),
    setup: GOOGLE_SETUP,
    note: "Google's own servers: the most capable, but setting them up takes a while (steps below). For a quicker start with a personal Google account, use Google Sheets (robot account).",
  };
}

const windows = typeof navigator !== "undefined" && /Windows/i.test(navigator.userAgent);

export const CATALOG: CatalogEntry[] = [
  {
    id: "duckduckgo",
    name: "DuckDuckGo Search",
    category: "Search & the web",
    description: "Search the web and read the pages it finds. No account or key needed.",
    transport: "stdio",
    command: "uvx",
    args: ["duckduckgo-mcp-server"],
    inputs: [
      { key: "region", label: "Region", kind: "env", name: "DDG_REGION", optional: true, placeholder: "au-en" },
      { key: "safe", label: "Safe search", kind: "env", name: "DDG_SAFE_SEARCH", optional: true, placeholder: "MODERATE (or STRICT, OFF)" },
    ],
    cost: "Free, no key (up to 30 searches a minute)",
    page: "https://github.com/nickclyde/duckduckgo-mcp-server",
    starter: true,
  },
  {
    id: "fetch",
    name: "Fetch",
    category: "Search & the web",
    description: "Read any web page you give it (as text), a part at a time for long pages.",
    transport: "stdio",
    command: "uvx",
    args: ["mcp-server-fetch"],
    inputs: [],
    cost: "Free",
    page: "https://github.com/modelcontextprotocol/servers/tree/main/src/fetch",
    starter: true,
  },
  {
    id: "exa",
    name: "Exa Search",
    category: "Search & the web",
    description: "Search the web by meaning (good for research), and read pages.",
    transport: "http",
    url: "https://mcp.exa.ai/mcp",
    inputs: [
      { key: "key", label: "Exa API key", kind: "header", name: "x-api-key", secret: true, optional: true, help: "Works without one, with lower limits.", link: "https://dashboard.exa.ai" },
    ],
    cost: "Free with limits; a key raises them",
    page: "https://github.com/exa-labs/exa-mcp-server",
  },
  {
    id: "brave-search",
    name: "Brave Search",
    category: "Search & the web",
    description: "Web, news, image and local search from Brave's own index.",
    transport: "stdio",
    command: "npx",
    args: ["-y", "@brave/brave-search-mcp-server", "--transport", "stdio"],
    inputs: [{ key: "key", label: "Brave Search API key", kind: "env", name: "BRAVE_API_KEY", secret: true, link: "https://api-search.brave.com" }],
    cost: "Needs a Brave Search API key (a small monthly credit; check their pricing)",
    page: "https://github.com/brave/brave-search-mcp-server",
  },
  {
    id: "firecrawl",
    name: "Firecrawl",
    category: "Search & the web",
    description: "Turn web pages into clean text, search, and map whole sites.",
    transport: "http",
    url: "https://mcp.firecrawl.dev/v2/mcp",
    inputs: [
      { key: "key", label: "Firecrawl API key", kind: "header", name: "Authorization", format: "Bearer {value}", secret: true, optional: true, help: "Reading and searching work without one (rate-limited); crawling needs one.", link: "https://firecrawl.dev" },
    ],
    cost: "Free with limits; a key for more",
    page: "https://github.com/firecrawl/firecrawl-mcp-server",
  },
  {
    id: "playwright",
    name: "Web browser",
    category: "Search & the web",
    description: "Drive a real browser (hidden): open pages, click, fill in forms, read what's on screen.",
    transport: "stdio",
    command: "npx",
    args: ["-y", "@playwright/mcp@latest", "--headless", "--isolated", ...(windows ? ["--browser=msedge"] : ["--browser=chromium"])],
    inputs: [],
    cost: "Free",
    page: "https://github.com/microsoft/playwright-mcp",
    note: windows ? "Uses Microsoft Edge, which is already on Windows." : "May need its browser installed first (npx playwright install chromium).",
  },
  // ---- Office & files
  {
    id: "google",
    name: "Google (Drive, Docs, Sheets, Gmail, Calendar)",
    category: "Office & files",
    description:
      "Built into Outpost: search and read your Drive, read and edit Docs and Sheets, search mail and write drafts (never sent), see and add calendar events. Works with personal Google accounts.",
    transport: "builtin",
    command: "google",
    args: ["drive", "docs", "sheets", "gmail", "calendar", "write"],
    inputs: [
      { key: "client_id", label: "OAuth client ID", kind: "client_id", placeholder: "…apps.googleusercontent.com" },
      { key: "client_secret", label: "Client secret", kind: "client_secret", secret: true },
    ],
    cost: "Free (your own Google Cloud project, made once)",
    page: "https://console.cloud.google.com/",
    signIn: true,
    starter: true,
    setup: [
      "Go to console.cloud.google.com and sign in with your Google account. Make a new project (top bar › New project), e.g. \"Outpost\".",
      "APIs & Services › Library: search for and Enable each of Google Drive API, Google Docs API, Google Sheets API, Gmail API and Google Calendar API.",
      "Google Auth Platform (or OAuth consent screen) › Get started: app name Outpost, your email, Audience: External. Then Audience › Publish app (In production), so sign-ins don't expire every 7 days.",
      "Clients › Create client › Application type: Desktop app. Copy its client ID and client secret into the boxes below.",
      "When you sign in, Google says it hasn't verified the app (it's your own): click Advanced › Go to Outpost, and tick what it may use.",
      "Once made, the same client works for everyone in your household and on all your PCs (up to 100 people).",
    ],
    note: "Choose which parts it can use, and whether it can change things, under On this PC after adding it.",
  },
  {
    id: "notion-signin",
    name: "Notion",
    category: "Office & files",
    description: "Search, read and edit your Notion pages and databases. Sign in with Notion.",
    transport: "http",
    url: "https://mcp.notion.com/mcp",
    inputs: [],
    cost: "Free with a Notion account (some tools need a paid plan)",
    page: "https://developers.notion.com/docs/mcp",
    signIn: true,
    starter: true,
  },
  {
    id: "airtable",
    name: "Airtable",
    category: "Office & files",
    description: "Read and write your Airtable bases and records (you pick which bases when signing in).",
    transport: "http",
    url: "https://mcp.airtable.com/mcp",
    inputs: [],
    cost: "Free with an Airtable account",
    page: "https://airtable.com/developers/agents/mcp",
    signIn: true,
  },
  {
    id: "canva",
    name: "Canva",
    category: "Office & files",
    description: "Make, find, edit and export Canva designs.",
    transport: "http",
    url: "https://mcp.canva.com/mcp",
    inputs: [],
    cost: "Free with a Canva account (some tools need Pro)",
    page: "https://www.canva.dev/docs/apps/mcp/",
    signIn: true,
  },
  {
    id: "ms365",
    name: "Microsoft 365 / Outlook.com",
    category: "Office & files",
    description: "Outlook mail and calendar, OneDrive, Excel, OneNote, To Do and contacts.",
    transport: "stdio",
    command: "npx",
    args: ["-y", "@softeria/ms-365-mcp-server"],
    inputs: [
      {
        key: "tenant",
        label: "Account type",
        kind: "env",
        name: "MS365_MCP_TENANT_ID",
        value: "consumers",
        help: "consumers for a personal Microsoft account (Outlook.com, Hotmail); common for a work or school one.",
      },
      { key: "readonly", label: "Read only", kind: "env", name: "READ_ONLY", optional: true, placeholder: "true", help: "Type true so it can't send, change or delete anything." },
    ],
    cost: "Free",
    page: "https://github.com/Softeria/ms-365-mcp-server",
    note: "Signs in with a code: the first time, ask the chat to log in to Microsoft. It gives a link and a code to enter in your browser; then say you're done and it carries on.",
  },
  {
    id: "google-sheets-sa",
    name: "Google Sheets (robot account)",
    category: "Office & files",
    description: "Create, read and edit spreadsheets in one Google Drive folder you share with it. Works with a personal Google account.",
    transport: "stdio",
    command: "uvx",
    args: ["mcp-google-sheets@latest"],
    inputs: [
      { key: "key", label: "Service account key file", kind: "env", name: "SERVICE_ACCOUNT_PATH", placeholder: windows ? "C:\\Users\\you\\Documents\\outpost-sheets.json" : "/home/me/outpost-sheets.json" },
      { key: "folder", label: "Drive folder ID", kind: "env", name: "DRIVE_FOLDER_ID", help: "The last part of the folder's address in Drive (after /folders/)." },
    ],
    cost: "Free",
    page: "https://github.com/xing5/mcp-google-sheets",
    setup: [
      "In Google Cloud (console.cloud.google.com), make a project, and under APIs & Services › Library enable the Google Sheets API and Google Drive API.",
      "IAM & Admin › Service Accounts: create one. Then Keys › Add key › JSON: save the file somewhere on this PC.",
      "In Google Drive, make a folder for it, and share it (as Editor) with the service account's email (in the JSON file, client_email).",
      "Enter the key file's path and the folder's ID below.",
    ],
  },
  google("drive", "Google Drive (Google)", "Search, read, create and copy your Drive files.", "drivemcp", ["drive.readonly", "drive.file"]),
  google("docs", "Google Docs (Google)", "Read and write your Google Docs.", "docsmcp", ["drive.readonly", "drive.file", "documents"]),
  google("sheets", "Google Sheets (Google)", "Read and write your Google Sheets.", "sheetsmcp", ["drive.readonly", "drive.file", "spreadsheets"]),
  google("gmail", "Gmail (Google)", "Search and read your mail, and write drafts.", "gmailmcp", ["gmail.readonly", "gmail.compose"]),
  google("calendar", "Google Calendar (Google)", "Your calendars, events and when you're free.", "calendarmcp", [
    "calendar.calendarlist.readonly",
    "calendar.events.freebusy",
    "calendar.events.readonly",
  ]),
  // ---- Coding
  {
    id: "linear",
    name: "Linear",
    category: "Coding",
    description: "Issues, projects and comments in Linear.",
    transport: "http",
    url: "https://mcp.linear.app/mcp",
    inputs: [],
    cost: "Free with a Linear account",
    page: "https://linear.app/docs/mcp",
    signIn: true,
  },
  {
    id: "atlassian",
    name: "Jira and Confluence",
    category: "Coding",
    description: "Search and edit Jira issues and Confluence pages (Atlassian Cloud).",
    transport: "http",
    url: "https://mcp.atlassian.com/v2/mcp",
    inputs: [],
    cost: "With an Atlassian Cloud account",
    page: "https://support.atlassian.com/atlassian-rovo-mcp-server/docs/getting-started-with-the-atlassian-remote-mcp-server/",
    signIn: true,
  },
  {
    id: "supabase",
    name: "Supabase",
    category: "Coding",
    description: "Your Supabase projects: tables, SQL, edge functions, logs and docs.",
    transport: "http",
    url: "https://mcp.supabase.com/mcp",
    inputs: [],
    cost: "Free with a Supabase account",
    page: "https://supabase.com/docs/guides/getting-started/mcp",
    signIn: true,
  },
  {
    id: "sentry",
    name: "Sentry",
    category: "Coding",
    description: "Look up errors, issues and traces in Sentry.",
    transport: "http",
    url: "https://mcp.sentry.dev/mcp",
    inputs: [],
    cost: "With a Sentry account",
    page: "https://mcp.sentry.dev/",
    signIn: true,
  },
  {
    id: "github",
    name: "GitHub",
    category: "Coding",
    description: "Your repositories, issues, pull requests, code search and Actions.",
    transport: "http",
    url: "https://api.githubcopilot.com/mcp/",
    inputs: [
      {
        key: "token",
        label: "GitHub personal access token",
        kind: "header",
        name: "Authorization",
        format: "Bearer {value}",
        secret: true,
        help: "A fine-grained token with only the repositories and access you want it to have.",
        link: "https://github.com/settings/personal-access-tokens",
      },
      { key: "readonly", label: "Read only", kind: "header", name: "X-MCP-Readonly", optional: true, placeholder: "true", help: "Type true to stop it changing anything." },
    ],
    cost: "Free with a GitHub account",
    page: "https://github.com/github/github-mcp-server",
    starter: true,
  },
  {
    id: "git",
    name: "Git",
    category: "Coding",
    description: "Work with a local repository: status, diffs, history, branches and commits.",
    transport: "stdio",
    command: "uvx",
    args: ["mcp-server-git"],
    inputs: [{ key: "repo", label: "Repository folder", kind: "arg", args: ["--repository", "{value}"], placeholder: windows ? "C:\\dev\\my-project" : "/home/me/my-project" }],
    cost: "Free",
    page: "https://github.com/modelcontextprotocol/servers/tree/main/src/git",
  },
  {
    id: "context7",
    name: "Context7",
    category: "Coding",
    description: "Up-to-date documentation and examples for programming libraries, for the version you use.",
    transport: "http",
    url: "https://mcp.context7.com/mcp",
    inputs: [{ key: "key", label: "Context7 API key", kind: "header", name: "Authorization", format: "Bearer {value}", secret: true, optional: true, help: "Works without one, with lower limits.", link: "https://context7.com/dashboard" }],
    cost: "Free",
    page: "https://context7.com",
    starter: true,
  },
  {
    id: "deepwiki",
    name: "DeepWiki",
    category: "Coding",
    description: "Ask questions about any public GitHub project and read its generated docs.",
    transport: "http",
    url: "https://mcp.deepwiki.com/mcp",
    inputs: [],
    cost: "Free, no key",
    page: "https://docs.devin.ai/work-with-devin/deepwiki-mcp",
  },
  {
    id: "filesystem",
    name: "Filesystem",
    category: "Coding",
    description: "Read, write and search files in one folder on this PC (e.g. a project or an Obsidian vault).",
    transport: "stdio",
    command: "npx",
    args: ["-y", "@modelcontextprotocol/server-filesystem"],
    inputs: [{ key: "dir", label: "Folder it may use", kind: "arg", args: ["{value}"], placeholder: windows ? "C:\\Users\\you\\Documents\\Notes" : "/home/me/notes" }],
    cost: "Free",
    page: "https://github.com/modelcontextprotocol/servers/tree/main/src/filesystem",
    note: "Changes it makes are saved straight away (unlike a chat's own folders, it doesn't ask first).",
  },
  {
    id: "wikipedia",
    name: "Wikipedia",
    category: "Knowledge",
    description: "Search Wikipedia and read articles, summaries and sections.",
    transport: "stdio",
    command: "uvx",
    args: ["wikipedia-mcp"],
    inputs: [{ key: "lang", label: "Language", kind: "arg", args: ["--language", "{value}"], optional: true, placeholder: "en" }],
    cost: "Free",
    page: "https://github.com/Rudra-ravi/wikipedia-mcp",
    starter: true,
  },
  {
    id: "arxiv",
    name: "arXiv",
    category: "Knowledge",
    description: "Search research papers on arXiv, and download and read them.",
    transport: "stdio",
    command: "uvx",
    args: ["arxiv-mcp-server"],
    inputs: [{ key: "dir", label: "Folder for downloaded papers", kind: "arg", args: ["--storage-path", "{value}"], optional: true }],
    cost: "Free",
    page: "https://github.com/blazickjp/arxiv-mcp-server",
  },
  {
    id: "weather",
    name: "Weather",
    category: "Knowledge",
    description: "Current weather, forecasts and air quality anywhere (Open-Meteo).",
    transport: "stdio",
    command: "uvx",
    // Its code needs the MCP library before 2.0 (which renamed McpError): pin it.
    args: ["--with", "mcp<2", "mcp_weather_server"],
    inputs: [],
    cost: "Free, no key",
    page: "https://github.com/isdaniel/mcp_weather_server",
  },
  {
    id: "memory",
    name: "Memory notebook",
    category: "Knowledge",
    description: "A notebook of people, things and how they relate, that the model can add to and look up across chats.",
    transport: "stdio",
    command: "npx",
    args: ["-y", "@modelcontextprotocol/server-memory"],
    inputs: [
      {
        key: "file",
        label: "Where to keep it",
        kind: "env",
        name: "MEMORY_FILE_PATH",
        placeholder: windows ? "C:\\Users\\you\\Documents\\outpost-memory.jsonl" : "/home/me/outpost-memory.jsonl",
        help: "A file path; it's created if it isn't there.",
      },
    ],
    cost: "Free",
    page: "https://github.com/modelcontextprotocol/servers/tree/main/src/memory",
    note: "Separate from a persona's own memories (those stay as they are).",
  },
  {
    id: "home-assistant",
    name: "Home Assistant",
    category: "Home & tasks",
    description: "Ask about and control the lights, switches and sensors you expose to Assist.",
    transport: "http",
    url: "{url}",
    inputs: [
      {
        key: "url",
        label: "Its MCP address",
        kind: "url",
        placeholder: "http://homeassistant.local:8123/api/mcp",
        help: "In Home Assistant: Settings › System › AI, turn on the Model Context Protocol card and copy its address.",
      },
      {
        key: "token",
        label: "Long-lived access token",
        kind: "header",
        name: "Authorization",
        format: "Bearer {value}",
        secret: true,
        help: "In Home Assistant: your profile › Security › Long-lived access tokens (as an admin).",
      },
    ],
    cost: "Free (your own Home Assistant)",
    page: "https://www.home-assistant.io/integrations/mcp_server/",
    note: "First add the Model Context Protocol Server integration (Settings › Devices & services), then copy the MCP address from Settings › System › AI. Use an admin's token.",
  },
  {
    id: "todoist-signin",
    name: "Todoist",
    category: "Home & tasks",
    description: "Read, add and finish your Todoist tasks and projects. Sign in with Todoist.",
    transport: "http",
    url: "https://ai.todoist.net/mcp",
    inputs: [],
    cost: "Free with a Todoist account",
    page: "https://github.com/Doist/todoist-ai",
    signIn: true,
  },
  {
    id: "todoist",
    name: "Todoist (with an API token)",
    category: "Home & tasks",
    description: "Read, add and finish your Todoist tasks and projects.",
    transport: "stdio",
    command: "npx",
    args: ["-y", "@doist/todoist-mcp"],
    inputs: [{ key: "key", label: "Todoist API token", kind: "env", name: "TODOIST_API_KEY", secret: true, help: "Todoist › Settings › Integrations › Developer.", link: "https://app.todoist.com/app/settings/integrations/developer" }],
    cost: "Free with a Todoist account",
    page: "https://github.com/Doist/todoist-mcp",
  },
  {
    id: "notion",
    name: "Notion (with an integration token)",
    category: "Home & tasks",
    description: "Search, read, create and update the Notion pages you share with it.",
    transport: "stdio",
    command: "npx",
    args: ["-y", "@notionhq/notion-mcp-server"],
    inputs: [
      {
        key: "token",
        label: "Notion integration token",
        kind: "env",
        name: "NOTION_TOKEN",
        secret: true,
        placeholder: "ntn_…",
        help: "Make an internal integration, copy its token, then share pages with it (the page's ••• › Connections).",
        link: "https://www.notion.so/profile/integrations",
      },
    ],
    cost: "Free with a Notion account",
    page: "https://github.com/makenotion/notion-mcp-server",
  },
  {
    id: "time",
    name: "Time zones",
    category: "Helpers",
    description: "The time anywhere, and converting between time zones.",
    transport: "stdio",
    command: "uvx",
    args: ["mcp-server-time"],
    inputs: [{ key: "tz", label: "Your time zone", kind: "arg", args: ["--local-timezone={value}"], optional: true, placeholder: "Australia/Sydney" }],
    cost: "Free",
    page: "https://github.com/modelcontextprotocol/servers/tree/main/src/time",
  },
  {
    id: "sequential-thinking",
    name: "Step-by-step thinking",
    category: "Helpers",
    description: "Lets a model without its own reasoning work through a hard problem in steps.",
    transport: "stdio",
    command: "npx",
    args: ["-y", "@modelcontextprotocol/server-sequential-thinking"],
    inputs: [],
    cost: "Free",
    page: "https://github.com/modelcontextprotocol/servers/tree/main/src/sequentialthinking",
  },
];

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
