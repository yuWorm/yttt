-- GitHub and Google sign-in, set up in the console. The client secret is
-- sealed with a key from the configuration, so the database alone does not
-- reveal it.
CREATE TABLE oauth_providers (
    provider TEXT PRIMARY KEY CHECK (provider IN ('github', 'google')),
    client_id TEXT NOT NULL,
    client_secret_sealed BYTEA NOT NULL,
    nonce BYTEA NOT NULL,
    enabled BOOLEAN NOT NULL,
    updated_unix BIGINT NOT NULL
);
