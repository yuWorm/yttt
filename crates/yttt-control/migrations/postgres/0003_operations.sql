-- Plans and sign-up are run from the console; the configuration only seeds
-- them into an empty database.

ALTER TABLE plans
    ADD COLUMN name TEXT NOT NULL DEFAULT '',
    ADD COLUMN description TEXT NOT NULL DEFAULT '',
    -- An archived plan keeps its accounts but takes no new ones.
    ADD COLUMN archived BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN sort INTEGER NOT NULL DEFAULT 0,
    ADD COLUMN created_unix BIGINT NOT NULL DEFAULT 0;
UPDATE plans SET name = id WHERE name = '';

-- One row, written when the database is first seeded.
CREATE TABLE settings (
    id BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK (id),
    signup_enabled BOOLEAN NOT NULL,
    signup_plan TEXT REFERENCES plans (id),
    signup_node TEXT REFERENCES nodes (id)
);

-- Relayed bytes per account, node and UTC day (days since the Unix epoch).
CREATE TABLE usage_days (
    account_id BIGINT NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    day INTEGER NOT NULL,
    node_id TEXT NOT NULL REFERENCES nodes (id) ON DELETE CASCADE,
    upstream_bytes BIGINT NOT NULL DEFAULT 0,
    PRIMARY KEY (account_id, day, node_id)
);
CREATE INDEX usage_days_by_day ON usage_days (day);
CREATE INDEX usage_days_by_node ON usage_days (node_id, day);
