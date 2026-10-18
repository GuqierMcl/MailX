CREATE TABLE folders (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    -- Folders belong to one server identity; rows from an older identity are never shown.
    mailbox_source_id TEXT NOT NULL,
    -- Name as sent by the server (modified UTF-7); the key used to address the folder.
    remote_name TEXT NOT NULL,
    -- Local id of the parent folder, derived from the server delimiter.
    parent_id TEXT,
    name TEXT NOT NULL,
    delimiter TEXT,
    role TEXT NOT NULL CHECK (role IN ('inbox', 'sent', 'drafts', 'archive', 'junk', 'trash', 'other')),
    selectable INTEGER NOT NULL CHECK (selectable IN (0, 1)),
    -- JSON array of LIST attributes as the server sent them.
    attributes TEXT NOT NULL,
    unread_count INTEGER NOT NULL CHECK (unread_count >= 0),
    total_count INTEGER NOT NULL CHECK (total_count >= 0),
    uid_next INTEGER,
    uid_validity INTEGER,
    synced_at TEXT NOT NULL,
    UNIQUE (account_id, mailbox_source_id, remote_name)
) STRICT;

CREATE INDEX folders_by_account ON folders (account_id, mailbox_source_id);
