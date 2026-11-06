use std::{net::SocketAddr, time::Duration};

use serde::Deserialize;

/// The control plane's configuration, read from TOML.
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControlConfig {
    pub listen: SocketAddr,
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
    pub nodes: Vec<NodeEntry>,
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
        Duration::from_secs(self.ticket_ttl_secs.clamp(60, 3600))
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

fn default_ticket_ttl_secs() -> u64 {
    600
}

fn default_session_ttl_secs() -> u64 {
    30 * 24 * 3600
}
