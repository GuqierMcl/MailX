-- Recipient suggestions read the distinct (sender address, display name)
-- pairs; this covering index lets them skip the message rows.
CREATE INDEX messages_by_sender_name ON messages (from_address, from_name);
