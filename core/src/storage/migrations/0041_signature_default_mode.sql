ALTER TABLE signatures
    ADD COLUMN is_default INTEGER NOT NULL DEFAULT 0 CHECK (is_default IN (0, 1));

CREATE UNIQUE INDEX signatures_single_default
    ON signatures (is_default) WHERE is_default = 1;

CREATE TABLE account_signatures_next (
    account_id TEXT PRIMARY KEY REFERENCES accounts(id) ON DELETE CASCADE,
    mode TEXT NOT NULL CHECK (mode IN ('none', 'specific')),
    signature_id TEXT REFERENCES signatures(id) ON DELETE CASCADE,
    CHECK (
        (mode = 'none' AND signature_id IS NULL) OR
        (mode = 'specific' AND signature_id IS NOT NULL)
    )
);

INSERT INTO account_signatures_next (account_id, mode, signature_id)
SELECT account_id, 'specific', signature_id FROM account_signatures;

DROP TABLE account_signatures;
ALTER TABLE account_signatures_next RENAME TO account_signatures;

CREATE INDEX account_signatures_by_signature ON account_signatures (signature_id);
