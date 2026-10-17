CREATE TABLE schema_migrations (
    version INTEGER PRIMARY KEY,
    checksum TEXT NOT NULL
) STRICT;
CREATE TABLE accounts (
    id TEXT PRIMARY KEY,
    revision INTEGER NOT NULL CHECK (revision > 0),
    display_name TEXT NOT NULL,
    email_address TEXT NOT NULL,
    provider_id TEXT,
    mailbox_source_id TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
) STRICT;
CREATE TABLE auth_profiles (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL REFERENCES accounts(id),
    kind TEXT NOT NULL CHECK (kind IN ('password', 'oauth2')),
    config_version INTEGER NOT NULL CHECK (config_version = 1),
    generation INTEGER NOT NULL CHECK (generation > 0),
    secret_ref TEXT NOT NULL UNIQUE,
    trusted_endpoints_json TEXT NOT NULL,
    UNIQUE(account_id, id)
) STRICT;
CREATE TABLE mail_connections (
    account_id TEXT NOT NULL REFERENCES accounts(id),
    protocol TEXT NOT NULL CHECK (protocol IN ('imap', 'smtp')),
    host TEXT NOT NULL,
    port INTEGER NOT NULL CHECK (port BETWEEN 1 AND 65535),
    tls TEXT NOT NULL CHECK (tls IN ('implicit', 'startTls')),
    username TEXT NOT NULL,
    auth_profile_id TEXT NOT NULL,
    PRIMARY KEY(account_id, protocol),
    FOREIGN KEY(account_id, auth_profile_id) REFERENCES auth_profiles(account_id, id)
        DEFERRABLE INITIALLY DEFERRED
) STRICT;
CREATE TABLE network_routes (
    account_id TEXT PRIMARY KEY REFERENCES accounts(id),
    revision INTEGER NOT NULL CHECK (revision > 0),
    kind TEXT NOT NULL CHECK (kind IN ('direct', 'inherit', 'custom')),
    proxy_kind TEXT CHECK (proxy_kind IN ('socks5', 'httpConnect')),
    host TEXT,
    port INTEGER CHECK (port BETWEEN 1 AND 65535),
    dns TEXT CHECK (dns = 'proxy'),
    username TEXT,
    secret_ref TEXT UNIQUE,
    generation INTEGER,
    trusted_endpoint_json TEXT,
    CHECK (
        (kind IN ('direct', 'inherit') AND proxy_kind IS NULL AND host IS NULL AND port IS NULL
            AND dns IS NULL AND username IS NULL AND secret_ref IS NULL AND generation IS NULL AND trusted_endpoint_json IS NULL)
        OR (kind = 'custom' AND proxy_kind IS NOT NULL AND host IS NOT NULL AND port IS NOT NULL AND dns IS NOT NULL AND dns = 'proxy'
            AND ((username IS NULL AND secret_ref IS NULL AND generation IS NULL AND trusted_endpoint_json IS NULL)
                OR (username IS NOT NULL AND secret_ref IS NOT NULL AND generation IS NOT NULL AND generation > 0 AND trusted_endpoint_json IS NOT NULL)))
    )
) STRICT;
CREATE TABLE create_requests (
    request_id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL REFERENCES accounts(id),
    canonical_config TEXT NOT NULL
) STRICT;
CREATE TABLE credential_journal (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL,
    expected_revision INTEGER,
    snapshot_json TEXT NOT NULL,
    new_refs_json TEXT NOT NULL,
    retired_refs_json TEXT NOT NULL,
    create_request_json TEXT,
    applied INTEGER NOT NULL CHECK (applied IN (0, 1))
) STRICT;
