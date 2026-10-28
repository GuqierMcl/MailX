-- The queued MIME snapshot moves out of `outbox.mime` into the content-addressed
-- blob store (docs/architecture/sending.md section 16.6): with attachments a
-- snapshot can be tens of megabytes, which does not belong in a SQLite row.
--
-- `snapshot_hash` / `snapshot_size` identify the blob. They are NULL on a row
-- whose snapshot was released (`snapshot_released_at` is set: a failed or
-- canceled row removed from the queue or superseded by a re-queue) and, until
-- the startup move runs, on a row that still carries its bytes in `mime`.
-- `mime` is now nullable and new rows leave it NULL; the startup move copies
-- legacy bytes into the blob store and then clears it. The move is not SQL, see
-- docs/architecture/database-migrations.md.
--
-- SQLite cannot relax NOT NULL in place, so the table is rebuilt. Foreign keys
-- stay on: the only reference to `outbox` is its own `superseded_by`, filled in
-- after every row exists.
DROP INDEX outbox_due;
DROP INDEX outbox_by_account;
DROP INDEX outbox_by_draft;
DROP INDEX outbox_by_message_id;
DROP INDEX outbox_sent;
ALTER TABLE outbox RENAME TO outbox_old;

CREATE TABLE outbox (
    id TEXT PRIMARY KEY CHECK (length(id) = 36),
    draft_id TEXT REFERENCES drafts(id) ON DELETE SET NULL,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    client_mutation_id TEXT NOT NULL CHECK (length(client_mutation_id) = 36),
    queue_canonical TEXT NOT NULL,
    message_id TEXT NOT NULL CHECK (length(message_id) BETWEEN 3 AND 998),
    mime BLOB,
    snapshot_hash TEXT CHECK (snapshot_hash IS NULL OR length(snapshot_hash) = 64),
    snapshot_size INTEGER CHECK (snapshot_size IS NULL OR snapshot_size >= 0),
    snapshot_released_at TEXT,
    envelope TEXT NOT NULL CHECK (json_valid(envelope) AND json_type(envelope) = 'object'),
    subject TEXT NOT NULL,
    recipients_preview TEXT NOT NULL CHECK (json_valid(recipients_preview) AND json_type(recipients_preview) = 'array'),
    recipient_count INTEGER NOT NULL CHECK (recipient_count > 0),
    state TEXT NOT NULL CHECK (state IN ('queued', 'sending', 'accepted', 'failed', 'unknown', 'canceled')),
    not_before TEXT NOT NULL,
    data_started_at TEXT,
    attempts INTEGER NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    next_attempt_at TEXT,
    first_queued_at TEXT NOT NULL,
    last_error TEXT CHECK (last_error IS NULL OR (json_valid(last_error) AND json_type(last_error) = 'object')),
    accepted_at TEXT,
    server_reply TEXT CHECK (server_reply IS NULL OR (json_valid(server_reply) AND json_type(server_reply) = 'object')),
    server_copy_message_id TEXT,
    deleted_locally_at TEXT,
    resolution TEXT CHECK (resolution IS NULL OR resolution IN ('markedSent', 'resent')),
    resolution_mutation_id TEXT CHECK (resolution_mutation_id IS NULL OR length(resolution_mutation_id) = 36),
    superseded_by TEXT REFERENCES outbox(id) ON DELETE SET NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE (account_id, client_mutation_id),
    CHECK ((snapshot_hash IS NULL) = (snapshot_size IS NULL)),
    CHECK (state <> 'accepted' OR accepted_at IS NOT NULL),
    CHECK (data_started_at IS NULL OR state NOT IN ('queued', 'canceled')),
    CHECK (
        resolution IS NULL
        OR (resolution = 'markedSent' AND state = 'accepted')
        OR (resolution = 'resent' AND state = 'unknown')
    ),
    CHECK (resolution IS NULL OR resolution_mutation_id IS NOT NULL)
) STRICT;

INSERT INTO outbox (
    id, draft_id, account_id, client_mutation_id, queue_canonical, message_id,
    mime, envelope, subject, recipients_preview, recipient_count, state,
    not_before, data_started_at, attempts, next_attempt_at, first_queued_at,
    last_error, accepted_at, server_reply, server_copy_message_id,
    deleted_locally_at, resolution, resolution_mutation_id, superseded_by,
    created_at, updated_at
)
SELECT
    id, draft_id, account_id, client_mutation_id, queue_canonical, message_id,
    mime, envelope, subject, recipients_preview, recipient_count, state,
    not_before, data_started_at, attempts, next_attempt_at, first_queued_at,
    last_error, accepted_at, server_reply, server_copy_message_id,
    deleted_locally_at, resolution, resolution_mutation_id, NULL,
    created_at, updated_at
FROM outbox_old;

UPDATE outbox SET superseded_by = (
    SELECT old.superseded_by FROM outbox_old AS old WHERE old.id = outbox.id
);

DROP TABLE outbox_old;

-- The next due item of an account, and the time to wake for.
CREATE INDEX outbox_due ON outbox (account_id, not_before) WHERE state = 'queued';
CREATE INDEX outbox_by_account ON outbox (account_id, created_at DESC, id DESC);
CREATE INDEX outbox_by_draft ON outbox (draft_id) WHERE draft_id IS NOT NULL;
CREATE INDEX outbox_by_message_id ON outbox (account_id, message_id);
-- Local Sent copies, newest first.
CREATE INDEX outbox_sent ON outbox (account_id, accepted_at DESC)
    WHERE state = 'accepted' AND deleted_locally_at IS NULL;
-- Snapshot blobs still referenced by a row.
CREATE INDEX outbox_by_snapshot ON outbox (snapshot_hash) WHERE snapshot_hash IS NOT NULL;
