CREATE TABLE contact_aggregations (
    address TEXT PRIMARY KEY NOT NULL,
    created_at TEXT NOT NULL
) STRICT;
CREATE INDEX messages_by_sender ON messages (lower(from_address), received_at DESC, id DESC);
