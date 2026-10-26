-- Which message taught an automatic memory, so deleting that message forgets it too.
-- (Memories learned before this have only their chat.)
ALTER TABLE memories ADD COLUMN message_id TEXT;
CREATE INDEX memories_message ON memories(message_id);
