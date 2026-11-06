-- Sign-in without a password: the app through the browser, headless Hosts
-- with a short code, and the console with GitHub or Google.

-- An account that signs in only through GitHub or Google has no password.
ALTER TABLE accounts ALTER COLUMN password_hash DROP NOT NULL;

-- `web` sessions come from the console and travel in its cookie; `device`
-- sessions are bearer tokens for the app and the CLI, which reach only what a
-- device needs. Sessions from before cannot be told apart, so they become
-- `device`: the app keeps working and the console asks to sign in again.
ALTER TABLE sessions
    ADD COLUMN scope TEXT NOT NULL DEFAULT 'device' CHECK (scope IN ('web', 'device')),
    ADD COLUMN signed_in_unix BIGINT NOT NULL DEFAULT 0;
ALTER TABLE sessions ALTER COLUMN scope DROP DEFAULT;

-- A GitHub or Google identity linked to an account. `subject` is the
-- provider's unchanging id for the person, never their email.
CREATE TABLE identities (
    provider TEXT NOT NULL,
    subject TEXT NOT NULL,
    account_id BIGINT NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    email TEXT NOT NULL,
    created_unix BIGINT NOT NULL,
    PRIMARY KEY (provider, subject)
);
CREATE INDEX identities_by_account ON identities (account_id);

-- One-time codes the console hands the app's loopback redirect.
CREATE TABLE oauth_codes (
    -- SHA-256 of the code.
    code_hash BYTEA PRIMARY KEY,
    account_id BIGINT NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    -- base64url SHA-256 of the app's PKCE verifier.
    challenge TEXT NOT NULL,
    redirect_uri TEXT NOT NULL,
    label TEXT NOT NULL,
    expires_unix BIGINT NOT NULL
);

-- A headless device waiting for someone to confirm its short code.
CREATE TABLE device_grants (
    -- SHA-256 of the code the device polls with.
    device_code_hash BYTEA PRIMARY KEY,
    -- Normalized: upper case, no separator.
    user_code TEXT NOT NULL UNIQUE,
    label TEXT NOT NULL,
    requester_ip TEXT NOT NULL,
    -- Set once someone allows it; the next poll turns it into a session.
    account_id BIGINT REFERENCES accounts (id) ON DELETE CASCADE,
    denied BOOLEAN NOT NULL DEFAULT FALSE,
    created_unix BIGINT NOT NULL,
    expires_unix BIGINT NOT NULL,
    last_poll_unix BIGINT NOT NULL DEFAULT 0
);

-- A console sign-in or link in progress at GitHub or Google.
CREATE TABLE login_states (
    -- SHA-256 of the state the browser carries in its cookie and the URL.
    state_hash BYTEA PRIMARY KEY,
    provider TEXT NOT NULL,
    verifier TEXT NOT NULL,
    next TEXT NOT NULL,
    -- Set when a signed-in account links a provider instead of signing in.
    link_account BIGINT REFERENCES accounts (id) ON DELETE CASCADE,
    expires_unix BIGINT NOT NULL
);
