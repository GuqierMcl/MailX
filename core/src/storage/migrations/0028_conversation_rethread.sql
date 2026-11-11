-- conversation_id is derived data, rebuilt with stricter rules (sibling
-- linking, two-way subject fallback). Existing values are dropped and the
-- messages are re-threaded in the background; threading_version records which
-- rules a row was last threaded with.
ALTER TABLE messages ADD COLUMN threading_version INTEGER NOT NULL DEFAULT 0;
UPDATE messages SET conversation_id = NULL;
CREATE INDEX messages_pending_rethread ON messages (received_at, id)
    WHERE threading_version < 1;
