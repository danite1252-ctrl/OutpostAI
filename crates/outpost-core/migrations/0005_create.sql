-- Phase 2: Create. Jobs wait in one queue for the GPU; what they make goes into
-- the account's gallery (files under <data>/files/<account_id>/gallery/).

CREATE TABLE jobs (
    id           TEXT PRIMARY KEY,
    account_id   TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    pack_id      TEXT NOT NULL,
    -- The values the user gave, as JSON.
    inputs       TEXT NOT NULL,
    status       TEXT NOT NULL CHECK (status IN ('queued', 'running', 'done', 'failed', 'cancelled')),
    error        TEXT,
    seed         INTEGER,
    created_at   INTEGER NOT NULL,
    started_at   INTEGER,
    finished_at  INTEGER
);
CREATE INDEX jobs_queue ON jobs(status, created_at);
CREATE INDEX jobs_account ON jobs(account_id, created_at);

CREATE TABLE media (
    id          TEXT PRIMARY KEY,
    account_id  TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    job_id      TEXT REFERENCES jobs(id) ON DELETE SET NULL,
    kind        TEXT NOT NULL CHECK (kind IN ('image', 'video')),
    mime        TEXT NOT NULL,
    bytes       INTEGER NOT NULL,
    -- Relative to <data>/files.
    path        TEXT NOT NULL,
    pack_id     TEXT,
    prompt      TEXT,
    seed        INTEGER,
    created_at  INTEGER NOT NULL
);
CREATE INDEX media_account ON media(account_id, created_at);
