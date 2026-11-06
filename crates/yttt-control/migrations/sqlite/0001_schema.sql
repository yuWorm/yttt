-- The control plane's schema on SQLite, equivalent to migrations/postgres
-- 0001 through 0007. Columns use only INTEGER, TEXT and BLOB, the types every
-- query reads them as; booleans are 0 or 1. Later changes add a migration
-- here and one in migrations/postgres.

CREATE TABLE plans (
    id TEXT PRIMARY KEY,
    rate_bps INTEGER NOT NULL CHECK (rate_bps > 0),
    burst_bytes INTEGER NOT NULL CHECK (burst_bytes > 0),
    monthly_bytes INTEGER NOT NULL CHECK (monthly_bytes >= 0),
    throttled_rate_bps INTEGER NOT NULL CHECK (throttled_rate_bps > 0),
    max_devices INTEGER NOT NULL CHECK (max_devices > 0),
    name TEXT NOT NULL DEFAULT '',
    description TEXT NOT NULL DEFAULT '',
    archived INTEGER NOT NULL DEFAULT 0,
    sort INTEGER NOT NULL DEFAULT 0,
    created_unix INTEGER NOT NULL DEFAULT 0,
    max_nodes INTEGER NOT NULL DEFAULT 1 CHECK (max_nodes > 0)
);

CREATE TABLE nodes (
    id TEXT PRIMARY KEY,
    url TEXT NOT NULL,
    -- SHA-256 of the node's bearer token; empty until it enrolls.
    token_hash BLOB,
    name TEXT NOT NULL DEFAULT '',
    state TEXT NOT NULL DEFAULT 'active' CHECK (state IN ('pending', 'active', 'disabled')),
    created_unix INTEGER NOT NULL DEFAULT 0,
    enrolled_unix INTEGER,
    last_report_unix INTEGER,
    last_connected_devices INTEGER,
    version TEXT,
    reachable INTEGER,
    region TEXT NOT NULL DEFAULT '',
    max_connections INTEGER CHECK (max_connections > 0)
);

CREATE TABLE accounts (
    id INTEGER PRIMARY KEY,
    email TEXT NOT NULL UNIQUE,
    password_hash TEXT,
    plan_id TEXT NOT NULL REFERENCES plans (id),
    node_id TEXT NOT NULL REFERENCES nodes (id),
    suspended INTEGER NOT NULL DEFAULT 0,
    created_unix INTEGER NOT NULL,
    role TEXT NOT NULL DEFAULT 'user' CHECK (role IN ('user', 'admin'))
);

CREATE TABLE devices (
    endpoint_id BLOB PRIMARY KEY CHECK (length(endpoint_id) = 32),
    account_id INTEGER NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    label TEXT NOT NULL,
    registered_unix INTEGER NOT NULL,
    role TEXT NOT NULL DEFAULT 'unknown' CHECK (role IN ('host', 'client', 'unknown')),
    node_id TEXT NOT NULL REFERENCES nodes (id),
    follows_host BLOB REFERENCES devices (endpoint_id) ON DELETE CASCADE,
    node_changed_unix INTEGER NOT NULL DEFAULT 0,
    last_seen_unix INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX devices_by_account ON devices (account_id);
CREATE INDEX devices_by_node ON devices (node_id);
CREATE INDEX devices_by_host ON devices (follows_host);

CREATE TABLE sessions (
    token_hash BLOB PRIMARY KEY,
    account_id INTEGER NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    expires_unix INTEGER NOT NULL,
    scope TEXT NOT NULL CHECK (scope IN ('web', 'device')),
    signed_in_unix INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX sessions_by_account ON sessions (account_id);

CREATE TABLE usage_months (
    account_id INTEGER NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    month INTEGER NOT NULL,
    upstream_bytes INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (account_id, month)
);

CREATE TABLE node_enrollments (
    token_hash BLOB PRIMARY KEY,
    node_id TEXT NOT NULL REFERENCES nodes (id) ON DELETE CASCADE,
    created_unix INTEGER NOT NULL,
    expires_unix INTEGER NOT NULL,
    used_unix INTEGER
);
CREATE INDEX node_enrollments_by_node ON node_enrollments (node_id);

CREATE TABLE admin_actions (
    id INTEGER PRIMARY KEY,
    admin_id INTEGER NOT NULL,
    action TEXT NOT NULL,
    target_account INTEGER,
    target_node TEXT,
    detail TEXT NOT NULL DEFAULT '',
    at_unix INTEGER NOT NULL
);
CREATE INDEX admin_actions_by_time ON admin_actions (at_unix DESC);

-- One row, always written with id TRUE (1 here, a boolean on PostgreSQL).
CREATE TABLE settings (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    signup_enabled INTEGER NOT NULL,
    signup_plan TEXT REFERENCES plans (id),
    signup_node TEXT REFERENCES nodes (id)
);

CREATE TABLE usage_days (
    account_id INTEGER NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    day INTEGER NOT NULL,
    node_id TEXT NOT NULL REFERENCES nodes (id) ON DELETE CASCADE,
    upstream_bytes INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (account_id, day, node_id)
);
CREATE INDEX usage_days_by_day ON usage_days (day);
CREATE INDEX usage_days_by_node ON usage_days (node_id, day);

CREATE TABLE identities (
    provider TEXT NOT NULL,
    subject TEXT NOT NULL,
    account_id INTEGER NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    email TEXT NOT NULL,
    created_unix INTEGER NOT NULL,
    PRIMARY KEY (provider, subject)
);
CREATE UNIQUE INDEX identities_by_account ON identities (account_id, provider);

CREATE TABLE oauth_codes (
    code_hash BLOB PRIMARY KEY,
    account_id INTEGER NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    challenge TEXT NOT NULL,
    redirect_uri TEXT NOT NULL,
    label TEXT NOT NULL,
    expires_unix INTEGER NOT NULL
);

CREATE TABLE device_grants (
    device_code_hash BLOB PRIMARY KEY,
    user_code TEXT NOT NULL UNIQUE,
    label TEXT NOT NULL,
    requester_ip TEXT NOT NULL,
    account_id INTEGER REFERENCES accounts (id) ON DELETE CASCADE,
    denied INTEGER NOT NULL DEFAULT 0,
    created_unix INTEGER NOT NULL,
    expires_unix INTEGER NOT NULL,
    last_poll_unix INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE login_states (
    state_hash BLOB PRIMARY KEY,
    provider TEXT NOT NULL,
    verifier TEXT NOT NULL,
    next TEXT NOT NULL,
    link_account INTEGER REFERENCES accounts (id) ON DELETE CASCADE,
    expires_unix INTEGER NOT NULL
);

CREATE TABLE plan_nodes (
    plan_id TEXT NOT NULL REFERENCES plans (id) ON DELETE CASCADE,
    node_id TEXT NOT NULL REFERENCES nodes (id) ON DELETE CASCADE,
    PRIMARY KEY (plan_id, node_id)
);

CREATE TABLE node_revocations (
    id INTEGER PRIMARY KEY,
    node_id TEXT NOT NULL REFERENCES nodes (id) ON DELETE CASCADE,
    endpoint_id BLOB NOT NULL,
    revoked_unix INTEGER NOT NULL
);
CREATE INDEX node_revocations_by_node ON node_revocations (node_id, id);
CREATE INDEX node_revocations_by_time ON node_revocations (revoked_unix);

CREATE TABLE node_samples (
    node_id TEXT NOT NULL REFERENCES nodes (id) ON DELETE CASCADE,
    bucket_unix INTEGER NOT NULL,
    connections_max INTEGER NOT NULL,
    upstream_bytes INTEGER NOT NULL,
    PRIMARY KEY (node_id, bucket_unix)
);

CREATE TABLE node_days (
    node_id TEXT NOT NULL REFERENCES nodes (id) ON DELETE CASCADE,
    day INTEGER NOT NULL,
    connections_max INTEGER NOT NULL,
    PRIMARY KEY (node_id, day)
);

CREATE TABLE oauth_providers (
    provider TEXT PRIMARY KEY CHECK (provider IN ('github', 'google')),
    client_id TEXT NOT NULL,
    client_secret_sealed BLOB NOT NULL,
    nonce BLOB NOT NULL,
    enabled INTEGER NOT NULL,
    updated_unix INTEGER NOT NULL
);
