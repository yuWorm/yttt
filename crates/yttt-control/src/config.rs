use std::{net::SocketAddr, time::Duration};

use serde::Deserialize;

/// The control plane's configuration, read from TOML.
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControlConfig {
    pub listen: SocketAddr,
    /// Where browsers and nodes reach the control plane, e.g.
    /// `https://control.example.com`. Decides whether the session cookie is
    /// `Secure`, and is the origin deploy commands download from.
    #[serde(default)]
    pub public_url: Option<String>,
    /// The built web console; without it only the API is served.
    #[serde(default)]
    pub web_root: Option<std::path::PathBuf>,
    /// Relay binaries the install script downloads, named
    /// `yttt-relay-linux-<arch>`, with a `SHA256SUMS` file beside them.
    #[serde(default)]
    pub relay_binaries_dir: Option<std::path::PathBuf>,
    /// Trust the last `X-Forwarded-For` hop as the client address: only behind
    /// exactly one reverse proxy that sets it.
    #[serde(default)]
    pub trusted_proxy: bool,
    #[serde(default)]
    pub signup: Option<SignupConfig>,
    pub database_url: String,
    /// Hex Ed25519 secret that signs relay tickets. Its public key goes into
    /// every node's `ticket_keys`.
    pub ticket_secret: String,
    /// Hex 32 bytes that seal secrets the console stores, such as OAuth
    /// client secrets. Without it those are configured here instead.
    #[serde(default)]
    pub secrets_key: Option<String>,
    #[serde(default = "default_ticket_ttl_secs")]
    pub ticket_ttl_secs: u64,
    #[serde(default = "default_session_ttl_secs")]
    pub session_ttl_secs: u64,
    /// Enables `/v1/dev/*`: account creation and suspension without billing.
    #[serde(default)]
    pub dev_mode: bool,
    pub plans: Vec<PlanConfig>,
    /// GitHub and Google sign-in on the console.
    #[serde(default)]
    pub oauth: OAuthConfig,
    /// Seeds for a fresh database only; nodes are managed in the console
    /// afterwards and never overwritten from here.
    #[serde(default)]
    pub nodes: Vec<NodeEntry>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OAuthConfig {
    #[serde(default)]
    pub github: Option<ProviderConfig>,
    #[serde(default)]
    pub google: Option<ProviderConfig>,
}

/// An OAuth application registered with GitHub or Google. Its callback is
/// `{public_url}/v1/auth/{provider}/callback`.
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderConfig {
    pub client_id: String,
    pub client_secret: String,
    /// Replace the provider's endpoints; for tests against a stand-in.
    #[serde(default)]
    pub authorize_url: Option<String>,
    #[serde(default)]
    pub token_url: Option<String>,
    /// GitHub's API base, or Google's userinfo endpoint.
    #[serde(default)]
    pub api_url: Option<String>,
}

impl std::fmt::Debug for ProviderConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ProviderConfig")
            .field("client_id", &self.client_id)
            .field("client_secret", &"[redacted]")
            .field("authorize_url", &self.authorize_url)
            .field("token_url", &self.token_url)
            .field("api_url", &self.api_url)
            .finish()
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignupConfig {
    pub enabled: bool,
    pub plan: String,
    pub node: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanConfig {
    pub id: String,
    /// Sustained upstream rate for the whole account.
    pub rate_bps: u32,
    pub burst_bytes: u32,
    /// Upstream bytes per calendar month before the account is throttled.
    pub monthly_bytes: u64,
    pub throttled_rate_bps: u32,
    pub max_devices: u32,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeEntry {
    pub id: String,
    /// The relay URL devices on this node put in their endpoint.
    pub url: String,
    /// The bearer token the node reports usage with.
    pub token: String,
}

impl ControlConfig {
    pub fn from_toml(text: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(text)
    }

    pub(crate) fn ticket_ttl(&self) -> Duration {
        Duration::from_secs(self.ticket_ttl_secs.clamp(3600, 365 * 24 * 3600))
    }

    /// Whether browsers reach the console over HTTPS.
    pub(crate) fn secure_cookies(&self) -> bool {
        self.public_url
            .as_deref()
            .is_some_and(|url| url.starts_with("https://"))
    }

    pub(crate) fn session_ttl(&self) -> Duration {
        Duration::from_secs(self.session_ttl_secs.max(60))
    }
}

impl std::fmt::Debug for ControlConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ControlConfig")
            .field("listen", &self.listen)
            .field("public_url", &self.public_url)
            .field("web_root", &self.web_root)
            .field("relay_binaries_dir", &self.relay_binaries_dir)
            .field("trusted_proxy", &self.trusted_proxy)
            .field("signup", &self.signup)
            .field("database_url", &"[redacted]")
            .field("ticket_secret", &"[redacted]")
            .field(
                "secrets_key",
                &self.secrets_key.as_ref().map(|_| "[redacted]"),
            )
            .field("ticket_ttl_secs", &self.ticket_ttl_secs)
            .field("session_ttl_secs", &self.session_ttl_secs)
            .field("dev_mode", &self.dev_mode)
            .field("plans", &self.plans)
            .field("oauth", &self.oauth)
            .field("nodes", &self.nodes)
            .finish()
    }
}

impl std::fmt::Debug for NodeEntry {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NodeEntry")
            .field("id", &self.id)
            .field("url", &self.url)
            .field("token", &"[redacted]")
            .finish()
    }
}

/// Tickets are long-lived: iroh reuses a relay connection's first token for
/// every reconnect. Devices renew well before expiry, and live control goes
/// through node usage reports instead.
fn default_ticket_ttl_secs() -> u64 {
    30 * 24 * 3600
}

fn default_session_ttl_secs() -> u64 {
    30 * 24 * 3600
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXAMPLE: &str = include_str!("../../../packaging/relay/control.example.toml");

    #[test]
    fn the_example_configuration_parses_with_and_without_oauth() {
        let config = ControlConfig::from_toml(EXAMPLE).unwrap();
        assert!(config.oauth.github.is_none());
        let enabled = EXAMPLE
            .lines()
            .map(|line| {
                let uncommented = line.strip_prefix("# ").unwrap_or(line);
                if uncommented.starts_with("[oauth.")
                    || uncommented.starts_with("client_id")
                    || uncommented.starts_with("client_secret")
                {
                    uncommented
                } else {
                    line
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        let config = ControlConfig::from_toml(&enabled).unwrap();
        assert!(config.oauth.github.is_some() && config.oauth.google.is_some());
    }
}
