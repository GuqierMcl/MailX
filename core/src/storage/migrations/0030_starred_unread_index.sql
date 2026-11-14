-- The Now page counts starred unread mail on every sync event; this keeps the
-- count to the (usually few) starred unread rows instead of every message.
CREATE INDEX messages_starred_unread ON messages (account_id)
    WHERE is_flagged = 1 AND is_read = 0;
