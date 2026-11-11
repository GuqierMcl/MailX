ALTER TABLE messages ADD COLUMN in_reply_to TEXT;
ALTER TABLE messages ADD COLUMN references_header TEXT;
ALTER TABLE messages ADD COLUMN conversation_id TEXT;
CREATE INDEX messages_by_conversation ON messages (account_id, conversation_id)
    WHERE conversation_id IS NOT NULL;
