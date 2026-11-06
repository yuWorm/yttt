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
    #[serde(default = "default_ticket_ttl_secs")]
    pub ticket_ttl_secs: u64,
    #[serde(default = "default_session_ttl_secs")]
    pub session_ttl_secs: u64,
    /// Enables `/v1/dev/*`: account creation and suspension without billing.
    #[serde(default)]
    pub dev_mode: bool,
    pub plans: Vec<PlanConfig>,
    /// Seeds for a fresh database only; nodes are managed in the console
    /// afterwards and never overwritten from here.
    #[serde(default)]
    pub nodes: Vec<NodeEntry>,
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
            .field("ticket_ttl_secs", &self.ticket_ttl_secs)
            .field("session_ttl_secs", &self.session_ttl_secs)
            .field("dev_mode", &self.dev_mode)
            .field("plans", &self.plans)
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
