-- Chats Outpost keeps for itself rather than ones you started: "routine" (a routine's own chat)
-- and "voice" (Phone voice, Home Assistant voice). Apps list them apart.
ALTER TABLE chats ADD COLUMN automatic TEXT;
UPDATE chats SET automatic = 'routine' WHERE id IN (SELECT chat_id FROM routines WHERE chat_id IS NOT NULL);
UPDATE chats SET automatic = 'voice' WHERE id IN (SELECT value FROM settings WHERE key LIKE 'voice.%.chat.%');
