-- Local drafts (docs/architecture/sending.md sections 7 and 10) and the name
-- recipients see in `From`, which belongs to the account profile.
--
-- `sender_name` is NULL when the account sends with the bare address.
ALTER TABLE accounts ADD COLUMN sender_name TEXT
    CHECK (sender_name IS NULL OR (length(sender_name) BETWEEN 1 AND 128));

-- Deleting an account deletes its drafts. `source_email_id` is only a pointer
-- for display and threading: the quoted text and headers were copied into the
-- draft when it was created, so a vanished source message must not take the
-- draft (the user's own writing) with it. It is cleared instead.
--
-- `to_addresses`, `cc_addresses` and `bcc_addresses` are JSON arrays of
-- { name: string | null, address: string }. `references_header` holds the
-- space-separated, angle-bracketed Message-IDs. `create_canonical` is the
-- canonical form of the request that created the draft, so a retried create
-- with the same `client_mutation_id` returns the draft (even after edits) and
-- the same key with different content is rejected.
CREATE TABLE drafts (
    id TEXT PRIMARY KEY CHECK (length(id) = 36),
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK (kind IN ('new', 'reply', 'replyAll', 'forward')),
    source_email_id TEXT REFERENCES messages(id) ON DELETE SET NULL,
    to_addresses TEXT NOT NULL CHECK (json_valid(to_addresses) AND json_type(to_addresses) = 'array'),
    cc_addresses TEXT NOT NULL CHECK (json_valid(cc_addresses) AND json_type(cc_addresses) = 'array'),
    bcc_addresses TEXT NOT NULL CHECK (json_valid(bcc_addresses) AND json_type(bcc_addresses) = 'array'),
    subject TEXT NOT NULL CHECK (length(CAST(subject AS BLOB)) <= 998),
    body_text TEXT NOT NULL CHECK (length(CAST(body_text AS BLOB)) <= 1048576),
    in_reply_to TEXT,
    references_header TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    revision INTEGER NOT NULL DEFAULT 1 CHECK (revision > 0),
    client_mutation_id TEXT NOT NULL CHECK (length(client_mutation_id) = 36),
    create_canonical TEXT NOT NULL,
    UNIQUE (account_id, client_mutation_id)
) STRICT;

CREATE INDEX drafts_by_updated ON drafts (updated_at DESC, id DESC);
CREATE INDEX drafts_by_account ON drafts (account_id, updated_at DESC, id DESC);
