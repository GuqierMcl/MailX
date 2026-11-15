-- Whether this generation has finished at least one sync round that was not
-- capped. Unlike backfill_done, which every round reopens until it ends, this
-- stays set while later rounds run, so readers that only need "the history was
-- fetched once" do not flicker during each round. A capped round clears it and
-- a new generation starts at 0.
ALTER TABLE folder_sync_cursors ADD COLUMN history_complete INTEGER NOT NULL DEFAULT 0
    CHECK (history_complete IN (0, 1));
UPDATE folder_sync_cursors SET history_complete = 1
    WHERE backfill_done = 1 AND window_capped = 0;
