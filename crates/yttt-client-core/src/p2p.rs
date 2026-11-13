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
use yttt_control_client::{PlacementError, RelayTicket, SignedPlacement};
use yttt_core::model::ids::ProfileId;
use yttt_protocol::{
    EndpointKey,
    p2p::{
        PAIR_ALPN, PAIRING_TTL_MILLIS, PairMac, PairMessage, PairNonce, PairRejectReason,
        PairTranscript, PairingCode, WORK_ALPN, commit_host_nonce,
    },
};
use yttt_transport::{Credential, DeviceKeyCredential, receive_pairing, send_pairing};
use yttt_transport_iroh::{IrohConnector, IrohEndpoint, PeerAddr, RelayTarget};

/// How long the Host may take to answer before the user confirms.
const HOST_REPLY_TIMEOUT: Duration = Duration::from_secs(30);

/// What a Client keeps about a Host it paired with. `host` is the trust anchor.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairedHost {
    pub host: EndpointKey,
    pub environment_id: String,
    pub profile_id: ProfileId,
    pub host_label: String,
    /// The relay the pairing code named. Not dialed: only a placement the
    /// Host signed says where it relays.
    #[serde(default)]
    pub relay_url: Option<String>,
    pub direct_addrs: Vec<SocketAddr>,
    /// Where the Host relays, as it signed it; checked against `host`.
    #[serde(default)]
    pub placement: Option<SignedPlacement>,
    /// The last relay ticket for this device's key, for when the control
    /// plane is out of reach. It only works with this device's key.
    #[serde(default)]
    pub ticket: Option<RelayTicket>,
}

impl PairedHost {
    pub fn peer_addr(&self) -> PeerAddr {
        PeerAddr {
            key: self.host,
            relay_url: self
                .placement
                .as_ref()
                .map(|placement| placement.placement.relay_url.clone()),
            direct_addrs: self.direct_addrs.clone(),
        }
    }

    /// Keeps `signed` if the Host signed it and it is newer than the one
    /// kept; whether it was kept.
    pub fn accept_placement(&mut self, signed: &SignedPlacement) -> Result<bool, PlacementError> {
        signed.verify(&self.host.0)?;
        if !signed.newer_than(self.placement.as_ref())? {
            return Ok(false);
        }
        self.placement = Some(signed.clone());
        Ok(true)
    }

    /// The relay to use: the Host's signed one, with a ticket still usable at
    /// `now_unix` for the same node, `ticket` first and then the cached one.
    pub fn relay_target(
        &self,
        ticket: Option<&RelayTicket>,
        now_unix: u64,
    ) -> Result<RelayTarget, &'static str> {
        let placement = &self
            .placement
            .as_ref()
            .ok_or("the Host has not said where it relays")?
            .placement;
        let mut tickets = ticket.into_iter().chain(self.ticket.as_ref());
        let ticket = tickets
            .find(|ticket| ticket.usable(now_unix) && ticket.node_id == placement.node_id)
            .ok_or("no relay ticket for the node the Host chose")?;
        Ok(RelayTarget {
            url: placement.relay_url.clone(),
            token: Some(ticket.token.clone()),
        })
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

/// Pairs this device with the Host named in `code`, through `relay_url` (the
/// relay the Host signed, if this device knows it) or the code's direct
/// addresses. `show_short_code` receives the six-digit code to display; the
/// user compares it with the Host's screen while the Host user decides.
/// Returns only after the Host accepted the device.
pub async fn pair(
    endpoint: &IrohEndpoint,
    code: &PairingCode,
    relay_url: Option<String>,
    device_label: &str,
    show_short_code: impl FnOnce(u32),
) -> Result<PairedHost, PairError> {
    let peer = PeerAddr {
        key: code.host,
        relay_url,
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
                relay_url: None,
                direct_addrs: code.direct_addrs.clone(),
                placement: None,
                ticket: None,
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

#[cfg(test)]
mod placement_tests {
    use super::*;

    const HOST_SECRET: [u8; 32] = [9; 32];

    fn host() -> PairedHost {
        PairedHost {
            host: yttt_transport_iroh::endpoint_key(&HOST_SECRET),
            environment_id: "environment".into(),
            profile_id: ProfileId::new("default"),
            host_label: "mac".into(),
            relay_url: Some("https://from-the-code.example".into()),
            direct_addrs: Vec::new(),
            placement: None,
            ticket: None,
        }
    }

    fn placement(secret: &[u8; 32], seq: u64, node: &str) -> SignedPlacement {
        SignedPlacement::sign(secret, seq, node, format!("https://{node}.example")).unwrap()
    }

    fn ticket(node: &str, expires_unix: u64) -> RelayTicket {
        RelayTicket {
            relay_url: format!("https://{node}.example"),
            node_id: node.into(),
            token: format!("token-{node}"),
            received_unix: 0,
            expires_unix,
        }
    }

    #[test]
    fn only_a_newer_placement_the_host_signed_is_kept() {
        let mut host = host();
        assert_eq!(
            host.peer_addr().relay_url,
            None,
            "the code's relay is not dialed"
        );
        assert!(
            host.accept_placement(&placement(&HOST_SECRET, 2, "tokyo-1"))
                .unwrap()
        );
        assert_eq!(
            host.peer_addr().relay_url.as_deref(),
            Some("https://tokyo-1.example")
        );
        // Older, the same again, or signed by another key: kept as it was.
        assert!(
            !host
                .accept_placement(&placement(&HOST_SECRET, 1, "evil-1"))
                .unwrap()
        );
        assert!(
            !host
                .accept_placement(&placement(&HOST_SECRET, 2, "tokyo-1"))
                .unwrap()
        );
        assert!(
            host.accept_placement(&placement(&[8; 32], 9, "evil-1"))
                .is_err()
        );
        assert!(
            host.accept_placement(&placement(&HOST_SECRET, 2, "osaka-1"))
                .is_err()
        );
        assert_eq!(host.placement.unwrap().placement.node_id, "tokyo-1");
    }

    #[test]
    fn a_ticket_is_used_only_for_the_node_the_host_signed() {
        let mut host = host();
        assert!(
            host.relay_target(Some(&ticket("tokyo-1", 1_000)), 10)
                .is_err()
        );
        host.accept_placement(&placement(&HOST_SECRET, 1, "tokyo-1"))
            .unwrap();
        // The control plane's ticket for another node is not used.
        assert!(
            host.relay_target(Some(&ticket("evil-1", 1_000)), 10)
                .is_err()
        );
        let target = host
            .relay_target(Some(&ticket("tokyo-1", 1_000)), 10)
            .unwrap();
        assert_eq!(target.url, "https://tokyo-1.example");
        assert_eq!(target.token.as_deref(), Some("token-tokyo-1"));
        // Without a fresh one, the cached ticket serves while it is usable.
        host.ticket = Some(ticket("tokyo-1", 1_000));
        assert!(host.relay_target(None, 10).is_ok());
        assert!(host.relay_target(None, 999).is_err());
    }
}
