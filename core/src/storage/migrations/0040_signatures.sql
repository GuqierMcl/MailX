CREATE TABLE signatures (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    body_html TEXT NOT NULL,
    body_text TEXT NOT NULL,
    sort_order INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE signature_images (
    signature_id TEXT NOT NULL REFERENCES signatures(id) ON DELETE CASCADE,
    id TEXT NOT NULL,
    file_name TEXT NOT NULL,
    media_type TEXT NOT NULL,
    size INTEGER NOT NULL CHECK (size >= 0),
    blob_hash TEXT NOT NULL,
    content_id TEXT NOT NULL,
    created_at TEXT NOT NULL,
    PRIMARY KEY (signature_id, id)
);

CREATE INDEX signature_images_by_blob ON signature_images (blob_hash);
CREATE UNIQUE INDEX signature_images_content_id
    ON signature_images (signature_id, content_id);

CREATE TABLE account_signatures (
    account_id TEXT PRIMARY KEY REFERENCES accounts(id) ON DELETE CASCADE,
    signature_id TEXT NOT NULL REFERENCES signatures(id) ON DELETE CASCADE
);

CREATE INDEX account_signatures_by_signature ON account_signatures (signature_id);
