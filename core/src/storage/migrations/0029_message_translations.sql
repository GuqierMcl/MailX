-- Cached translations of message bodies. A row is only used while it matches
-- the message's raw hash, sanitizer version and segmenter version, so a changed
-- body or new segmentation rules make it stale without any cleanup pass.
CREATE TABLE message_translations (
    message_id TEXT NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    target_language TEXT NOT NULL,
    include_quotes INTEGER NOT NULL CHECK (include_quotes IN (0, 1)),
    raw_sha256 TEXT NOT NULL,
    sanitizer_version INTEGER NOT NULL,
    segmenter_version INTEGER NOT NULL,
    provider TEXT NOT NULL,
    source_language TEXT,
    subject TEXT,
    -- JSON array of string|null, one per extracted segment.
    segments TEXT NOT NULL,
    partial INTEGER NOT NULL CHECK (partial IN (0, 1)),
    created_at TEXT NOT NULL,
    PRIMARY KEY (message_id, target_language, include_quotes)
) STRICT;
