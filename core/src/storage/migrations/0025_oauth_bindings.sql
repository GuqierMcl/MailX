ALTER TABLE auth_profiles ADD COLUMN oauth_binding_json TEXT;
CREATE TABLE oauth_client_registrations (
    id TEXT PRIMARY KEY NOT NULL,
    provider_id TEXT NOT NULL,
    configuration_json TEXT NOT NULL
);
ALTER TABLE outbox ADD COLUMN retry_started_at TEXT;
UPDATE outbox SET retry_started_at = first_queued_at WHERE attempts > 0;
ALTER TABLE outbox ADD COLUMN authorization_blocked_at TEXT;
