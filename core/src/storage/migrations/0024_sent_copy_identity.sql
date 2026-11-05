-- Provider-derived identity supplements the server's Message-ID; it never
-- replaces it (reply threading and archives still use the original column).
ALTER TABLE messages ADD COLUMN original_message_id TEXT;
-- Empty means the cached header predates identity extraction. A policy change
-- also makes the row eligible for a bounded header refresh, including old UIDs.
ALTER TABLE messages ADD COLUMN sent_identity_policy TEXT NOT NULL DEFAULT '';
CREATE INDEX messages_by_message_id ON messages(account_id, message_id_header);
CREATE INDEX messages_by_original_message_id ON messages(account_id, original_message_id)
    WHERE original_message_id IS NOT NULL;
