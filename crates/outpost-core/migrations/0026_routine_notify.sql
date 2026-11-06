-- When a routine tells you about a run: always | worth (only when there's something new) |
-- failures | never. Each run records whether it should be told.
ALTER TABLE routines ADD COLUMN notify TEXT NOT NULL DEFAULT 'always';
ALTER TABLE routine_runs ADD COLUMN tell INTEGER NOT NULL DEFAULT 0;
