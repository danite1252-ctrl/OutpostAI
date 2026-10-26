-- What a character is ("woman", "dog") and how it's drawn ("realistic photography"),
-- from its captions, so "draw Kat at the beach" can say so: a trigger word alone
-- tells the model nothing ("zk4t" came out as a car).
ALTER TABLE characters ADD COLUMN subject TEXT;
ALTER TABLE characters ADD COLUMN style TEXT;
