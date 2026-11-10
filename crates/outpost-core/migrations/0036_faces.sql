-- Known faces: people Outpost can name at the door. Kept on this server only.
CREATE TABLE known_faces (
    id          TEXT PRIMARY KEY,
    name        TEXT NOT NULL,
    -- Someone who uses Outpost, when this is them.
    account_id  TEXT REFERENCES accounts(id) ON DELETE SET NULL,
    added_by    TEXT NOT NULL,
    created_at  INTEGER NOT NULL
);

-- One photo's face: its 128 numbers (little-endian f32) and the face itself (JPEG).
CREATE TABLE face_samples (
    id          TEXT PRIMARY KEY,
    face_id     TEXT NOT NULL REFERENCES known_faces(id) ON DELETE CASCADE,
    feature     BLOB NOT NULL,
    crop        BLOB NOT NULL,
    created_at  INTEGER NOT NULL
);
CREATE INDEX face_samples_face ON face_samples(face_id);

-- Routines that say who it is when they see a known face.
ALTER TABLE routines ADD COLUMN recognise INTEGER NOT NULL DEFAULT 0;
