-- How busy each node is: connections and relayed bytes per five minutes for
-- the last two days, and the daily peak for longer.
CREATE TABLE node_samples (
    node_id TEXT NOT NULL REFERENCES nodes (id) ON DELETE CASCADE,
    -- Unix time rounded down to five minutes.
    bucket_unix BIGINT NOT NULL,
    connections_max INTEGER NOT NULL,
    upstream_bytes BIGINT NOT NULL,
    PRIMARY KEY (node_id, bucket_unix)
);

CREATE TABLE node_days (
    node_id TEXT NOT NULL REFERENCES nodes (id) ON DELETE CASCADE,
    -- Days since the Unix epoch, UTC.
    day INTEGER NOT NULL,
    connections_max INTEGER NOT NULL,
    PRIMARY KEY (node_id, day)
);
