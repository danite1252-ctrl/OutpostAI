-- Phase 5b: each persona can have its own speaking voice (a Kokoro voice name, e.g.
-- af_heart) and read its replies aloud.
ALTER TABLE personas ADD COLUMN voice TEXT;
ALTER TABLE personas ADD COLUMN speak_replies INTEGER NOT NULL DEFAULT 0;
