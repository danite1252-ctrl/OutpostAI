-- A tool's actions that ask the person first, in every chat (JSON list of names; its owner chooses).
ALTER TABLE tools ADD COLUMN ask_tools TEXT NOT NULL DEFAULT '[]';
