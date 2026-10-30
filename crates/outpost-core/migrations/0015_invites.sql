-- Invites: a pairing code tied to one person. "join" makes a new account (name and role
-- set by the owner) and "reset" sets a new PIN for an existing one; either way the person
-- picks their own PIN on their own device, which is then paired.
ALTER TABLE pairing_codes ADD COLUMN invite_kind TEXT;
ALTER TABLE pairing_codes ADD COLUMN invite_account TEXT;
ALTER TABLE pairing_codes ADD COLUMN invite_name TEXT;
ALTER TABLE pairing_codes ADD COLUMN invite_role TEXT;
