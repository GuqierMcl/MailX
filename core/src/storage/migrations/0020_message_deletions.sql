-- Deletions the user requested but MailX has not yet carried out on the
-- server. A row hides its message from every list immediately; the account's
-- write-back worker executes it once due_at passes, and undo is only possible
-- while started = 0. Rows are removed on success (the message becomes a
-- `deleted` tombstone) and on final failure (the message shows again).
CREATE TABLE message_deletions (
    message_id TEXT PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,
    batch_id TEXT NOT NULL,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    mailbox_source_id TEXT NOT NULL,
    folder_id TEXT NOT NULL REFERENCES folders(id) ON DELETE CASCADE,
    uid_validity INTEGER NOT NULL CHECK (uid_validity > 0),
    uid INTEGER NOT NULL CHECK (uid > 0),
    -- NULL means expunge for good; otherwise the Trash folder to move into.
    target_folder_id TEXT REFERENCES folders(id) ON DELETE SET NULL,
    permanent INTEGER NOT NULL CHECK (permanent IN (0, 1)),
    due_at TEXT NOT NULL,
    started INTEGER NOT NULL DEFAULT 0 CHECK (started IN (0, 1)),
    attempts INTEGER NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    created_at TEXT NOT NULL
);

CREATE INDEX message_deletions_account_due
    ON message_deletions(account_id, due_at);
CREATE INDEX message_deletions_batch
    ON message_deletions(batch_id);
