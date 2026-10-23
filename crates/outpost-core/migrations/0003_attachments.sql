-- Phase 1c: images attached to chat messages. The files live under
-- <data>/files/<account_id>/images/; `path` is relative to <data>/files.

CREATE TABLE attachments (
    id          TEXT PRIMARY KEY,
    account_id  TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    message_id  TEXT NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    kind        TEXT NOT NULL CHECK (kind IN ('image')),
    mime        TEXT NOT NULL,
    bytes       INTEGER NOT NULL,
    path        TEXT NOT NULL,
    created_at  INTEGER NOT NULL
);
CREATE INDEX attachments_message ON attachments(message_id);
CREATE INDEX attachments_account ON attachments(account_id);
