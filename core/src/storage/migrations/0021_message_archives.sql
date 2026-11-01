-- Local-only archive state of inbox messages. Archiving never touches the
-- server. A row is keyed by the message's Message-ID header ('m:' + header)
-- so it survives a UIDVALIDITY reset that rebuilds message rows with new
-- ids; mail without that header is keyed by its local id ('i:' + id) and can
-- return to the inbox after such a reset. Rows are scoped to the account's
-- mailbox_source_id, so a re-pointed account does not inherit them.
CREATE TABLE message_archives (
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    mailbox_source_id TEXT NOT NULL,
    archive_key TEXT NOT NULL,
    archived_at TEXT NOT NULL,
    PRIMARY KEY (account_id, mailbox_source_id, archive_key)
) WITHOUT ROWID;
