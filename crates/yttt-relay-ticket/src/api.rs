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

/// `POST /v1/oauth/token`: turns an approved sign-in into a session. A JSON
/// endpoint for yttt's own clients, shaped after OAuth 2.0 but not its form
/// encoding.
#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "grant_type", rename_all = "snake_case")]
pub enum TokenRequest {
    /// The app's browser sign-in (authorization code with PKCE).
    AuthorizationCode {
        code: String,
        code_verifier: String,
        redirect_uri: String,
    },
    /// A headless device whose short code someone confirmed.
    DeviceCode { device_code: String },
}

impl std::fmt::Debug for TokenRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AuthorizationCode { redirect_uri, .. } => formatter
                .debug_struct("AuthorizationCode")
                .field("redirect_uri", redirect_uri)
                .finish_non_exhaustive(),
            Self::DeviceCode { .. } => formatter.write_str("DeviceCode"),
        }
    }
}

/// Why `/v1/oauth/token` has no session yet, in the `error` of a 400.
pub mod token_error {
    /// Nobody has confirmed the short code yet; poll again after the interval.
    pub const PENDING: &str = "authorization_pending";
    /// Polled sooner than the interval.
    pub const SLOW_DOWN: &str = "slow_down";
    pub const DENIED: &str = "access_denied";
    pub const EXPIRED: &str = "expired_token";
    /// The code is unknown, used, or does not match its verifier or redirect.
    pub const INVALID: &str = "invalid_grant";
}

/// `POST /v1/oauth/device`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DeviceAuthorizationRequest {
    /// What the device calls itself; shown to whoever confirms the code.
    pub label: String,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct DeviceAuthorization {
    pub device_code: String,
    /// What a person types, e.g. `ABCD-EFGH`.
    pub user_code: String,
    /// The console page to confirm on, when the control plane knows its
    /// public address; otherwise `/device` under the address the device used.
    pub verification_uri: Option<String>,
    /// Seconds between polls.
    pub interval: u64,
    /// Seconds until the codes expire.
    pub expires_in: u64,
}

impl std::fmt::Debug for DeviceAuthorization {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DeviceAuthorization")
            .field("device_code", &"[redacted]")
            .field("user_code", &self.user_code)
            .field("verification_uri", &self.verification_uri)
            .field("interval", &self.interval)
            .field("expires_in", &self.expires_in)
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

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
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
    #[serde(default)]
    pub plan_name: String,
    #[serde(default)]
    pub plan_description: String,
    #[serde(default)]
    pub node_name: String,
    /// The plan's sustained rate, and its rate once the month's quota is used.
    #[serde(default)]
    pub rate_bps: u32,
    #[serde(default)]
    pub throttled_rate_bps: u32,
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
