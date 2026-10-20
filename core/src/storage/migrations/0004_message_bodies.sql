CREATE TABLE message_bodies (
    message_id TEXT PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,
    raw_sha256 TEXT NOT NULL,
    raw_size INTEGER NOT NULL,
    plain_text TEXT,
    -- Only the isolated body protocol may serve this field; never return it over IPC.
    sanitized_html TEXT,
    sanitizer_version INTEGER NOT NULL,
    links TEXT NOT NULL,
    remote_images_blocked INTEGER NOT NULL,
    truncated INTEGER NOT NULL CHECK (truncated IN (0, 1)),
    fetched_at TEXT NOT NULL
) STRICT;

CREATE TABLE message_parts (
    message_id TEXT NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    part_index INTEGER NOT NULL,
    content_id TEXT,
    filename TEXT,
    mime_type TEXT NOT NULL,
    size INTEGER NOT NULL,
    is_inline INTEGER NOT NULL CHECK (is_inline IN (0, 1)),
    PRIMARY KEY (message_id, part_index)
) STRICT;

CREATE INDEX message_parts_by_cid ON message_parts (message_id, content_id);

ALTER TABLE messages ADD COLUMN body_state TEXT NOT NULL DEFAULT 'missing'
    CHECK (body_state IN ('missing', 'fetching', 'ready', 'failed'));
ALTER TABLE messages ADD COLUMN body_failure TEXT;
ALTER TABLE messages ADD COLUMN preview TEXT NOT NULL DEFAULT '';
