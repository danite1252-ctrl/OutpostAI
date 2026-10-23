-- Phase 1d: automatic memory. Memories learned from chats are marked 'auto' and
-- recalled by relevance; ones you add yourself are 'manual' and always included.
-- `embedding` is a unit-length f32 vector (little-endian) from `embedding_model`.

ALTER TABLE memories ADD COLUMN source TEXT NOT NULL DEFAULT 'manual' CHECK (source IN ('manual', 'auto'));
ALTER TABLE memories ADD COLUMN chat_id TEXT REFERENCES chats(id) ON DELETE SET NULL;
ALTER TABLE memories ADD COLUMN embedding BLOB;
ALTER TABLE memories ADD COLUMN embedding_model TEXT;
CREATE INDEX memories_account_source ON memories(account_id, source);
