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

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct BuildIdentity {
    pub product_version: String,
    pub build_fingerprint: String,
    pub resource_compatibility: String,
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

/// An Ed25519 public key; on the P2P ingress it is also the peer's iroh EndpointId.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct EndpointKey(#[serde(with = "serde_bytes")] pub [u8; 32]);

impl fmt::Debug for EndpointKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "EndpointKey({})", self)
    }
}

impl fmt::Display for EndpointKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

impl std::str::FromStr for EndpointKey {
    type Err = InvalidEndpointKey;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        if text.len() != 64 || !text.is_ascii() {
            return Err(InvalidEndpointKey);
        }
        let mut bytes = [0_u8; 32];
        for (index, byte) in bytes.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&text[index * 2..index * 2 + 2], 16)
                .map_err(|_| InvalidEndpointKey)?;
        }
        Ok(Self(bytes))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("endpoint key must be 64 hexadecimal characters")]
pub struct InvalidEndpointKey;

/// An Ed25519 signature over a handshake transcript.
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Signature(#[serde(with = "serde_bytes")] pub [u8; 64]);

impl fmt::Debug for Signature {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Signature(..)")
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientHello {
    pub supported: ProtocolRange,
    pub build: BuildIdentity,
    pub profile_id: ProfileId,
    pub client_instance_id: ClientInstanceId,
    pub host_epoch_hint: Option<u64>,
    pub session_nonce: Nonce,
    pub credential_generation: u64,
    #[serde(default)]
    pub can_force_stop: bool,
    pub nonce: Nonce,
    #[serde(default)]
    pub channel: ConnectionChannel,
    /// The Client's device key; set only on the P2P ingress.
    #[serde(default)]
    pub endpoint_id: Option<EndpointKey>,
    /// The Client reads `ControlMessage::Compressed`; a Host may then send its
    /// large messages compressed.
    #[serde(default)]
    pub accepts_compression: bool,
}

/// One authenticated Client session uses one `Session` connection for requests,
/// terminal input, state events, and terminal frames.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConnectionChannel {
    #[default]
    Session,
    Lifecycle,
    DesktopOwner,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostChallenge {
    pub selected_version: u16,
    pub build: BuildIdentity,
    pub profile_id: ProfileId,
    pub environment_id: String,
    pub credential_generation: u64,
    pub host_id: HostId,
    pub host_epoch: u64,
    pub client_nonce: Nonce,
    pub host_nonce: Nonce,
    /// All zero when the Host proves itself with `signature` instead.
    pub proof: AuthMac,
    /// The Host's device key; set only on the P2P ingress.
    #[serde(default)]
    pub endpoint_id: Option<EndpointKey>,
    #[serde(default)]
    pub signature: Option<Signature>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientAuthenticate {
    pub client_instance_id: ClientInstanceId,
    pub host_epoch: u64,
    pub client_nonce: Nonce,
    pub host_nonce: Nonce,
    /// All zero when the Client proves itself with `signature` instead.
    pub proof: AuthMac,
    #[serde(default)]
    pub signature: Option<Signature>,
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
    ProfileMismatch,
    AuthenticationFailed,
    StaleHostEpoch,
    AlreadyConnected,
    InvalidMessage,
    HostShuttingDown,
    PermissionDenied,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum HandshakeMessage {
    ClientHello(ClientHello),
    HostChallenge(HostChallenge),
    ClientAuthenticate(ClientAuthenticate),
    HostReady(HostReady),
    Rejected(RejectReason),
}
