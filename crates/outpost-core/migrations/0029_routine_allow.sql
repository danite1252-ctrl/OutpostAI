-- What a routine may do while nobody's watching, and quiet hours (a run's news waits until
-- they're over).
ALTER TABLE routines ADD COLUMN allow_tools INTEGER NOT NULL DEFAULT 1;
-- wait (changes wait for the person to apply them) | apply
ALTER TABLE routines ADD COLUMN allow_changes TEXT NOT NULL DEFAULT 'wait';
-- Commands already allowed "always" in the folder (others never run unattended).
ALTER TABLE routines ADD COLUMN allow_commands INTEGER NOT NULL DEFAULT 1;
-- When a run is told (the end of quiet hours, if it finished during them).
ALTER TABLE routine_runs ADD COLUMN tell_at INTEGER;
UPDATE routine_runs SET tell_at = finished_at WHERE tell = 1;
