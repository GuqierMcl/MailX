-- The Now page groups unread inbox mail by sender on every sync event; this
-- keeps that to the unread rows instead of every inbox message.
CREATE INDEX messages_unread_by_sender ON messages (folder_id, lower(from_address))
    WHERE is_read = 0;
