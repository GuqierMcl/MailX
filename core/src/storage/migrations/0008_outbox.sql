-- Send queue (docs/architecture/sending.md sections 5, 6 and 10).
--
-- One row per send attempt group: the MIME snapshot, envelope and Message-ID
-- are fixed when the row is queued and every delivery try sends the same bytes.
-- An `accepted` row is also the local Sent copy: it keeps `mime`, `accepted_at`
-- and the server reply for good (listing it in folders is a later step).
--
-- `draft_id` is cleared, not cascaded, when the draft goes away: the snapshot
-- is the user's mail and outlives the draft that produced it. Deleting the
-- account deletes its queue.
--
-- A draft is locked while it has a live row: `queued`, `sending`, or `unknown`
-- that has not been resolved (`resolution IS NULL`). `resolution` is set on an
-- `unknown` row the user settled: `markedSent` (the row became `accepted`) or
-- `resent` (a new row, `superseded_by`, carries the same Message-ID and MIME).
--
-- `envelope` is { from, recipients }. `recipients_preview` holds up to three
-- addresses for the list so it never parses MIME. `last_error` is
-- { reason, smtpCode, text, rejected: [{ address, code, text }] } and
-- `server_reply` is { code, text }; both hold sanitized server text only.
-- Timestamps are RFC 3339 UTC with milliseconds, so text order is time order.
CREATE TABLE outbox (
    id TEXT PRIMARY KEY CHECK (length(id) = 36),
    draft_id TEXT REFERENCES drafts(id) ON DELETE SET NULL,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    client_mutation_id TEXT NOT NULL CHECK (length(client_mutation_id) = 36),
    queue_canonical TEXT NOT NULL,
    message_id TEXT NOT NULL CHECK (length(message_id) BETWEEN 3 AND 998),
    mime BLOB NOT NULL,
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
    CHECK (state <> 'accepted' OR accepted_at IS NOT NULL),
    CHECK (data_started_at IS NULL OR state NOT IN ('queued', 'canceled')),
    CHECK (
        resolution IS NULL
        OR (resolution = 'markedSent' AND state = 'accepted')
        OR (resolution = 'resent' AND state = 'unknown')
    ),
    CHECK (resolution IS NULL OR resolution_mutation_id IS NOT NULL)
) STRICT;

-- The next due item of an account, and the time to wake for.
CREATE INDEX outbox_due ON outbox (account_id, not_before) WHERE state = 'queued';
CREATE INDEX outbox_by_account ON outbox (account_id, created_at DESC, id DESC);
CREATE INDEX outbox_by_draft ON outbox (draft_id) WHERE draft_id IS NOT NULL;
CREATE INDEX outbox_by_message_id ON outbox (account_id, message_id);
-- Local Sent copies, newest first.
CREATE INDEX outbox_sent ON outbox (account_id, accepted_at DESC)
    WHERE state = 'accepted' AND deleted_locally_at IS NULL;
