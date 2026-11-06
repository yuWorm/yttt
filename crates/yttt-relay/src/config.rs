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
    /// Certificates from Let's Encrypt instead of files; needs `listen` on 443
    /// reachable from the internet.
    #[serde(default)]
    pub acme: Option<AcmeSettings>,
    /// Plain HTTP for captive-portal probes when `listen` is HTTPS.
    #[serde(default)]
    pub captive_portal_listen: Option<SocketAddr>,
    /// QUIC address discovery, which needs `tls`.
    #[serde(default)]
    pub quic_listen: Option<SocketAddr>,
    /// A certificate for QUIC alone, from files another program renews:
    /// for a node whose HTTP sits in the clear behind a reverse proxy that
    /// holds the certificate. Not with `[tls]` or `[acme]`.
    #[serde(default)]
    pub quic_tls: Option<TlsFiles>,
    /// Hex Ed25519 keys whose tickets this node accepts: the control plane's
    /// current key, and its previous one during a rotation.
    #[serde(default)]
    pub ticket_keys: Vec<String>,
    /// Self-hosted mode: relay for any endpoint without a ticket. Traffic
    /// stays end-to-end encrypted, but anyone who finds the node can use its
    /// bandwidth up to this rate per endpoint.
    #[serde(default)]
    pub open: Option<OpenConfig>,
    /// Where usage goes. Without it the node relays and limits on tickets
    /// alone and nothing is billed, which is only for local development.
    #[serde(default)]
    pub control: Option<ControlConfig>,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpenConfig {
    pub rate_bps: u32,
    pub burst_bytes: u32,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AcmeSettings {
    pub domains: Vec<String>,
    #[serde(default)]
    pub contact: Vec<String>,
    /// Where the account key and certificates persist; losing it means asking
    /// Let's Encrypt again, which it limits.
    pub cache_dir: PathBuf,
    /// Let's Encrypt's staging directory, for trying a setup out.
    #[serde(default)]
    pub staging: bool,
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
    #[serde(default)]
    pub node_token: String,
    /// Reads the token from a file instead. A relative path is looked up in
    /// systemd's `$CREDENTIALS_DIRECTORY` when set, else beside the config.
    #[serde(default)]
    pub node_token_file: Option<PathBuf>,
    #[serde(default = "default_report_interval_secs")]
    pub report_interval_secs: u64,
}

impl ControlConfig {
    pub fn from_parts(url: String, node_token: String, report_interval_secs: u64) -> Self {
        Self {
            url,
            node_token,
            node_token_file: None,
            report_interval_secs,
        }
    }

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
            acme: None,
            captive_portal_listen: None,
            quic_listen: None,
            quic_tls: None,
            ticket_keys: ticket_keys
                .iter()
                .map(|key| yttt_relay_ticket::hex(key))
                .collect(),
            open: None,
            control: None,
        }
    }

    pub fn from_toml(text: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(text)
    }

    /// Reads a config file and the node token it points at.
    pub fn load(path: &std::path::Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|error| error.to_string())?;
        let mut config = Self::from_toml(&text).map_err(|error| error.to_string())?;
        if let Some(control) = &mut config.control
            && let Some(file) = &control.node_token_file
        {
            let file = if file.is_relative() {
                match std::env::var_os("CREDENTIALS_DIRECTORY") {
                    Some(directory) => PathBuf::from(directory).join(file),
                    None => path
                        .parent()
                        .unwrap_or_else(|| std::path::Path::new("."))
                        .join(file),
                }
            } else {
                file.clone()
            };
            control.node_token = std::fs::read_to_string(&file)
                .map_err(|error| format!("{}: {error}", file.display()))?
                .trim()
                .to_owned();
        }
        if config
            .control
            .as_ref()
            .is_some_and(|control| control.node_token.is_empty())
        {
            return Err("[control] needs node_token or node_token_file".into());
        }
        Ok(config)
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
