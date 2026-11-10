-- In quiet hours, skip telling (news that's stale by morning, like the doorbell) rather than
-- telling when they end.
ALTER TABLE routines ADD COLUMN quiet_skip INTEGER NOT NULL DEFAULT 0;
