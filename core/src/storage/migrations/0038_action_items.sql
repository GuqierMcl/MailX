CREATE TABLE message_action_extractions (
    message_id TEXT PRIMARY KEY NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    source_sha256 TEXT NOT NULL,
    prompt_version INTEGER NOT NULL,
    items_json TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE message_action_states (
    message_id TEXT NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    item_key TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('added', 'dismissed')),
    updated_at TEXT NOT NULL,
    PRIMARY KEY (message_id, item_key)
);
