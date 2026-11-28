-- Some servers report UIDVALIDITY 0 for a folder. MailX treats it as an
-- ordinary generation, so pending deletions in such folders must be storable.
-- Rebuild message_deletions with the generation check relaxed to >= 0.
CREATE TABLE message_deletions_next (
    message_id TEXT PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,
    batch_id TEXT NOT NULL,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    mailbox_source_id TEXT NOT NULL,
    folder_id TEXT NOT NULL REFERENCES folders(id) ON DELETE CASCADE,
    uid_validity INTEGER NOT NULL CHECK (uid_validity >= 0),
    uid INTEGER NOT NULL CHECK (uid > 0),
    -- NULL means expunge for good; otherwise the Trash folder to move into.
    target_folder_id TEXT REFERENCES folders(id) ON DELETE SET NULL,
    permanent INTEGER NOT NULL CHECK (permanent IN (0, 1)),
    due_at TEXT NOT NULL,
    started INTEGER NOT NULL DEFAULT 0 CHECK (started IN (0, 1)),
    attempts INTEGER NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    created_at TEXT NOT NULL
);

INSERT INTO message_deletions_next
    (message_id, batch_id, account_id, mailbox_source_id, folder_id, uid_validity, uid,
     target_folder_id, permanent, due_at, started, attempts, created_at)
SELECT message_id, batch_id, account_id, mailbox_source_id, folder_id, uid_validity, uid,
       target_folder_id, permanent, due_at, started, attempts, created_at
FROM message_deletions;

DROP TABLE message_deletions;
ALTER TABLE message_deletions_next RENAME TO message_deletions;

CREATE INDEX message_deletions_account_due
    ON message_deletions(account_id, due_at);
CREATE INDEX message_deletions_batch
    ON message_deletions(batch_id);
