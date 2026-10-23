-- Outpost schema v1: accounts, sign-in sessions, audit log, settings.
-- Times are Unix seconds (UTC).

CREATE TABLE accounts (
    id               TEXT PRIMARY KEY,
    name             TEXT NOT NULL UNIQUE COLLATE NOCASE,
    role             TEXT NOT NULL CHECK (role IN ('owner', 'member', 'restricted', 'service')),
    avatar_color     TEXT NOT NULL,
    -- Argon2id hash of the account PIN or password. NULL only for service accounts.
    secret_hash      TEXT,
    failed_attempts  INTEGER NOT NULL DEFAULT 0,
    locked_until     INTEGER,
    created_at       INTEGER NOT NULL
);

CREATE TABLE sessions (
    id            TEXT PRIMARY KEY,
    account_id    TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    -- SHA-256 of the bearer token; the token itself is never stored.
    token_hash    TEXT NOT NULL UNIQUE,
    kind          TEXT NOT NULL CHECK (kind IN ('desktop', 'device', 'app')),
    label         TEXT NOT NULL,
    created_at    INTEGER NOT NULL,
    last_used_at  INTEGER NOT NULL,
    revoked_at    INTEGER
);
CREATE INDEX sessions_account ON sessions(account_id);

-- No foreign keys: history survives when an account is deleted.
CREATE TABLE audit_log (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    at          INTEGER NOT NULL,
    account_id  TEXT,
    session_id  TEXT,
    action      TEXT NOT NULL,
    detail      TEXT
);
CREATE INDEX audit_log_at ON audit_log(at);

CREATE TABLE settings (
    key    TEXT PRIMARY KEY,
    value  TEXT NOT NULL
);
