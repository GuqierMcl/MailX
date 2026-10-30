-- List fields of a local Sent copy (sending.md section 9): the plain-text
-- excerpt and the count of regular attachments, written when the item is
-- queued. Rows queued earlier are filled at startup from their snapshot.
ALTER TABLE outbox ADD COLUMN preview TEXT;
ALTER TABLE outbox ADD COLUMN attachment_count INTEGER;
