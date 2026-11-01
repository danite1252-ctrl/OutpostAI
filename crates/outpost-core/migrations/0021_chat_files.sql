-- Folders a chat can read and change. Server folders are kept here (their files under
-- files/<account>/folders/<id>); PC folders stay on the PC, which reads and writes them for
-- the chat while its desktop app is open.
CREATE TABLE folders (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE INDEX folders_account ON folders(account_id);

CREATE TABLE chat_folders (
    chat_id TEXT NOT NULL REFERENCES chats(id) ON DELETE CASCADE,
    -- 'server' (a folder above) or 'pc' (a folder on one PC)
    kind TEXT NOT NULL CHECK (kind IN ('server', 'pc')),
    -- the server folder's id, or the PC folder's id on that PC
    ref TEXT NOT NULL,
    -- the name the model and the apps see (the first part of its paths)
    name TEXT NOT NULL,
    -- for a PC folder: which PC ("Dan's PC")
    device TEXT,
    PRIMARY KEY (chat_id, kind, ref)
);

-- How a chat changes files: 'ask' (each change waits for Apply) or 'auto' (straight away,
-- with Undo).
ALTER TABLE chats ADD COLUMN file_mode TEXT NOT NULL DEFAULT 'ask';

-- What a reply did with files (JSON list), shown under it.
ALTER TABLE messages ADD COLUMN files TEXT;

-- Each change a reply made or proposed: what the file was and becomes.
CREATE TABLE file_changes (
    id TEXT PRIMARY KEY,
    chat_id TEXT NOT NULL REFERENCES chats(id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    ref TEXT NOT NULL,
    folder TEXT NOT NULL,
    path TEXT NOT NULL,
    -- NULL: the file didn't exist
    before TEXT,
    after TEXT NOT NULL,
    -- 'proposed', 'applied', 'skipped', 'undone'
    status TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE INDEX file_changes_chat ON file_changes(chat_id, created_at);
