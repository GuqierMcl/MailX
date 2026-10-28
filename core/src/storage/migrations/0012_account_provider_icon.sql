ALTER TABLE accounts ADD COLUMN use_provider_icon INTEGER NOT NULL DEFAULT 1
    CHECK (use_provider_icon IN (0, 1));
