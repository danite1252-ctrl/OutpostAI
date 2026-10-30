-- Documents to chat with: each one's text in passages (with embeddings for finding the
-- relevant ones), and which chats and personas use which documents.
CREATE TABLE documents (
    id          TEXT PRIMARY KEY,
    account_id  TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    name        TEXT NOT NULL,
    mime        TEXT NOT NULL,
    bytes       INTEGER NOT NULL,
    -- Under the files folder: <account>/documents/<id>.<ext>
    path        TEXT NOT NULL,
    sha256      TEXT NOT NULL,
    status      TEXT NOT NULL DEFAULT 'reading' CHECK (status IN ('reading', 'ready', 'failed')),
    error       TEXT,
    pages       INTEGER,
    chars       INTEGER NOT NULL DEFAULT 0,
    created_at  INTEGER NOT NULL
);
CREATE INDEX documents_by_account ON documents (account_id, created_at);

CREATE TABLE document_chunks (
    document_id     TEXT NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
    idx             INTEGER NOT NULL,
    page            INTEGER,
    text            TEXT NOT NULL,
    embedding       BLOB,
    embedding_model TEXT,
    PRIMARY KEY (document_id, idx)
);

CREATE TABLE chat_documents (
    chat_id     TEXT NOT NULL REFERENCES chats(id) ON DELETE CASCADE,
    document_id TEXT NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
    PRIMARY KEY (chat_id, document_id)
);

CREATE TABLE persona_documents (
    persona_id  TEXT NOT NULL REFERENCES personas(id) ON DELETE CASCADE,
    document_id TEXT NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
    PRIMARY KEY (persona_id, document_id)
);

-- The passages a reply used, as JSON: [{"n":1,"document_id":..,"name":..,"page":3}, ...]
ALTER TABLE messages ADD COLUMN sources TEXT;
