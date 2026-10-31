-- 3D models (.glb) in the gallery: the gallery's kind check gains 'model'. SQLite can't
-- change a check, so the table is rebuilt, keeping every row (and the search index, which
-- points at it, is set aside and put back so nothing cascades away).
CREATE TABLE media_index_keep AS SELECT * FROM media_index;
DROP TABLE media_index;

CREATE TABLE media_new (
    id          TEXT PRIMARY KEY,
    account_id  TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    job_id      TEXT REFERENCES jobs(id) ON DELETE SET NULL,
    kind        TEXT NOT NULL CHECK (kind IN ('image', 'video', 'model')),
    mime        TEXT NOT NULL,
    bytes       INTEGER NOT NULL,
    -- Relative to <data>/files.
    path        TEXT NOT NULL,
    pack_id     TEXT,
    prompt      TEXT,
    seed        INTEGER,
    created_at  INTEGER NOT NULL,
    shared_at   INTEGER
);
INSERT INTO media_new (id, account_id, job_id, kind, mime, bytes, path, pack_id, prompt, seed, created_at, shared_at)
    SELECT id, account_id, job_id, kind, mime, bytes, path, pack_id, prompt, seed, created_at, shared_at FROM media;
DROP TABLE media;
ALTER TABLE media_new RENAME TO media;
CREATE INDEX media_account ON media(account_id, created_at);
CREATE INDEX media_shared ON media(shared_at) WHERE shared_at IS NOT NULL;

CREATE TABLE media_index (
    media_id        TEXT PRIMARY KEY REFERENCES media(id) ON DELETE CASCADE,
    description     TEXT,
    model           TEXT,
    described_at    INTEGER,
    tries           INTEGER NOT NULL DEFAULT 0,
    error           TEXT,
    embedding       BLOB,
    embedding_model TEXT
);
INSERT INTO media_index SELECT * FROM media_index_keep;
DROP TABLE media_index_keep;
