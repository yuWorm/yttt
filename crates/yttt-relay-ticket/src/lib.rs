//! Relay tickets: what the control plane signs and a relay node checks offline.
//!
//! A ticket only buys transport. It names one account, one EndpointId and one
//! node, and carries the bandwidth the account paid for. It never says anything
//! about which Host a device may reach; that stays end-to-end between devices.
//!
//! A ticket is checked when a relay connection is admitted, and is long-lived:
//! iroh keeps the token a relay connection started with for every reconnect,
//! so a short-lived ticket would lock a long-running endpoint out. Live control
//! (rate changes, suspension, removed devices) travels instead over the
//! node's usage reports, see [`api`].

pub mod api;

use ed25519_dalek::{Signer as _, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};

/// Longest token a relay accepts before decoding it.
pub const MAX_TICKET_TOKEN_BYTES: usize = 1024;
/// How far a device proof's timestamp may drift from the control plane's clock.
pub const DEVICE_PROOF_SKEW_SECS: u64 = 300;

const TICKET_LABEL: &[u8] = b"yttt-relay-ticket-v1";
const DEVICE_PROOF_LABEL: &[u8] = b"yttt-device-proof-v1";

/// The claims inside a ticket.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ticket {
    pub account_id: u64,
    #[serde(with = "serde_bytes")]
    pub endpoint_id: [u8; 32],
    pub node_id: String,
    /// Sustained upstream rate for the whole account on this node.
    pub rate_bps: u32,
    /// Bytes the account may send in a burst above the sustained rate.
    pub burst_bytes: u32,
    /// Lets a node refuse tickets issued before the device was removed.
    pub issued_unix: u64,
    pub expires_unix: u64,
}

#[derive(Serialize, Deserialize)]
struct SignedTicket {
    #[serde(with = "serde_bytes")]
    body: Vec<u8>,
    #[serde(with = "serde_bytes")]
    signature: [u8; 64],
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum TicketError {
    #[error("the ticket is malformed")]
    Malformed,
    #[error("the ticket was not signed by a trusted key")]
    BadSignature,
    #[error("the ticket has expired")]
    Expired,
    #[error("the ticket is for another relay node")]
    WrongNode,
    #[error("the ticket is for another endpoint")]
    WrongEndpoint,
}

/// Signs tickets; held only by the control plane.
pub struct TicketSigner {
    key: SigningKey,
}

impl TicketSigner {
    pub fn from_secret(secret: [u8; 32]) -> Self {
        Self {
            key: SigningKey::from_bytes(&secret),
        }
    }

    pub fn public_key(&self) -> [u8; 32] {
        self.key.verifying_key().to_bytes()
    }

    /// Returns the bearer token a device presents to the relay.
    pub fn issue(&self, ticket: &Ticket) -> String {
        let body = cbor(ticket);
        let signature = self.key.sign(&signed_bytes(&body)).to_bytes();
        data_encoding::BASE64URL_NOPAD.encode(&cbor(&SignedTicket { body, signature }))
    }
}

impl std::fmt::Debug for TicketSigner {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TicketSigner")
            .field("public_key", &hex(&self.public_key()))
            .finish()
    }
}

/// Checks tickets on a relay node. Holds the current and the previous control
/// plane key so a key rotation never strands live devices.
#[derive(Clone, Debug)]
pub struct TicketVerifier {
    keys: Vec<VerifyingKey>,
}

impl TicketVerifier {
    pub fn new(keys: &[[u8; 32]]) -> Result<Self, TicketError> {
        let keys = keys
            .iter()
            .map(|key| VerifyingKey::from_bytes(key).map_err(|_| TicketError::Malformed))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self { keys })
    }

    /// Accepts `token` only if a trusted key signed it, it is for `node_id`, it
    /// is for the endpoint the relay handshake proved, and it has not expired.
    pub fn verify(
        &self,
        token: &str,
        endpoint_id: &[u8; 32],
        node_id: &str,
        now_unix: u64,
    ) -> Result<Ticket, TicketError> {
        if token.len() > MAX_TICKET_TOKEN_BYTES {
            return Err(TicketError::Malformed);
        }
        let bytes = data_encoding::BASE64URL_NOPAD
            .decode(token.as_bytes())
            .map_err(|_| TicketError::Malformed)?;
        let signed: SignedTicket =
            ciborium::from_reader(bytes.as_slice()).map_err(|_| TicketError::Malformed)?;
        let signature = ed25519_dalek::Signature::from_bytes(&signed.signature);
        let payload = signed_bytes(&signed.body);
        if !self
            .keys
            .iter()
            .any(|key| key.verify_strict(&payload, &signature).is_ok())
        {
            return Err(TicketError::BadSignature);
        }
        let ticket: Ticket =
            ciborium::from_reader(signed.body.as_slice()).map_err(|_| TicketError::Malformed)?;
        if ticket.node_id != node_id {
            return Err(TicketError::WrongNode);
        }
        if &ticket.endpoint_id != endpoint_id {
            return Err(TicketError::WrongEndpoint);
        }
        if ticket.expires_unix <= now_unix {
            return Err(TicketError::Expired);
        }
        Ok(ticket)
    }
}

/// What a device proof authorizes, so a proof for one request cannot be
/// replayed as another.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProofPurpose {
    RegisterDevice,
    IssueTicket,
}

impl ProofPurpose {
    fn label(self) -> &'static [u8] {
        match self {
            Self::RegisterDevice => b"register-device",
            Self::IssueTicket => b"issue-ticket",
        }
    }
}

/// Proves to the control plane that the caller holds the device key it names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeviceProof {
    pub endpoint_id: [u8; 32],
    pub issued_unix: u64,
    pub signature: [u8; 64],
}

impl DeviceProof {
    pub fn sign(
        device_secret: &[u8; 32],
        purpose: ProofPurpose,
        account_id: u64,
        issued_unix: u64,
    ) -> Self {
        let key = SigningKey::from_bytes(device_secret);
        let endpoint_id = key.verifying_key().to_bytes();
        let message = device_proof_message(purpose, account_id, &endpoint_id, issued_unix);
        Self {
            endpoint_id,
            issued_unix,
            signature: key.sign(&message).to_bytes(),
        }
    }

    pub fn verify(
        &self,
        purpose: ProofPurpose,
        account_id: u64,
        now_unix: u64,
    ) -> Result<(), TicketError> {
        if self.issued_unix.abs_diff(now_unix) > DEVICE_PROOF_SKEW_SECS {
            return Err(TicketError::Expired);
        }
        let key =
            VerifyingKey::from_bytes(&self.endpoint_id).map_err(|_| TicketError::Malformed)?;
        let message =
            device_proof_message(purpose, account_id, &self.endpoint_id, self.issued_unix);
        key.verify_strict(
            &message,
            &ed25519_dalek::Signature::from_bytes(&self.signature),
        )
        .map_err(|_| TicketError::BadSignature)
    }
}

fn device_proof_message(
    purpose: ProofPurpose,
    account_id: u64,
    endpoint_id: &[u8; 32],
    issued_unix: u64,
) -> Vec<u8> {
    let label = purpose.label();
    let mut message = Vec::with_capacity(DEVICE_PROOF_LABEL.len() + label.len() + 50);
    message.extend_from_slice(DEVICE_PROOF_LABEL);
    message.push(label.len() as u8);
    message.extend_from_slice(label);
    message.extend_from_slice(&account_id.to_be_bytes());
    message.extend_from_slice(endpoint_id);
    message.extend_from_slice(&issued_unix.to_be_bytes());
    message
}

fn signed_bytes(body: &[u8]) -> Vec<u8> {
    let mut payload = Vec::with_capacity(TICKET_LABEL.len() + body.len());
    payload.extend_from_slice(TICKET_LABEL);
    payload.extend_from_slice(body);
    payload
}

fn cbor(value: &impl Serialize) -> Vec<u8> {
    let mut bytes = Vec::new();
    ciborium::into_writer(value, &mut bytes).expect("CBOR encoding into memory cannot fail");
    bytes
}

/// Lowercase hex, as keys appear in configuration files and the HTTP API.
pub fn hex(bytes: &[u8]) -> String {
    data_encoding::HEXLOWER.encode(bytes)
}

/// Parses a 32-byte key written as hex.
pub fn parse_key(text: &str) -> Option<[u8; 32]> {
    data_encoding::HEXLOWER_PERMISSIVE
        .decode(text.trim().as_bytes())
        .ok()?
        .try_into()
        .ok()
}

/// Parses a 64-byte signature written as hex.
pub fn parse_signature(text: &str) -> Option<[u8; 64]> {
    data_encoding::HEXLOWER_PERMISSIVE
        .decode(text.trim().as_bytes())
        .ok()?
        .try_into()
        .ok()
}

/// The public key of an Ed25519 secret, i.e. an iroh EndpointId.
pub fn public_key(secret: &[u8; 32]) -> [u8; 32] {
    SigningKey::from_bytes(secret).verifying_key().to_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_800_000_000;

    fn ticket(endpoint_id: [u8; 32]) -> Ticket {
        Ticket {
            account_id: 7,
            endpoint_id,
            node_id: "tokyo-1".into(),
            rate_bps: 1_000_000,
            burst_bytes: 250_000,
            issued_unix: NOW,
            expires_unix: NOW + 600,
        }
    }

    #[test]
    fn a_signed_ticket_verifies_for_its_endpoint_and_node() {
        let signer = TicketSigner::from_secret([1; 32]);
        let endpoint = public_key(&[9; 32]);
        let token = signer.issue(&ticket(endpoint));
        let verifier = TicketVerifier::new(&[signer.public_key()]).unwrap();
        assert_eq!(
            verifier.verify(&token, &endpoint, "tokyo-1", NOW),
            Ok(ticket(endpoint))
        );
    }

    #[test]
    fn a_ticket_is_refused_off_its_endpoint_node_or_lifetime() {
        let signer = TicketSigner::from_secret([1; 32]);
        let endpoint = public_key(&[9; 32]);
        let token = signer.issue(&ticket(endpoint));
        let verifier = TicketVerifier::new(&[signer.public_key()]).unwrap();
        assert_eq!(
            verifier.verify(&token, &public_key(&[8; 32]), "tokyo-1", NOW),
            Err(TicketError::WrongEndpoint)
        );
        assert_eq!(
            verifier.verify(&token, &endpoint, "osaka-1", NOW),
            Err(TicketError::WrongNode)
        );
        assert_eq!(
            verifier.verify(&token, &endpoint, "tokyo-1", NOW + 600),
            Err(TicketError::Expired)
        );
    }

    #[test]
    fn a_ticket_from_an_untrusted_key_or_with_any_changed_byte_is_refused() {
        let signer = TicketSigner::from_secret([1; 32]);
        let endpoint = public_key(&[9; 32]);
        let token = signer.issue(&ticket(endpoint));
        let stranger = TicketVerifier::new(&[TicketSigner::from_secret([2; 32]).public_key()]);
        assert_eq!(
            stranger.unwrap().verify(&token, &endpoint, "tokyo-1", NOW),
            Err(TicketError::BadSignature)
        );

        let verifier = TicketVerifier::new(&[signer.public_key()]).unwrap();
        let bytes = data_encoding::BASE64URL_NOPAD
            .decode(token.as_bytes())
            .unwrap();
        for index in 0..bytes.len() {
            let mut tampered = bytes.clone();
            tampered[index] ^= 0x01;
            let tampered = data_encoding::BASE64URL_NOPAD.encode(&tampered);
            assert!(
                verifier
                    .verify(&tampered, &endpoint, "tokyo-1", NOW)
                    .is_err(),
                "byte {index} was not covered"
            );
        }
        assert_eq!(
            verifier.verify(
                &"A".repeat(MAX_TICKET_TOKEN_BYTES + 1),
                &endpoint,
                "tokyo-1",
                NOW
            ),
            Err(TicketError::Malformed)
        );
    }

    #[test]
    fn a_relay_trusting_old_and_new_keys_accepts_tickets_from_both() {
        let old = TicketSigner::from_secret([1; 32]);
        let new = TicketSigner::from_secret([2; 32]);
        let endpoint = public_key(&[9; 32]);
        let verifier = TicketVerifier::new(&[new.public_key(), old.public_key()]).unwrap();
        for signer in [&old, &new] {
            let token = signer.issue(&ticket(endpoint));
            assert!(verifier.verify(&token, &endpoint, "tokyo-1", NOW).is_ok());
        }
    }

    #[test]
    fn a_device_proof_binds_its_key_purpose_account_and_time() {
        let proof = DeviceProof::sign(&[9; 32], ProofPurpose::RegisterDevice, 7, NOW);
        assert_eq!(proof.endpoint_id, public_key(&[9; 32]));
        assert_eq!(
            proof.verify(ProofPurpose::RegisterDevice, 7, NOW + 10),
            Ok(())
        );
        assert_eq!(
            proof.verify(ProofPurpose::IssueTicket, 7, NOW),
            Err(TicketError::BadSignature)
        );
        assert_eq!(
            proof.verify(ProofPurpose::RegisterDevice, 8, NOW),
            Err(TicketError::BadSignature)
        );
        assert_eq!(
            proof.verify(
                ProofPurpose::RegisterDevice,
                7,
                NOW + DEVICE_PROOF_SKEW_SECS + 1
            ),
            Err(TicketError::Expired)
        );
        let claimed = DeviceProof {
            endpoint_id: public_key(&[8; 32]),
            ..proof
        };
        assert_eq!(
            claimed.verify(ProofPurpose::RegisterDevice, 7, NOW),
            Err(TicketError::BadSignature)
        );
    }

    #[test]
    fn keys_round_trip_through_hex() {
        let key = public_key(&[9; 32]);
        assert_eq!(parse_key(&hex(&key)), Some(key));
        assert_eq!(parse_key("abcd"), None);
    }
}
