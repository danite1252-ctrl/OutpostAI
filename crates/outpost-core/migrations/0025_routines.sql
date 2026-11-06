-- Routines: a persona doing something on its own, on a schedule (each run is a reply in the
-- routine's own chat).
CREATE TABLE routines (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    persona_id TEXT,
    instructions TEXT NOT NULL,
    -- JSON: {"kind":"once","at":...} | {"kind":"days","time":"07:30","days":[1,2,3,4,5]} | {"kind":"every","hours":4}
    schedule TEXT NOT NULL,
    time_zone TEXT,
    utc_offset_minutes INTEGER,
    enabled INTEGER NOT NULL DEFAULT 1,
    chat_id TEXT,
    next_at INTEGER,
    last_at INTEGER,
    last_status TEXT,
    last_note TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE INDEX routines_due ON routines(enabled, next_at);

CREATE TABLE routine_runs (
    id TEXT PRIMARY KEY,
    routine_id TEXT NOT NULL REFERENCES routines(id) ON DELETE CASCADE,
    started_at INTEGER NOT NULL,
    finished_at INTEGER,
    -- running | ok | failed
    status TEXT NOT NULL,
    -- The reply it made (in the routine's chat).
    message_id TEXT,
    note TEXT,
    -- What started it: schedule | now
    reason TEXT NOT NULL DEFAULT 'schedule'
);
CREATE INDEX routine_runs_by_routine ON routine_runs(routine_id, started_at);
