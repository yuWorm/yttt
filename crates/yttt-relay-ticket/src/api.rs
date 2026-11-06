//! JSON bodies the control plane exchanges with relay nodes and with devices.

use serde::{Deserialize, Serialize};

/// What a node reports every interval: bytes each account sent since the last
/// accepted report, and which of its devices are connected. Accounts with live
/// connections or a known standing appear even with zero bytes, so the node
/// keeps learning their standing.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageReport {
    pub accounts: Vec<AccountUsage>,
    /// The node's software version, shown in the console.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountUsage {
    pub account_id: u64,
    pub upstream_bytes: u64,
    /// Hex EndpointIds of the account's devices connected to the node.
    #[serde(default)]
    pub endpoints: Vec<String>,
}

/// The control plane's answer: the standing of every account in the report.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageReply {
    pub accounts: Vec<AccountStanding>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountStanding {
    pub account_id: u64,
    pub standing: Standing,
    /// Reported endpoints that are no longer the account's devices. The node
    /// drops them and refuses their tickets issued before now.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub revoked_endpoints: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Standing {
    /// Relay at the plan's current rate, which may differ from older tickets.
    Active { rate_bps: u32, burst_bytes: u32 },
    /// Over quota: relay, but no faster than this.
    Throttled { rate_bps: u32, burst_bytes: u32 },
    /// Drop every connection and admit none.
    Suspended,
}

/// A [`crate::DeviceProof`] as JSON, keys and signatures in hex.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceProofBody {
    pub endpoint_id: String,
    pub issued_unix: u64,
    pub signature: String,
}

impl From<&crate::DeviceProof> for DeviceProofBody {
    fn from(proof: &crate::DeviceProof) -> Self {
        Self {
            endpoint_id: crate::hex(&proof.endpoint_id),
            issued_unix: proof.issued_unix,
            signature: crate::hex(&proof.signature),
        }
    }
}

impl DeviceProofBody {
    pub fn decode(&self) -> Option<crate::DeviceProof> {
        Some(crate::DeviceProof {
            endpoint_id: crate::parse_key(&self.endpoint_id)?,
            issued_unix: self.issued_unix,
            signature: crate::parse_signature(&self.signature)?,
        })
    }
}

/// Development-only account creation.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CreateAccount {
    pub email: String,
    pub password: String,
    pub plan: String,
    pub node: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountCreated {
    pub account_id: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Login {
    pub email: String,
    pub password: String,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct SessionGranted {
    pub token: String,
    pub account_id: u64,
    pub expires_unix: u64,
}

impl std::fmt::Debug for SessionGranted {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SessionGranted")
            .field("token", &"[redacted]")
            .field("account_id", &self.account_id)
            .field("expires_unix", &self.expires_unix)
            .finish()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RegisterDevice {
    pub label: String,
    pub proof: DeviceProofBody,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceEntry {
    pub endpoint_id: String,
    pub label: String,
    pub registered_unix: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceList {
    pub devices: Vec<DeviceEntry>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TicketRequest {
    pub proof: DeviceProofBody,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct TicketGranted {
    pub token: String,
    pub node_id: String,
    pub relay_url: String,
    pub expires_unix: u64,
}

impl std::fmt::Debug for TicketGranted {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TicketGranted")
            .field("token", &"[redacted]")
            .field("node_id", &self.node_id)
            .field("relay_url", &self.relay_url)
            .field("expires_unix", &self.expires_unix)
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountView {
    pub account_id: u64,
    pub email: String,
    pub plan: String,
    pub node_id: String,
    pub relay_url: String,
    pub month_upstream_bytes: u64,
    pub monthly_bytes: u64,
    pub suspended: bool,
    pub devices: u32,
    pub max_devices: u32,
    /// `user` or `admin`.
    #[serde(default)]
    pub role: String,
}

/// Development-only suspension switch.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct SetSuspended {
    pub suspended: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiError {
    pub error: String,
}
