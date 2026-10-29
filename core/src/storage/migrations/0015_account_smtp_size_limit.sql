ALTER TABLE accounts ADD COLUMN smtp_size_limit INTEGER
    CHECK (smtp_size_limit IS NULL OR smtp_size_limit > 0);
