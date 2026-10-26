-- Phase 4: Training Studio. A dataset is a folder of pictures of one character with
-- a caption each; a training turns it into a LoRA; a character is the LoRA someone
-- picked, by name, for Create and chat.
-- Files: <data>/training/datasets/<dataset_id>/<image_id>.<ext> (+ .txt at training
-- time), runs in <data>/training/runs/<training_id>/.

CREATE TABLE datasets (
    id           TEXT PRIMARY KEY,
    account_id   TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    name         TEXT NOT NULL,
    -- The made-up word that calls the character up in a prompt, e.g. zk4t.
    trigger_word TEXT NOT NULL,
    created_at   INTEGER NOT NULL,
    updated_at   INTEGER NOT NULL
);
CREATE INDEX datasets_account ON datasets(account_id, updated_at);

CREATE TABLE dataset_images (
    id          TEXT PRIMARY KEY,
    dataset_id  TEXT NOT NULL REFERENCES datasets(id) ON DELETE CASCADE,
    -- File name inside the dataset folder.
    file        TEXT NOT NULL,
    mime        TEXT NOT NULL,
    bytes       INTEGER NOT NULL,
    -- What's in the picture, without the trigger word (it is added when training).
    caption     TEXT NOT NULL DEFAULT '',
    created_at  INTEGER NOT NULL
);
CREATE INDEX dataset_images_dataset ON dataset_images(dataset_id, created_at);

CREATE TABLE trainings (
    id           TEXT PRIMARY KEY,
    account_id   TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    dataset_id   TEXT NOT NULL REFERENCES datasets(id) ON DELETE CASCADE,
    -- Its place in the GPU queue; status, progress and errors live on the job.
    job_id       TEXT NOT NULL,
    trigger_word TEXT NOT NULL,
    steps        INTEGER NOT NULL,
    image_count  INTEGER NOT NULL,
    created_at   INTEGER NOT NULL
);
CREATE INDEX trainings_dataset ON trainings(dataset_id, created_at);

CREATE TABLE characters (
    id           TEXT PRIMARY KEY,
    account_id   TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    name         TEXT NOT NULL,
    trigger_word TEXT NOT NULL,
    -- File name in the LoRA folder.
    lora_file    TEXT NOT NULL UNIQUE,
    training_id  TEXT,
    -- The checkpoint that was picked.
    step         INTEGER,
    created_at   INTEGER NOT NULL
);
CREATE INDEX characters_account ON characters(account_id, name);
