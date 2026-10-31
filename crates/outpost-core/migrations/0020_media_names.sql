-- A name for a gallery item (a 3D model, say): shown in the gallery and used for the file
-- when it's saved. None until given.
ALTER TABLE media ADD COLUMN name TEXT;
