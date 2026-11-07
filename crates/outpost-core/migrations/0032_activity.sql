-- Every tool call a persona makes: who, from which chat or routine, the action as the model
-- sent it, and what came of it. Owners see everyone's; people see their own.
CREATE TABLE activity (
    id TEXT PRIMARY KEY,
    at INTEGER NOT NULL,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    chat_id TEXT,
    routine_id TEXT,
    -- chat | hands_free | routine
    source TEXT NOT NULL,
    server TEXT NOT NULL,
    tool TEXT NOT NULL,
    -- The arguments exactly as the model sent them (JSON).
    arguments TEXT NOT NULL,
    -- done | failed | not_allowed | skipped | needs_ok
    outcome TEXT NOT NULL,
    result TEXT NOT NULL
);
CREATE INDEX activity_at ON activity(at);
CREATE INDEX activity_person ON activity(account_id, at);
