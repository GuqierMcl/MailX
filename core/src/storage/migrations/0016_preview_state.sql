ALTER TABLE messages ADD COLUMN preview_state TEXT NOT NULL DEFAULT 'missing'
    CHECK (preview_state IN ('missing', 'ready', 'failed'));
UPDATE messages SET preview_state = 'ready' WHERE body_state = 'ready';
