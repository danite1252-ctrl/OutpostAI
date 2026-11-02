-- A reply's tool calls and their results (JSON list of Ollama messages), sent back with the
-- chat's history so the model sees what it actually did (and keeps calling tools for follow-ups).
ALTER TABLE messages ADD COLUMN tool_trace TEXT;
