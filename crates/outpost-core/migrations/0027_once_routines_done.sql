-- One-off routines that had run were switched off (shown as Paused): they stay on, with
-- nothing next (shown as done).
UPDATE routines SET enabled = 1
WHERE enabled = 0 AND next_at IS NULL AND last_at IS NOT NULL AND schedule LIKE '%"kind":"once"%';
