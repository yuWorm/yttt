use std::{fmt, net::SocketAddr};

use serde::{Deserialize, Serialize};
use yttt_core::model::ids::{ClientInstanceId, ProfileId};
use zeroize::Zeroize;

use crate::{
    EndpointKey,
    p2p::{DevicePreset, PairedDevice},
};

pub const DEFAULT_REMOTE_ADDRESS: &str = "127.0.0.1:43123";
pub const MAX_CONNECTION_INFO_BYTES: usize = 8 * 1024;
pub const TLS_SERVER_NAME: &str = "yttt-host.local";
pub const MAX_REMOTE_CLIENTS: usize = 4;
pub const MAX_REMOTE_STREAMS: usize = 256;
pub const MAX_PENDING_HANDSHAKES: usize = 16;

/// Device policy lives outside environment configuration and is only writable over local IPC.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RemoteAccessSettings {
    pub enabled: bool,
    pub listen_address: SocketAddr,
    pub login_startup_consent_granted: bool,
    pub p2p: P2pSettings,
}

/// The P2P ingress, independent of the TLS listener.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct P2pSettings {
    pub enabled: bool,
    /// The one relay the Host keeps a home connection to; `None` allows only direct paths.
    pub relay_url: Option<String>,
}

impl Default for RemoteAccessSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            listen_address: "127.0.0.1:43123".parse().expect("constant socket address"),
            login_startup_consent_granted: false,
            p2p: P2pSettings::default(),
        }
    }
}

/// Import out of band. The forwarded address is deliberately not part of Host identity.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteConnectionInfo {
    pub environment_id: String,
    pub profile_id: ProfileId,
    pub server_name: String,
    #[serde(with = "serde_bytes")]
    pub certificate_der: Vec<u8>,
    pub certificate_sha256: String,
    pub credential_generation: u64,
    pub work_secret: [u8; 32],
}

impl fmt::Debug for RemoteConnectionInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RemoteConnectionInfo")
            .field("environment_id", &self.environment_id)
            .field("profile_id", &self.profile_id)
            .field("certificate_sha256", &self.certificate_sha256)
            .field("credential_generation", &self.credential_generation)
            .finish_non_exhaustive()
    }
}

impl Drop for RemoteConnectionInfo {
    fn drop(&mut self) {
        self.work_secret.zeroize();
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum RemoteAccessState {
    #[default]
    Disabled,
    Starting,
    Listening,
    Stopping,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteClientSummary {
    pub client_id: ClientInstanceId,
    pub connected_millis: u64,
    pub streams: usize,
    /// The paired device behind a P2P session; `None` for TLS sessions.
    #[serde(default)]
    pub device: Option<EndpointKey>,
}

/// Text that grants access, such as a pairing code; redacted from debug output.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretText(pub String);

impl fmt::Debug for SecretText {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretText([redacted])")
    }
}

impl Drop for SecretText {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PairingState {
    /// Waiting for a device to scan the code.
    Open { expires_millis: u64 },
    /// A device proved it holds the code; the Host user compares `short_code`.
    AwaitingConfirmation {
        endpoint_id: EndpointKey,
        device_label: String,
        short_code: u32,
        expires_millis: u64,
    },
    /// A second device presented the same code; the invitation was withdrawn.
    Compromised,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct P2pStatus {
    pub effective: RemoteAccessState,
    pub endpoint_id: Option<EndpointKey>,
    pub error: Option<String>,
    pub pairing: Option<PairingState>,
    pub relay_account: Option<Box<RelayAccountStatus>>,
}

/// A paid-relay account the Host uses for its own relay ticket. It buys
/// transport only: nothing in it lets anyone reach the Host.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelayAccount {
    pub control_url: String,
    pub account_id: u64,
    /// The control-plane session the local user logged in with.
    pub session: SecretText,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RelayAccountStatus {
    pub control_url: String,
    pub account_id: u64,
    /// The relay the Host joined with its ticket, once it has one.
    pub relay_url: Option<String>,
    pub ticket_expires_unix: Option<u64>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteAccessStatus {
    pub settings: RemoteAccessSettings,
    pub effective: RemoteAccessState,
    pub bound_address: Option<SocketAddr>,
    pub error: Option<String>,
    pub clients: Vec<RemoteClientSummary>,
    #[serde(default)]
    pub p2p: P2pStatus,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RemoteAccessRequest {
    Status,
    SetEnabled {
        enabled: bool,
        listen_address: SocketAddr,
        confirm_non_loopback: bool,
    },
    ExportConnectionInfo,
    ResetCredentials,
    DisconnectClient {
        client_id: ClientInstanceId,
    },
    DisconnectAll,
    SetP2p {
        enabled: bool,
        relay_url: Option<String>,
    },
    /// Opens a single-use invitation, replacing any open one.
    StartPairing,
    ConfirmPairing {
        endpoint_id: EndpointKey,
        preset: DevicePreset,
    },
    CancelPairing,
    ListDevices,
    /// Removes the device and closes its connections. `reset_tls` also rotates the TLS
    /// secret, which a revoked device may have imported separately.
    RevokeDevice {
        endpoint_id: EndpointKey,
        reset_tls: bool,
    },
    /// Uses a paid-relay account for the Host's relay ticket, replacing the
    /// plain relay URL while set; `None` forgets the account.
    SetRelayAccount { account: Option<RelayAccount> },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RemoteAccessResponse {
    Status(RemoteAccessStatus),
    ConnectionInfo(RemoteConnectionInfo),
    /// Carries the invitation secret; shown on screen only.
    PairingInvitation {
        code: SecretText,
        expires_millis: u64,
    },
    Devices(Vec<PairedDevice>),
}
