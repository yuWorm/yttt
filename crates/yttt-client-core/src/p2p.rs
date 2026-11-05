//! Pairing with a Host over P2P, and connecting to it afterwards.
//!
//! The Host key is learned only from the pairing code and trusted only once the
//! Host user has accepted this device; nothing a network peer says later can
//! change it.

use std::{net::SocketAddr, time::Duration};

use hmac::{Hmac, Mac as _};
use rand::RngCore as _;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use yttt_core::model::ids::ProfileId;
use yttt_protocol::{
    EndpointKey,
    p2p::{
        PAIR_ALPN, PAIRING_TTL_MILLIS, PairMac, PairMessage, PairNonce, PairRejectReason,
        PairTranscript, PairingCode, WORK_ALPN, commit_host_nonce,
    },
};
use yttt_transport::{Credential, DeviceKeyCredential, receive_pairing, send_pairing};
use yttt_transport_iroh::{IrohConnector, IrohEndpoint, PeerAddr};

/// How long the Host may take to answer before the user confirms.
const HOST_REPLY_TIMEOUT: Duration = Duration::from_secs(30);

/// What a Client keeps about a Host it paired with. `host` is the trust anchor.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairedHost {
    pub host: EndpointKey,
    pub environment_id: String,
    pub profile_id: ProfileId,
    pub host_label: String,
    /// Hints only: a wrong relay or address makes the Host unreachable, never
    /// another Host trusted.
    pub relay_url: Option<String>,
    pub direct_addrs: Vec<SocketAddr>,
}

impl PairedHost {
    pub fn peer_addr(&self) -> PeerAddr {
        PeerAddr {
            key: self.host,
            relay_url: self.relay_url.clone(),
            direct_addrs: self.direct_addrs.clone(),
        }
    }

    /// Opens work sessions to the paired Host through `endpoint`.
    pub fn connector(&self, endpoint: IrohEndpoint) -> IrohConnector {
        IrohConnector::new(endpoint, self.peer_addr(), WORK_ALPN)
    }

    /// The handshake credential of this device toward the paired Host.
    pub fn credential(&self, device_secret: [u8; 32]) -> Credential {
        Credential::DeviceKey(Box::new(
            DeviceKeyCredential::new(device_secret).with_peer(self.host),
        ))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum PairError {
    #[error("cannot reach the Host: {0}")]
    Transport(String),
    #[error("the Host refused pairing: {0:?}")]
    Rejected(PairRejectReason),
    #[error("the Host broke the pairing protocol: {0}")]
    Protocol(&'static str),
    #[error("the Host is not the one the pairing code names")]
    WrongHost,
    #[error("the Host did not answer in time")]
    TimedOut,
}

/// Pairs this device with the Host named in `code`. `show_short_code` receives the
/// six-digit code to display; the user compares it with the Host's screen while the
/// Host user decides. Returns only after the Host accepted the device.
pub async fn pair(
    endpoint: &IrohEndpoint,
    code: &PairingCode,
    device_label: &str,
    show_short_code: impl FnOnce(u32),
) -> Result<PairedHost, PairError> {
    let peer = PeerAddr {
        key: code.host,
        relay_url: code.relay_url.clone(),
        direct_addrs: code.direct_addrs.clone(),
    };
    let transport = |error: yttt_transport::TransportError| PairError::Transport(error.to_string());
    let connection = endpoint
        .connect(&peer, PAIR_ALPN)
        .await
        .map_err(transport)?;
    // The Host opens the pairing stream and speaks first.
    let mut stream = tokio::time::timeout(HOST_REPLY_TIMEOUT, connection.accept_stream())
        .await
        .map_err(|_| PairError::TimedOut)?
        .ok_or(PairError::Protocol(
            "the Host closed the pairing connection",
        ))?;
    let commit = match receive(&mut stream, HOST_REPLY_TIMEOUT).await? {
        PairMessage::Hello { host_nonce_commit } => host_nonce_commit,
        PairMessage::Rejected(reason) => return Err(PairError::Rejected(reason)),
        _ => return Err(PairError::Protocol("expected Hello")),
    };
    let mut client_nonce = PairNonce([0; 32]);
    rand::rngs::OsRng.fill_bytes(&mut client_nonce.0);
    let client = endpoint.id();
    let transcript = PairTranscript {
        invitation_id: &code.invitation_id,
        host: &code.host,
        client: &client,
        environment_id: &code.environment_id,
        profile_id: &code.profile_id,
        host_nonce_commit: &commit,
        client_nonce: &client_nonce,
        device_label,
    };
    let mut mac = Hmac::<Sha256>::new_from_slice(&code.secret).expect("HMAC accepts 32-byte keys");
    mac.update(&transcript.request_bytes());
    let request = PairMessage::Request {
        invitation_id: code.invitation_id,
        device_label: device_label.to_string(),
        client_nonce,
        mac: PairMac(mac.finalize().into_bytes().into()),
    };
    send_pairing(&mut stream, &request)
        .await
        .map_err(|error| PairError::Transport(error.to_string()))?;
    let host_nonce = match receive(&mut stream, HOST_REPLY_TIMEOUT).await? {
        PairMessage::Reveal { host_nonce } => host_nonce,
        PairMessage::Rejected(reason) => return Err(PairError::Rejected(reason)),
        _ => return Err(PairError::Protocol("expected Reveal")),
    };
    if commit_host_nonce(&host_nonce) != commit {
        return Err(PairError::Protocol(
            "the Host nonce does not match its commitment",
        ));
    }
    show_short_code(transcript.short_code(&host_nonce));
    let decision = receive(&mut stream, Duration::from_millis(PAIRING_TTL_MILLIS)).await?;
    connection.close();
    match decision {
        PairMessage::Accepted {
            environment_id,
            profile_id,
            host_label,
        } => {
            if environment_id != code.environment_id || profile_id != code.profile_id {
                return Err(PairError::WrongHost);
            }
            Ok(PairedHost {
                host: code.host,
                environment_id,
                profile_id,
                host_label,
                relay_url: code.relay_url.clone(),
                direct_addrs: code.direct_addrs.clone(),
            })
        }
        PairMessage::Rejected(reason) => Err(PairError::Rejected(reason)),
        _ => Err(PairError::Protocol("expected Accepted or Rejected")),
    }
}

async fn receive(
    stream: &mut yttt_transport::TransportStream,
    timeout: Duration,
) -> Result<PairMessage, PairError> {
    tokio::time::timeout(timeout, receive_pairing(stream))
        .await
        .map_err(|_| PairError::TimedOut)?
        .map_err(|error| PairError::Transport(error.to_string()))
}
