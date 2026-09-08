use std::{fmt, net::SocketAddr};

use serde::{Deserialize, Serialize};
use yttt_core::model::ids::{ClientInstanceId, ProfileId};
use zeroize::Zeroize;

use crate::session::ControlStatus;

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
}

impl Default for RemoteAccessSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            listen_address: "127.0.0.1:43123".parse().expect("constant socket address"),
            login_startup_consent_granted: false,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RemoteAccessState {
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
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteAccessStatus {
    pub settings: RemoteAccessSettings,
    pub effective: RemoteAccessState,
    pub bound_address: Option<SocketAddr>,
    pub sharing_ready: bool,
    pub error: Option<String>,
    pub clients: Vec<RemoteClientSummary>,
    pub control: ControlStatus,
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
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RemoteAccessResponse {
    Status(RemoteAccessStatus),
    ConnectionInfo(RemoteConnectionInfo),
}
