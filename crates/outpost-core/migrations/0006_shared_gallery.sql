-- The shared gallery: anyone can share one of their pictures or videos with every
-- account on this Outpost. NULL means private (the default).
ALTER TABLE media ADD COLUMN shared_at INTEGER;
CREATE INDEX media_shared ON media(shared_at) WHERE shared_at IS NOT NULL;
