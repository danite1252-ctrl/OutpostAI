-- Who may use a shared tool, and which of its actions: per person, set by its owner. No row:
-- everyone may use all of it (restricted accounts only when the tool allows them).
CREATE TABLE tool_people (
    tool_id TEXT NOT NULL REFERENCES tools(id) ON DELETE CASCADE,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    -- all | some | none
    access TEXT NOT NULL,
    -- For some: the actions allowed (JSON list of tool names).
    actions TEXT NOT NULL DEFAULT '[]',
    PRIMARY KEY (tool_id, account_id)
);
