-- Rolling summary: when a chat outgrows the model's context window, its oldest messages
-- are summarised; `summary_upto` is the last message seq the summary covers.
ALTER TABLE chats ADD COLUMN summary TEXT;
ALTER TABLE chats ADD COLUMN summary_upto INTEGER NOT NULL DEFAULT 0;
