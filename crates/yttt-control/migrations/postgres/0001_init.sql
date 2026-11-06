-- Plans and nodes mirror the control plane's configuration; it upserts them
-- at startup so accounts can reference them.
CREATE TABLE plans (
    id TEXT PRIMARY KEY,
    rate_bps INTEGER NOT NULL CHECK (rate_bps > 0),
    burst_bytes INTEGER NOT NULL CHECK (burst_bytes > 0),
    monthly_bytes BIGINT NOT NULL CHECK (monthly_bytes >= 0),
    throttled_rate_bps INTEGER NOT NULL CHECK (throttled_rate_bps > 0),
    max_devices INTEGER NOT NULL CHECK (max_devices > 0)
);

CREATE TABLE nodes (
    id TEXT PRIMARY KEY,
    url TEXT NOT NULL,
    -- SHA-256 of the node's bearer token; the token itself is never stored.
    token_hash BYTEA NOT NULL
);

CREATE TABLE accounts (
    id BIGSERIAL PRIMARY KEY,
    email TEXT NOT NULL UNIQUE,
    password_hash TEXT NOT NULL,
    plan_id TEXT NOT NULL REFERENCES plans (id),
    node_id TEXT NOT NULL REFERENCES nodes (id),
    suspended BOOLEAN NOT NULL DEFAULT FALSE,
    created_unix BIGINT NOT NULL
);

-- An EndpointId belongs to at most one account.
CREATE TABLE devices (
    endpoint_id BYTEA PRIMARY KEY CHECK (length(endpoint_id) = 32),
    account_id BIGINT NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    label TEXT NOT NULL,
    registered_unix BIGINT NOT NULL
);
CREATE INDEX devices_by_account ON devices (account_id);

CREATE TABLE sessions (
    -- SHA-256 of the bearer token.
    token_hash BYTEA PRIMARY KEY,
    account_id BIGINT NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    expires_unix BIGINT NOT NULL
);
CREATE INDEX sessions_by_account ON sessions (account_id);

CREATE TABLE usage_months (
    account_id BIGINT NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    -- Year * 100 + month, in UTC.
    month INTEGER NOT NULL,
    upstream_bytes BIGINT NOT NULL DEFAULT 0,
    PRIMARY KEY (account_id, month)
);
