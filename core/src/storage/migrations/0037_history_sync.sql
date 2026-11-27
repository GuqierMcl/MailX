-- Global/per-account history sync window override and per-folder historical backfill state.
ALTER TABLE accounts ADD COLUMN sync_window TEXT
    CHECK (sync_window IS NULL OR sync_window IN ('days30', 'days90', 'year1', 'all'));

ALTER TABLE folder_sync_cursors ADD COLUMN history_target_date TEXT;
ALTER TABLE folder_sync_cursors ADD COLUMN history_covered_since TEXT;
ALTER TABLE folder_sync_cursors ADD COLUMN history_cursor_before TEXT;
ALTER TABLE folder_sync_cursors ADD COLUMN history_fetched INTEGER NOT NULL DEFAULT 0;
ALTER TABLE folder_sync_cursors ADD COLUMN history_total INTEGER;
ALTER TABLE folder_sync_cursors ADD COLUMN history_state TEXT NOT NULL DEFAULT 'idle'
    CHECK (history_state IN ('idle', 'running', 'paused', 'completed', 'failed'));
ALTER TABLE folder_sync_cursors ADD COLUMN history_failure_code TEXT;
