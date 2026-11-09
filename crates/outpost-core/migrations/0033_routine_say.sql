-- Where a routine says its result out loud: "phone" and/or Home Assistant speakers
-- ("media_player.kitchen"), as a JSON list.
ALTER TABLE routines ADD COLUMN say_on TEXT NOT NULL DEFAULT '[]';
