-- Preserve attachment order and references while admitting tool provenance.
CREATE TABLE draft_attachments_next (
    draft_id TEXT NOT NULL REFERENCES drafts(id) ON DELETE CASCADE,
    id TEXT NOT NULL CHECK (length(id) = 36),
    file_name TEXT NOT NULL CHECK (length(file_name) BETWEEN 1 AND 255),
    media_type TEXT NOT NULL CHECK (length(media_type) BETWEEN 3 AND 255),
    size INTEGER NOT NULL CHECK (size >= 0),
    blob_hash TEXT NOT NULL CHECK (length(blob_hash) = 64),
    inline INTEGER NOT NULL DEFAULT 0 CHECK (inline IN (0, 1)),
    content_id TEXT,
    origin TEXT NOT NULL CHECK (origin IN ('user', 'forwarded', 'mcp', 'ai')),
    created_at TEXT NOT NULL,
    PRIMARY KEY (draft_id, id)
) STRICT;

INSERT INTO draft_attachments_next
    (rowid, draft_id, id, file_name, media_type, size, blob_hash, inline, content_id, origin, created_at)
SELECT rowid, draft_id, id, file_name, media_type, size, blob_hash, inline, content_id, origin, created_at
FROM draft_attachments;

DROP TABLE draft_attachments;
ALTER TABLE draft_attachments_next RENAME TO draft_attachments;

CREATE INDEX draft_attachments_by_blob ON draft_attachments (blob_hash);
CREATE UNIQUE INDEX draft_attachments_inline_content_id
    ON draft_attachments(draft_id, content_id) WHERE inline = 1;
