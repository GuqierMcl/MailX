-- Existing accounts retain their connections, secrets and mailbox source.
ALTER TABLE accounts ADD COLUMN mail_backend TEXT NOT NULL DEFAULT 'imapSmtp'
    CHECK (mail_backend IN ('imapSmtp', 'ews'));
CREATE TABLE ews_connections (
    account_id TEXT PRIMARY KEY REFERENCES accounts(id) ON DELETE CASCADE,
    url TEXT NOT NULL,
    username TEXT NOT NULL,
    auth_profile_id TEXT NOT NULL,
    FOREIGN KEY(account_id, auth_profile_id) REFERENCES auth_profiles(account_id, id)
        DEFERRABLE INITIALLY DEFERRED
) STRICT;
-- Protocol identifiers stay behind the shared local message identity.
CREATE TABLE ews_items (
    account_id TEXT NOT NULL REFERENCES accounts(id),
    mailbox_source_id TEXT NOT NULL,
    folder_id TEXT NOT NULL REFERENCES folders(id) ON DELETE CASCADE,
    item_id TEXT NOT NULL,
    change_key TEXT,
    uid INTEGER NOT NULL CHECK (uid > 0 AND uid <= 4294967295),
    PRIMARY KEY (folder_id, item_id),
    UNIQUE (folder_id, uid)
) STRICT;
CREATE TABLE ews_folder_state (
    folder_id TEXT NOT NULL PRIMARY KEY REFERENCES folders(id) ON DELETE CASCADE,
    highest_uid INTEGER NOT NULL DEFAULT 0 CHECK (highest_uid >= 0 AND highest_uid <= 4294967295)
) STRICT;
