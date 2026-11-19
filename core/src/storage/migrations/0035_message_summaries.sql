-- Cached AI summaries of messages. A row is only used while its source hash
-- (subject and body text) and prompt version match, so a changed body or new
-- prompt makes it stale without any cleanup pass.
CREATE TABLE message_summaries (
    message_id TEXT NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    style TEXT NOT NULL,
    -- A translation target code, or 'original' for the email's own language.
    language TEXT NOT NULL,
    source_sha256 TEXT NOT NULL,
    prompt_version INTEGER NOT NULL,
    text TEXT NOT NULL,
    created_at TEXT NOT NULL,
    PRIMARY KEY (message_id, style, language)
) STRICT;
