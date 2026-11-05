//! Device pairing and the paired-device model of the P2P ingress.
//!
//! A Client pairs once by reading a [`PairingCode`] from the Host's screen, then
//! connects by its device key alone. Pairing messages travel on their own ALPN and
//! never share the session handshake.

use std::{fmt, net::SocketAddr};

use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use yttt_core::model::ids::ProfileId;
use zeroize::Zeroize;

use crate::{Capability, EndpointKey};

/// The ALPN of authenticated work sessions.
pub const WORK_ALPN: &[u8] = b"yttt/work/1";
/// The ALPN of pairing, accepted only while an invitation is open.
pub const PAIR_ALPN: &[u8] = b"yttt/pair/1";
/// Upper case so a QR code can use its compact alphanumeric mode.
pub const PAIRING_CODE_PREFIX: &str = "YTTT-PAIR:1:";
pub const MAX_PAIRING_CODE_BYTES: usize = 1024;
pub const MAX_DIRECT_ADDRS: usize = 8;
pub const MAX_RELAY_URL_BYTES: usize = 256;
pub const MAX_DEVICE_LABEL_BYTES: usize = 64;
pub const MAX_PAIRED_DEVICES: usize = 32;
/// How long an invitation stays open.
pub const PAIRING_TTL_MILLIS: u64 = 5 * 60 * 1000;
/// Invalid pairing attempts an invitation tolerates before it closes.
pub const MAX_PAIRING_FAILURES: u32 = 3;
/// Encoded pairing messages are small; anything larger is refused.
pub const MAX_PAIR_MESSAGE_BYTES: usize = 4 * 1024;

/// What a paired device may do. Both presets exclude device administration, which
/// only local IPC reaches.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DevicePreset {
    /// Everything a TLS work session may do.
    Full,
    /// Reads projects, Git and workspaces, and watches terminals without typing.
    Observe,
}

impl DevicePreset {
    /// Whether a request needing `capability` is allowed; `None` needs none.
    pub fn allows(self, capability: Option<Capability>) -> bool {
        match self {
            Self::Full => true,
            Self::Observe => matches!(
                capability,
                None | Some(
                    Capability::ProjectRead | Capability::GitRead | Capability::WorkspaceRead
                )
            ),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairedDevice {
    pub endpoint_id: EndpointKey,
    pub label: String,
    pub preset: DevicePreset,
    pub paired_at_millis: u64,
    #[serde(default)]
    pub last_seen_millis: Option<u64>,
}

/// What a Host shows to invite one device. It carries a one-time secret, so it is
/// displayed, never written to the clipboard, argv or a URL handler.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairingCode {
    pub host: EndpointKey,
    #[serde(default)]
    pub relay_url: Option<String>,
    #[serde(default)]
    pub direct_addrs: Vec<SocketAddr>,
    #[serde(with = "serde_bytes")]
    pub invitation_id: [u8; 16],
    #[serde(with = "serde_bytes")]
    pub secret: [u8; 32],
    pub environment_id: String,
    pub profile_id: ProfileId,
}

impl fmt::Debug for PairingCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PairingCode")
            .field("host", &self.host)
            .field("relay_url", &self.relay_url)
            .field("direct_addrs", &self.direct_addrs)
            .field("environment_id", &self.environment_id)
            .field("profile_id", &self.profile_id)
            .finish_non_exhaustive()
    }
}

impl Drop for PairingCode {
    fn drop(&mut self) {
        self.secret.zeroize();
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum PairingCodeError {
    #[error("not a yttt pairing code")]
    NotAPairingCode,
    #[error("pairing code is too long")]
    TooLong,
    #[error("pairing code is malformed")]
    Malformed,
}

impl PairingCode {
    pub fn encode(&self) -> String {
        let mut payload = Vec::new();
        ciborium::into_writer(self, &mut payload).expect("pairing code encodes");
        let text = format!(
            "{PAIRING_CODE_PREFIX}{}",
            data_encoding::BASE32_NOPAD.encode(&payload)
        );
        payload.zeroize();
        text
    }

    pub fn decode(text: &str) -> Result<Self, PairingCodeError> {
        let text = text.trim();
        if text.len() > MAX_PAIRING_CODE_BYTES {
            return Err(PairingCodeError::TooLong);
        }
        let body = text
            .get(..PAIRING_CODE_PREFIX.len())
            .filter(|prefix| prefix.eq_ignore_ascii_case(PAIRING_CODE_PREFIX))
            .map(|_| &text[PAIRING_CODE_PREFIX.len()..])
            .ok_or(PairingCodeError::NotAPairingCode)?;
        let mut payload = data_encoding::BASE32_NOPAD
            .decode(body.to_ascii_uppercase().as_bytes())
            .map_err(|_| PairingCodeError::Malformed)?;
        let code: Result<Self, _> = ciborium::from_reader(payload.as_slice());
        payload.zeroize();
        let code = code.map_err(|_| PairingCodeError::Malformed)?;
        let valid = code.direct_addrs.len() <= MAX_DIRECT_ADDRS
            && code
                .relay_url
                .as_ref()
                .is_none_or(|url| url.len() <= MAX_RELAY_URL_BYTES)
            && !code.environment_id.is_empty();
        valid.then_some(code).ok_or(PairingCodeError::Malformed)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairNonce(#[serde(with = "serde_bytes")] pub [u8; 32]);

impl fmt::Debug for PairNonce {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PairNonce(..)")
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairMac(#[serde(with = "serde_bytes")] pub [u8; 32]);

impl fmt::Debug for PairMac {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PairMac(..)")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PairRejectReason {
    /// The invitation is unknown, used, expired or was closed by the Host user.
    InvitationClosed,
    AuthenticationFailed,
    /// The Host user did not confirm the short code.
    Declined,
    InvalidMessage,
    DeviceLimitReached,
}

/// Pairing runs Host `Hello`, Client `Request`, Host `Reveal`, then the Host user
/// compares the short code and the Host sends `Accepted` or `Rejected`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PairMessage {
    /// Commits to the Host nonce before seeing the Client's, so neither side can pick
    /// its nonce to steer the short code.
    Hello {
        #[serde(with = "serde_bytes")]
        host_nonce_commit: [u8; 32],
    },
    Request {
        #[serde(with = "serde_bytes")]
        invitation_id: [u8; 16],
        device_label: String,
        client_nonce: PairNonce,
        mac: PairMac,
    },
    Reveal {
        host_nonce: PairNonce,
    },
    Accepted {
        environment_id: String,
        profile_id: ProfileId,
        host_label: String,
    },
    Rejected(PairRejectReason),
}

/// The values both sides bind into the request MAC and the short code.
pub struct PairTranscript<'a> {
    pub invitation_id: &'a [u8; 16],
    pub host: &'a EndpointKey,
    pub client: &'a EndpointKey,
    pub environment_id: &'a str,
    pub profile_id: &'a ProfileId,
    pub host_nonce_commit: &'a [u8; 32],
    pub client_nonce: &'a PairNonce,
    pub device_label: &'a str,
}

impl PairTranscript<'_> {
    /// The bytes the Client authenticates with the invitation secret.
    pub fn request_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(256);
        push_field(&mut bytes, b"yttt-pair-request-v1");
        push_field(&mut bytes, self.invitation_id);
        push_field(&mut bytes, &self.host.0);
        push_field(&mut bytes, &self.client.0);
        push_field(&mut bytes, self.environment_id.as_bytes());
        push_field(&mut bytes, self.profile_id.as_str().as_bytes());
        push_field(&mut bytes, self.host_nonce_commit);
        push_field(&mut bytes, &self.client_nonce.0);
        push_field(&mut bytes, self.device_label.as_bytes());
        bytes
    }

    /// The six-digit code both screens show once the Host nonce is revealed.
    pub fn short_code(&self, host_nonce: &PairNonce) -> u32 {
        let mut digest = Sha256::new();
        digest.update(b"yttt-pair-sas-v1");
        digest.update(self.request_bytes());
        digest.update(host_nonce.0);
        let hash = digest.finalize();
        u32::from_be_bytes([hash[0], hash[1], hash[2], hash[3]]) % 1_000_000
    }
}

/// The commitment a Host sends in `Hello` for `host_nonce`.
pub fn commit_host_nonce(host_nonce: &PairNonce) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(b"yttt-pair-commit-v1");
    digest.update(host_nonce.0);
    digest.finalize().into()
}

fn push_field(bytes: &mut Vec<u8>, field: &[u8]) {
    bytes.extend_from_slice(&(field.len() as u32).to_be_bytes());
    bytes.extend_from_slice(field);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code() -> PairingCode {
        PairingCode {
            host: EndpointKey([7; 32]),
            relay_url: Some("http://127.0.0.1:3340".into()),
            direct_addrs: vec!["192.0.2.1:4433".parse().unwrap()],
            invitation_id: [1; 16],
            secret: [2; 32],
            environment_id: "env".into(),
            profile_id: ProfileId::new("default"),
        }
    }

    #[test]
    fn pairing_code_round_trips_in_qr_alphanumeric_characters() {
        let text = code().encode();
        assert!(
            text.chars()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || ":-".contains(c))
        );
        assert_eq!(PairingCode::decode(&text).unwrap(), code());
        assert_eq!(
            PairingCode::decode(&text.to_ascii_lowercase()).unwrap(),
            code()
        );
    }

    #[test]
    fn malformed_pairing_codes_are_refused() {
        assert_eq!(
            PairingCode::decode("yttt://pair?x").unwrap_err(),
            PairingCodeError::NotAPairingCode
        );
        assert_eq!(
            PairingCode::decode(&format!("{PAIRING_CODE_PREFIX}!!!")).unwrap_err(),
            PairingCodeError::Malformed
        );
        let long = format!(
            "{PAIRING_CODE_PREFIX}{}",
            "A".repeat(MAX_PAIRING_CODE_BYTES)
        );
        assert_eq!(
            PairingCode::decode(&long).unwrap_err(),
            PairingCodeError::TooLong
        );
        let mut crowded = code();
        crowded.direct_addrs = vec!["192.0.2.1:1".parse().unwrap(); MAX_DIRECT_ADDRS + 1];
        assert_eq!(
            PairingCode::decode(&crowded.encode()).unwrap_err(),
            PairingCodeError::Malformed
        );
    }

    #[test]
    fn observe_allows_only_reading() {
        use Capability::*;
        let reads = [ProjectRead, GitRead, WorkspaceRead];
        for capability in [
            TerminalInteractive,
            ProjectRead,
            ProjectMutate,
            GitRead,
            GitMutate,
            SshConnect,
            RemoteCommandPrivileged,
            CredentialAnswer,
            WorkspaceRead,
            WorkspaceMutate,
        ] {
            assert!(DevicePreset::Full.allows(Some(capability)));
            assert_eq!(
                DevicePreset::Observe.allows(Some(capability)),
                reads.contains(&capability),
                "{capability:?}"
            );
        }
        assert!(DevicePreset::Observe.allows(None));
    }

    #[test]
    fn the_short_code_depends_on_every_transcript_field() {
        let profile = ProfileId::new("default");
        let commit = commit_host_nonce(&PairNonce([3; 32]));
        let base = PairTranscript {
            invitation_id: &[1; 16],
            host: &EndpointKey([7; 32]),
            client: &EndpointKey([8; 32]),
            environment_id: "env",
            profile_id: &profile,
            host_nonce_commit: &commit,
            client_nonce: &PairNonce([4; 32]),
            device_label: "phone",
        };
        let reference = base.short_code(&PairNonce([3; 32]));
        assert!(reference < 1_000_000);
        let other_client = PairTranscript {
            client: &EndpointKey([9; 32]),
            ..base
        };
        assert_ne!(other_client.request_bytes(), base.request_bytes());
        let base = PairTranscript {
            client: &EndpointKey([8; 32]),
            ..other_client
        };
        assert_ne!(base.short_code(&PairNonce([5; 32])), reference);
        assert_ne!(commit_host_nonce(&PairNonce([5; 32])), commit);
    }
}
