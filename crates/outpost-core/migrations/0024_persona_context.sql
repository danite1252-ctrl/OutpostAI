-- How much of a chat a persona's model keeps in mind (tokens). NULL: as much as the card
-- allows (Outpost's full window); smaller keeps a big model on the graphics card, faster.
ALTER TABLE personas ADD COLUMN context_length INTEGER;
