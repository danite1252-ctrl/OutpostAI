-- A persona can be shared with everyone on this Outpost: they chat with it (their chats and
-- memories stay theirs) and can make their own copy; only its owner edits it.
ALTER TABLE personas ADD COLUMN shared INTEGER NOT NULL DEFAULT 0;
