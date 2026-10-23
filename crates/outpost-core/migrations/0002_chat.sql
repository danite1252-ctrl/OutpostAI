-- Phase 1b: personas, chats, messages and memory. Everything belongs to an account
-- and disappears with it.

CREATE TABLE personas (
    id             TEXT PRIMARY KEY,
    account_id     TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    -- Used in the API as `persona:<slug>`.
    slug           TEXT NOT NULL,
    name           TEXT NOT NULL,
    avatar_color   TEXT NOT NULL,
    system_prompt  TEXT NOT NULL DEFAULT '',
    -- Ollama model name; NULL means "the default chat model".
    model          TEXT,
    temperature    REAL,
    think          INTEGER NOT NULL DEFAULT 0,
    use_memory     INTEGER NOT NULL DEFAULT 1,
    created_at     INTEGER NOT NULL,
    updated_at     INTEGER NOT NULL,
    UNIQUE (account_id, slug)
);

CREATE TABLE chats (
    id          TEXT PRIMARY KEY,
    account_id  TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    persona_id  TEXT REFERENCES personas(id) ON DELETE SET NULL,
    title       TEXT NOT NULL,
    -- Overrides the persona's model for this chat when set.
    model       TEXT,
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL
);
CREATE INDEX chats_account ON chats(account_id, updated_at);

CREATE TABLE messages (
    id          TEXT PRIMARY KEY,
    chat_id     TEXT NOT NULL REFERENCES chats(id) ON DELETE CASCADE,
    seq         INTEGER NOT NULL,
    role        TEXT NOT NULL CHECK (role IN ('user', 'assistant', 'system')),
    content     TEXT NOT NULL,
    -- The model's reasoning, for thinking models.
    thinking    TEXT,
    model       TEXT,
    created_at  INTEGER NOT NULL,
    UNIQUE (chat_id, seq)
);

CREATE TABLE memories (
    id          TEXT PRIMARY KEY,
    account_id  TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    -- NULL: shared by all of the account's personas.
    persona_id  TEXT REFERENCES personas(id) ON DELETE CASCADE,
    text        TEXT NOT NULL,
    created_at  INTEGER NOT NULL
);
CREATE INDEX memories_account ON memories(account_id);
