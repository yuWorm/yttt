-- The web console and nodes managed from it.

ALTER TABLE accounts
    ADD COLUMN role TEXT NOT NULL DEFAULT 'user' CHECK (role IN ('user', 'admin'));

-- A node created in the console has no token until it enrolls.
ALTER TABLE nodes ALTER COLUMN token_hash DROP NOT NULL;
ALTER TABLE nodes
    ADD COLUMN name TEXT NOT NULL DEFAULT '',
    ADD COLUMN state TEXT NOT NULL DEFAULT 'active'
        CHECK (state IN ('pending', 'active', 'disabled')),
    ADD COLUMN created_unix BIGINT NOT NULL DEFAULT 0,
    ADD COLUMN enrolled_unix BIGINT,
    ADD COLUMN last_report_unix BIGINT,
    ADD COLUMN last_connected_devices INTEGER,
    ADD COLUMN version TEXT,
    -- Whether the control plane last reached the node's /healthz.
    ADD COLUMN reachable BOOLEAN;

-- One-time tokens that let a machine become a node.
CREATE TABLE node_enrollments (
    -- SHA-256 of the token.
    token_hash BYTEA PRIMARY KEY,
    node_id TEXT NOT NULL REFERENCES nodes (id) ON DELETE CASCADE,
    created_unix BIGINT NOT NULL,
    expires_unix BIGINT NOT NULL,
    used_unix BIGINT
);
CREATE INDEX node_enrollments_by_node ON node_enrollments (node_id);

CREATE TABLE admin_actions (
    id BIGSERIAL PRIMARY KEY,
    admin_id BIGINT NOT NULL,
    action TEXT NOT NULL,
    target_account BIGINT,
    target_node TEXT,
    detail TEXT NOT NULL DEFAULT '',
    at_unix BIGINT NOT NULL
);
CREATE INDEX admin_actions_by_time ON admin_actions (at_unix DESC);
