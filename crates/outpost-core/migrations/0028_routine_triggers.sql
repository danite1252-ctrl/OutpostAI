-- Starting a routine from a secret link (n8n, Home Assistant...): the link's token, hashed.
ALTER TABLE routines ADD COLUMN hook_hash TEXT;
CREATE UNIQUE INDEX routines_hook ON routines(hook_hash) WHERE hook_hash IS NOT NULL;
