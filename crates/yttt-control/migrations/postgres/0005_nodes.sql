-- Each Host chooses its node; a Client relays through the node of the Host it
-- connects to. The account's node becomes the default for new Hosts.

ALTER TABLE devices
    ADD COLUMN role TEXT NOT NULL DEFAULT 'unknown' CHECK (role IN ('host', 'client', 'unknown')),
    ADD COLUMN node_id TEXT REFERENCES nodes (id),
    -- A Client's Host, on the same account; removing the Host removes it.
    ADD COLUMN follows_host BYTEA REFERENCES devices (endpoint_id) ON DELETE CASCADE,
    ADD COLUMN node_changed_unix BIGINT NOT NULL DEFAULT 0,
    -- When a node last reported the device connected.
    ADD COLUMN last_seen_unix BIGINT NOT NULL DEFAULT 0;
UPDATE devices d SET node_id = a.node_id FROM accounts a WHERE a.id = d.account_id;
ALTER TABLE devices ALTER COLUMN node_id SET NOT NULL;
CREATE INDEX devices_by_node ON devices (node_id);
CREATE INDEX devices_by_host ON devices (follows_host);

-- How many nodes an account's Hosts may use at once; each node paces the
-- account on its own.
ALTER TABLE plans ADD COLUMN max_nodes INTEGER NOT NULL DEFAULT 1 CHECK (max_nodes > 0);

-- The nodes a plan may use. Every plan starts with every node.
CREATE TABLE plan_nodes (
    plan_id TEXT NOT NULL REFERENCES plans (id) ON DELETE CASCADE,
    node_id TEXT NOT NULL REFERENCES nodes (id) ON DELETE CASCADE,
    PRIMARY KEY (plan_id, node_id)
);
INSERT INTO plan_nodes (plan_id, node_id) SELECT p.id, n.id FROM plans p CROSS JOIN nodes n;

ALTER TABLE nodes
    ADD COLUMN region TEXT NOT NULL DEFAULT '',
    -- No new Hosts once this many devices are connected; empty for no limit.
    ADD COLUMN max_connections INTEGER CHECK (max_connections > 0);

-- Endpoints a node must refuse, kept longer than any ticket lives so a node
-- that restarts learns them again.
CREATE TABLE node_revocations (
    id BIGSERIAL PRIMARY KEY,
    node_id TEXT NOT NULL REFERENCES nodes (id) ON DELETE CASCADE,
    endpoint_id BYTEA NOT NULL,
    revoked_unix BIGINT NOT NULL
);
CREATE INDEX node_revocations_by_node ON node_revocations (node_id, id);
CREATE INDEX node_revocations_by_time ON node_revocations (revoked_unix);
