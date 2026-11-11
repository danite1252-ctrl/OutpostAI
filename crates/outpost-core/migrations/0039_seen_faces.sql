-- Faces a routine saw (the doorbell), kept 30 days so an owner can say who they were.
CREATE TABLE seen_faces (
    id            TEXT PRIMARY KEY,
    routine_id    TEXT,
    feature       BLOB NOT NULL,
    crop          BLOB NOT NULL,
    -- Who it looked like when it was seen, if anyone.
    match_name    TEXT,
    match_score   REAL,
    created_at    INTEGER NOT NULL
);
CREATE INDEX seen_faces_at ON seen_faces(created_at);
