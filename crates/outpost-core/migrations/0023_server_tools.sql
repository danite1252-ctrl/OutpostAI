-- Tools on the Outpost server, per account: MCP servers (an address, or a program the server
-- runs) and Outpost's own (Google). Personal by default; shared ones (Home Assistant) work in
-- everyone's chats. Keys and sign-ins are sealed (encrypted with the server's own key, kept
-- outside the database); `secret_names` only names them, for the apps.
CREATE TABLE tools (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    catalog_id TEXT,
    transport TEXT NOT NULL CHECK (transport IN ('stdio', 'http', 'builtin')),
    command TEXT NOT NULL DEFAULT '',
    args TEXT NOT NULL DEFAULT '[]',
    url TEXT NOT NULL DEFAULT '',
    -- sealed JSON {"env":{...},"headers":{...}}
    secrets TEXT,
    -- JSON list of the setting names kept in `secrets`
    secret_names TEXT NOT NULL DEFAULT '[]',
    -- sealed JSON sign-in (OAuth), and who it's signed in as ("dan@gmail.com")
    oauth TEXT,
    signed_in_as TEXT,
    enabled INTEGER NOT NULL DEFAULT 1,
    shared INTEGER NOT NULL DEFAULT 0,
    -- restricted accounts (children) may use it, when shared
    restricted_ok INTEGER NOT NULL DEFAULT 0,
    disabled_tools TEXT NOT NULL DEFAULT '[]',
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE INDEX tools_account ON tools(account_id);

-- A persona's tools: chats with it start with these.
CREATE TABLE persona_tools (
    persona_id TEXT NOT NULL REFERENCES personas(id) ON DELETE CASCADE,
    tool_id TEXT NOT NULL REFERENCES tools(id) ON DELETE CASCADE,
    PRIMARY KEY (persona_id, tool_id)
);

-- A chat's own choice over its persona's: on (1) adds a tool, off (0) takes one away.
CREATE TABLE chat_tools (
    chat_id TEXT NOT NULL REFERENCES chats(id) ON DELETE CASCADE,
    tool_id TEXT NOT NULL REFERENCES tools(id) ON DELETE CASCADE,
    is_on INTEGER NOT NULL,
    PRIMARY KEY (chat_id, tool_id)
);
