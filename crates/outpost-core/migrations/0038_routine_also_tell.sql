-- Other people a routine's news also goes to (account ids, JSON): the doorbell on a partner's
-- phone too.
ALTER TABLE routines ADD COLUMN also_tell TEXT NOT NULL DEFAULT '[]';
