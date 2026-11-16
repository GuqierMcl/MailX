-- Chat conversations with the built-in assistant, kept on this device.
-- A conversation is a header row plus its messages in order. The message
-- content is JSON written by the core (model history, tool results with the
-- activity the interface shows, and a closing row per finished reply); it is
-- never sent anywhere by this table's existence. account_id is deliberately
-- not a foreign key: it is the set of accounts a conversation's tool calls
-- touched, and account removal deletes those conversations explicitly.
CREATE TABLE assistant_conversations (
    id TEXT PRIMARY KEY,
    title TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    -- User messages in the conversation.
    message_count INTEGER NOT NULL CHECK (message_count >= 0)
) STRICT;
CREATE INDEX assistant_conversations_recent
    ON assistant_conversations(updated_at DESC, id DESC);

CREATE TABLE assistant_conversation_accounts (
    conversation_id TEXT NOT NULL REFERENCES assistant_conversations(id) ON DELETE CASCADE,
    account_id TEXT NOT NULL,
    PRIMARY KEY (conversation_id, account_id)
) STRICT, WITHOUT ROWID;
CREATE INDEX assistant_conversation_accounts_by_account
    ON assistant_conversation_accounts(account_id);

CREATE TABLE assistant_messages (
    conversation_id TEXT NOT NULL REFERENCES assistant_conversations(id) ON DELETE CASCADE,
    seq INTEGER NOT NULL CHECK (seq >= 0),
    role TEXT NOT NULL CHECK (role IN ('user', 'assistant', 'tool', 'end')),
    content TEXT NOT NULL,
    created_at TEXT NOT NULL,
    PRIMARY KEY (conversation_id, seq)
) STRICT, WITHOUT ROWID;
