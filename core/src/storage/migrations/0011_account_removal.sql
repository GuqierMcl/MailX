-- Removal receipts survive the account so a retried request has the same
-- result, and a delayed create request cannot restore the removed account.
CREATE TABLE account_removals (
    account_id TEXT PRIMARY KEY,
    revision INTEGER NOT NULL CHECK (revision > 0),
    draft_ids TEXT NOT NULL CHECK (json_valid(draft_ids) AND json_type(draft_ids) = 'array')
) STRICT;

CREATE TABLE removed_create_requests (
    request_id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL REFERENCES account_removals(account_id)
) STRICT;

-- Contains opaque references only. The removal transaction owns these keys
-- before dropping configuration/journal references; deletion is idempotent and
-- resumes after a crash or a locked credential store without restoring data.
CREATE TABLE secret_gc (
    secret_ref TEXT NOT NULL,
    account_id TEXT NOT NULL REFERENCES account_removals(account_id),
    PRIMARY KEY (account_id, secret_ref)
) STRICT;
