-- A Home Assistant camera whose picture a routine looks at each run ("camera.front_door").
ALTER TABLE routines ADD COLUMN camera TEXT;
