-- Phase 5a: webhooks. Outpost POSTs a signed JSON message to the account's own URL
-- (n8n, Home Assistant, a script) when something finishes.
CREATE TABLE webhooks (
    id           TEXT PRIMARY KEY,
    account_id   TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    url          TEXT NOT NULL,
    -- Shared secret for the X-Outpost-Signature HMAC; shown once when made.
    secret       TEXT NOT NULL,
    -- Comma-separated event names, e.g. "job.finished,training.finished".
    events       TEXT NOT NULL,
    created_at   INTEGER NOT NULL,
    -- How the last delivery went.
    last_at      INTEGER,
    last_status  INTEGER,
    last_error   TEXT
);
CREATE INDEX webhooks_account ON webhooks(account_id);
