use std::fmt;

use serde::{Deserialize, Serialize};
use yttt_core::model::ids::{ClientInstanceId, HostId, ProfileId};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProtocolRange {
    pub minimum: u16,
    pub maximum: u16,
}

impl ProtocolRange {
    pub const fn exact(version: u16) -> Self {
        Self {
            minimum: version,
            maximum: version,
        }
    }

    pub fn negotiate(self, other: Self) -> Option<u16> {
        let minimum = self.minimum.max(other.minimum);
        let maximum = self.maximum.min(other.maximum);
        (minimum <= maximum).then_some(maximum)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Nonce(pub [u8; 32]);

impl fmt::Debug for Nonce {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Nonce([redacted])")
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthMac(pub [u8; 32]);

impl fmt::Debug for AuthMac {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AuthMac([redacted])")
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientHello {
    pub supported: ProtocolRange,
    pub build_id: String,
    pub profile_id: ProfileId,
    pub client_instance_id: ClientInstanceId,
    pub host_epoch_hint: Option<u64>,
    pub nonce: Nonce,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostChallenge {
    pub selected_version: u16,
    pub build_id: String,
    pub profile_id: ProfileId,
    pub host_id: HostId,
    pub host_epoch: u64,
    pub client_nonce: Nonce,
    pub host_nonce: Nonce,
    pub proof: AuthMac,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientAuthenticate {
    pub client_instance_id: ClientInstanceId,
    pub host_epoch: u64,
    pub client_nonce: Nonce,
    pub host_nonce: Nonce,
    pub proof: AuthMac,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostReady {
    pub selected_version: u16,
    pub host_id: HostId,
    pub host_epoch: u64,
    pub connection_sequence: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RejectReason {
    VersionMismatch { supported: ProtocolRange },
    BuildMismatch,
    ProfileMismatch,
    AuthenticationFailed,
    StaleHostEpoch,
    AlreadyConnected,
    InvalidMessage,
    HostShuttingDown,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum HandshakeMessage {
    ClientHello(ClientHello),
    HostChallenge(HostChallenge),
    ClientAuthenticate(ClientAuthenticate),
    HostReady(HostReady),
    Rejected(RejectReason),
}
