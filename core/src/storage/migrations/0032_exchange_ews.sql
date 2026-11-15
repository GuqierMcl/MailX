-- On-premises Exchange accounts reached over EWS (Exchange + HTTP + NTLM).
-- One HTTPS endpoint carries both reading and sending; the account keeps its
-- IMAP/SMTP rows as compatibility placeholders for storage and outbox code.
CREATE TABLE exchange_endpoints (
    account_id TEXT NOT NULL PRIMARY KEY REFERENCES accounts(id),
    url TEXT NOT NULL,
    auth TEXT NOT NULL CHECK (auth IN ('ntlm', 'basic')),
    username TEXT NOT NULL,
    cert_principal_name TEXT
) STRICT;

-- EWS ItemIds are opaque and change on moves. Map each one to a stable local
-- uid inside its folder so the shared message tables stay protocol-neutral.
CREATE TABLE ews_items (
    account_id TEXT NOT NULL REFERENCES accounts(id),
    folder_id TEXT NOT NULL,
    item_id TEXT NOT NULL,
    change_key TEXT,
    uid INTEGER NOT NULL,
    PRIMARY KEY (account_id, folder_id, item_id),
    UNIQUE (account_id, folder_id, uid)
) STRICT;

-- Per-folder SyncFolderItems watermark and the next uid to hand out.
CREATE TABLE ews_folder_state (
    account_id TEXT NOT NULL REFERENCES accounts(id),
    folder_id TEXT NOT NULL,
    sync_state TEXT,
    highest_uid INTEGER NOT NULL DEFAULT 0,
    last_synced_at TEXT,
    PRIMARY KEY (account_id, folder_id)
) STRICT;
