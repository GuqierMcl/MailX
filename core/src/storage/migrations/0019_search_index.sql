-- Local full-text index (storage-search.md 5.1). The FTS table keeps its own
-- copy of the text so snippet() works; search_docs maps rowids to email ids
-- (server messages.id, or local-sent:<outbox id> for local sent copies).
CREATE VIRTUAL TABLE search_fts USING fts5(
    subject, people, preview, body, attachments,
    tokenize = 'trigram case_sensitive 0 remove_diacritics 1'
);

CREATE TABLE search_docs (
    rowid INTEGER PRIMARY KEY,
    email_id TEXT NOT NULL UNIQUE,
    has_body INTEGER NOT NULL DEFAULT 0 CHECK (has_body IN (0, 1))
);

CREATE TABLE search_index_state (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    version INTEGER NOT NULL,
    backfill_done INTEGER NOT NULL DEFAULT 0 CHECK (backfill_done IN (0, 1)),
    messages_cursor TEXT NOT NULL DEFAULT '',
    local_done INTEGER NOT NULL DEFAULT 0 CHECK (local_done IN (0, 1))
);
INSERT INTO search_index_state (id, version, backfill_done) VALUES (1, 1, 0);

-- Deleting a message (folder or account cascades included) or an outbox row
-- drops its index row in the same statement; no Rust path can forget it.
CREATE TRIGGER search_docs_message_delete AFTER DELETE ON messages
BEGIN
    DELETE FROM search_fts WHERE rowid IN (SELECT rowid FROM search_docs WHERE email_id = old.id);
    DELETE FROM search_docs WHERE email_id = old.id;
END;

CREATE TRIGGER search_docs_outbox_delete AFTER DELETE ON outbox
BEGIN
    DELETE FROM search_fts WHERE rowid IN (SELECT rowid FROM search_docs WHERE email_id = 'local-sent:' || old.id);
    DELETE FROM search_docs WHERE email_id = 'local-sent:' || old.id;
END;
