-- Compose attachments (docs/architecture/sending.md section 16.3).
--
-- One row per file attached to a draft. The bytes live in the content-addressed
-- blob store under `blob_hash`; identical files share one blob. Deleting the
-- draft deletes its rows, and the startup blob cleanup then frees blobs that
-- nothing else references. `inline` and `content_id` are reserved for inline
-- images and are always 0 / NULL for now. `origin` tells a file the user added
-- from one carried over by a forward.
CREATE TABLE draft_attachments (
    draft_id TEXT NOT NULL REFERENCES drafts(id) ON DELETE CASCADE,
    id TEXT NOT NULL CHECK (length(id) = 36),
    file_name TEXT NOT NULL CHECK (length(file_name) BETWEEN 1 AND 255),
    media_type TEXT NOT NULL CHECK (length(media_type) BETWEEN 3 AND 255),
    size INTEGER NOT NULL CHECK (size >= 0),
    blob_hash TEXT NOT NULL CHECK (length(blob_hash) = 64),
    inline INTEGER NOT NULL DEFAULT 0 CHECK (inline IN (0, 1)),
    content_id TEXT,
    origin TEXT NOT NULL CHECK (origin IN ('user', 'forwarded')),
    created_at TEXT NOT NULL,
    PRIMARY KEY (draft_id, id)
) STRICT;

CREATE INDEX draft_attachments_by_blob ON draft_attachments (blob_hash);
