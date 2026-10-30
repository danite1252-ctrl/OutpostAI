-- Gallery search: what a vision model sees in each picture, and an embedding of that (with
-- the prompt) for finding pictures by meaning. One row per described (or tried) item.
CREATE TABLE media_index (
    media_id        TEXT PRIMARY KEY REFERENCES media(id) ON DELETE CASCADE,
    description     TEXT,
    -- The vision model that wrote it.
    model           TEXT,
    described_at    INTEGER,
    -- Failed tries (it stops trying after a few), and why the last one failed.
    tries           INTEGER NOT NULL DEFAULT 0,
    error           TEXT,
    embedding       BLOB,
    embedding_model TEXT
);
