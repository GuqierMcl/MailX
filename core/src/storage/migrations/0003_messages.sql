-- The composite parent key prevents crossing account or server identities.
CREATE UNIQUE INDEX folders_identity ON folders (id, account_id, mailbox_source_id);

CREATE TABLE folder_sync_cursors (
    folder_id TEXT PRIMARY KEY REFERENCES folders(id) ON DELETE CASCADE,
    uid_validity INTEGER NOT NULL,
    -- UTC date fixed when this generation first starts, not a rolling window.
    window_start TEXT NOT NULL,
    window_capped INTEGER NOT NULL CHECK (window_capped IN (0, 1)),
    backfill_done INTEGER NOT NULL CHECK (backfill_done IN (0, 1)),
    highest_uid INTEGER NOT NULL,
    updated_at TEXT NOT NULL
) STRICT;

CREATE TABLE messages (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL,
    mailbox_source_id TEXT NOT NULL,
    folder_id TEXT NOT NULL,
    uid_validity INTEGER NOT NULL,
    uid INTEGER NOT NULL,
    message_id_header TEXT,
    subject TEXT NOT NULL,
    from_name TEXT,
    from_address TEXT NOT NULL,
    -- JSON arrays of { name: string | null, address: string }.
    to_addresses TEXT NOT NULL,
    cc_addresses TEXT NOT NULL,
    reply_to_addresses TEXT NOT NULL,
    -- Fixed UTC millisecond timestamps sort chronologically as text.
    received_at TEXT NOT NULL,
    sent_at TEXT,
    size INTEGER,
    is_read INTEGER NOT NULL CHECK (is_read IN (0, 1)),
    is_flagged INTEGER NOT NULL CHECK (is_flagged IN (0, 1)),
    -- JSON array preserving every server flag, including custom keywords.
    flags TEXT NOT NULL,
    has_attachment INTEGER NOT NULL CHECK (has_attachment IN (0, 1)),
    remote_state TEXT NOT NULL CHECK (remote_state IN ('present', 'deleted', 'stale')),
    updated_at TEXT NOT NULL,
    UNIQUE (folder_id, uid_validity, uid),
    FOREIGN KEY (folder_id, account_id, mailbox_source_id)
        REFERENCES folders (id, account_id, mailbox_source_id) ON DELETE CASCADE
) STRICT;

CREATE INDEX messages_by_account ON messages (account_id, remote_state, received_at DESC, id DESC);
CREATE INDEX messages_by_folder ON messages (folder_id, remote_state, received_at DESC, id DESC);
