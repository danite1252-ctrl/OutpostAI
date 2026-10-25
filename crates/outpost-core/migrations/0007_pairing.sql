-- Pairing a phone: a one-time 6-digit code shown on the Outpost PC. Only its hash
-- is kept. A code dies when used, when it expires, or after too many wrong guesses.
CREATE TABLE pairing_codes (
    id          TEXT PRIMARY KEY,
    code_hash   TEXT NOT NULL,
    created_by  TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    created_at  INTEGER NOT NULL,
    expires_at  INTEGER NOT NULL,
    used_at     INTEGER,
    -- The device session it made.
    session_id  TEXT,
    wrong_guesses INTEGER NOT NULL DEFAULT 0
);

-- Who goes first on the GPU: 0 someone at the desktop, 1 a phone, 2 an app key.
ALTER TABLE jobs ADD COLUMN priority INTEGER NOT NULL DEFAULT 1;
