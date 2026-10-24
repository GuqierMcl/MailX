-- Legacy text-only rows are converted on read, then persisted on their next save.
-- Queued MIME snapshots are intentionally unaffected.
-- The input limit is enforced by the core. A valid old text body can expand
-- past it when escaped as HTML; unchanged legacy content must remain saveable.
ALTER TABLE drafts ADD COLUMN body_html TEXT;
