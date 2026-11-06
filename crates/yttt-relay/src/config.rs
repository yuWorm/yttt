use std::{net::SocketAddr, path::PathBuf, time::Duration};

use serde::Deserialize;

/// A relay node's configuration, read from TOML.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeConfig {
    /// Must match the `node_id` the control plane writes into tickets.
    pub node_id: String,
    /// Serves the relay protocol; HTTPS when `tls` is set, plain HTTP otherwise.
    pub listen: SocketAddr,
    #[serde(default)]
    pub tls: Option<TlsFiles>,
    /// Plain HTTP for captive-portal probes when `listen` is HTTPS.
    #[serde(default)]
    pub captive_portal_listen: Option<SocketAddr>,
    /// QUIC address discovery, which needs `tls`.
    #[serde(default)]
    pub quic_listen: Option<SocketAddr>,
    /// Hex Ed25519 keys whose tickets this node accepts: the control plane's
    /// current key, and its previous one during a rotation.
    pub ticket_keys: Vec<String>,
    /// Where usage goes. Without it the node relays and limits on tickets
    /// alone and nothing is billed, which is only for local development.
    #[serde(default)]
    pub control: Option<ControlConfig>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TlsFiles {
    pub cert_chain: PathBuf,
    pub private_key: PathBuf,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControlConfig {
    pub url: String,
    pub node_token: String,
    #[serde(default = "default_report_interval_secs")]
    pub report_interval_secs: u64,
}

impl ControlConfig {
    pub fn report_interval(&self) -> Duration {
        Duration::from_secs(self.report_interval_secs.max(1))
    }
}

impl std::fmt::Debug for ControlConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ControlConfig")
            .field("url", &self.url)
            .field("node_token", &"[redacted]")
            .field("report_interval_secs", &self.report_interval_secs)
            .finish()
    }
}

fn default_report_interval_secs() -> u64 {
    15
}

impl NodeConfig {
    /// A plain-HTTP node with no control plane, as tests and local runs use.
    pub fn local(node_id: impl Into<String>, listen: SocketAddr, ticket_keys: &[[u8; 32]]) -> Self {
        Self {
            node_id: node_id.into(),
            listen,
            tls: None,
            captive_portal_listen: None,
            quic_listen: None,
            ticket_keys: ticket_keys
                .iter()
                .map(|key| yttt_relay_ticket::hex(key))
                .collect(),
            control: None,
        }
    }

    pub fn from_toml(text: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_production_config_reads_from_toml() {
        let config = NodeConfig::from_toml(
            r#"
            node_id = "tokyo-1"
            listen = "0.0.0.0:443"
            captive_portal_listen = "0.0.0.0:80"
            quic_listen = "0.0.0.0:7842"
            ticket_keys = ["0101010101010101010101010101010101010101010101010101010101010101"]

            [tls]
            cert_chain = "/etc/yttt-relay/fullchain.pem"
            private_key = "/etc/yttt-relay/privkey.pem"

            [control]
            url = "https://control.example"
            node_token = "secret"
            "#,
        )
        .unwrap();
        assert_eq!(config.node_id, "tokyo-1");
        assert_eq!(
            config.control.as_ref().unwrap().report_interval(),
            Duration::from_secs(15)
        );
        assert!(!format!("{config:?}").contains("secret"));
        assert!(
            NodeConfig::from_toml(
                "node_id = \"x\"\nlisten = \"0.0.0.0:1\"\nticket_keys = []\nport = 1"
            )
            .is_err()
        );
    }
}
