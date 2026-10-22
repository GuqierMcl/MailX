-- Local read / starred changes that the server has not confirmed yet. One row
-- per message and flag, holding only the latest wanted value, so toggling a
-- star twice offline writes back once (or not at all) instead of replaying.
--
-- `state`: 'pending' is waiting to be sent; 'local' is final and kept only on
-- this device, because the server cannot store that flag.
CREATE TABLE flag_changes (
    message_id TEXT NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    flag TEXT NOT NULL CHECK (flag IN ('seen', 'flagged')),
    wanted INTEGER NOT NULL CHECK (wanted IN (0, 1)),
    state TEXT NOT NULL CHECK (state IN ('pending', 'local')) DEFAULT 'pending',
    attempts INTEGER NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (message_id, flag)
) STRICT;

CREATE INDEX flag_changes_pending ON flag_changes(state, message_id);

-- Whether a folder can store the starred flag, learned from the server's
-- PERMANENTFLAGS when the folder is opened for writing. NULL means unknown.
ALTER TABLE folders ADD COLUMN can_flag INTEGER CHECK (can_flag IS NULL OR can_flag IN (0, 1));
