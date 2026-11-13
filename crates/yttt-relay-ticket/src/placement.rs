//! Where a Host's devices find it: the node and relay address the Host chose,
//! signed with the Host's own key.
//!
//! The control plane stores and hands out these records but cannot make one:
//! a Client checks each against the Host key it pinned when pairing, and takes
//! only a record with a higher `seq` than the one it has. So a Client relays
//! only through a node the Host chose.

use ed25519_dalek::{Signer as _, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};

/// Longest node id a placement carries.
pub const MAX_PLACEMENT_NODE_ID_BYTES: usize = 64;
/// Longest relay URL a placement carries.
pub const MAX_PLACEMENT_RELAY_URL_BYTES: usize = 256;

const PLACEMENT_LABEL: &[u8] = b"yttt host placement v1\0";

/// The node a Host relays through, and the address its devices dial.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostPlacement {
    /// The Host's endpoint key, which Clients pinned when they paired.
    #[serde(with = "serde_bytes")]
    pub host: [u8; 32],
    /// Grows with every placement the Host signs; never reused.
    pub seq: u64,
    pub node_id: String,
    pub relay_url: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedPlacement {
    pub placement: HostPlacement,
    #[serde(with = "serde_bytes")]
    pub signature: [u8; 64],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum PlacementError {
    #[error("the placement is not for this Host")]
    WrongHost,
    #[error("the placement's signature does not verify")]
    BadSignature,
    #[error("the placement's node id or relay URL is too long")]
    TooLong,
    #[error("two placements share a sequence number but differ")]
    Conflict,
}

impl HostPlacement {
    /// The bytes the Host signs; a fixed layout, independent of any encoding.
    pub fn signed_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(
            PLACEMENT_LABEL.len() + 32 + 8 + 4 + self.node_id.len() + self.relay_url.len(),
        );
        bytes.extend_from_slice(PLACEMENT_LABEL);
        bytes.extend_from_slice(&self.host);
        bytes.extend_from_slice(&self.seq.to_be_bytes());
        for text in [&self.node_id, &self.relay_url] {
            // Lengths are checked before signing and verifying.
            bytes.extend_from_slice(&(text.len() as u16).to_be_bytes());
            bytes.extend_from_slice(text.as_bytes());
        }
        bytes
    }

    fn check_lengths(&self) -> Result<(), PlacementError> {
        if self.node_id.len() > MAX_PLACEMENT_NODE_ID_BYTES
            || self.relay_url.len() > MAX_PLACEMENT_RELAY_URL_BYTES
        {
            return Err(PlacementError::TooLong);
        }
        Ok(())
    }
}

impl SignedPlacement {
    /// Signs `node_id` and `relay_url` as placement `seq` of the Host whose
    /// secret key is `host_secret`.
    pub fn sign(
        host_secret: &[u8; 32],
        seq: u64,
        node_id: impl Into<String>,
        relay_url: impl Into<String>,
    ) -> Result<Self, PlacementError> {
        let key = SigningKey::from_bytes(host_secret);
        let placement = HostPlacement {
            host: key.verifying_key().to_bytes(),
            seq,
            node_id: node_id.into(),
            relay_url: relay_url.into(),
        };
        placement.check_lengths()?;
        let signature = key.sign(&placement.signed_bytes()).to_bytes();
        Ok(Self {
            placement,
            signature,
        })
    }

    /// The placement, if `expected_host` signed it.
    pub fn verify(&self, expected_host: &[u8; 32]) -> Result<&HostPlacement, PlacementError> {
        if &self.placement.host != expected_host {
            return Err(PlacementError::WrongHost);
        }
        self.placement.check_lengths()?;
        let key = VerifyingKey::from_bytes(expected_host).map_err(|_| PlacementError::WrongHost)?;
        key.verify_strict(
            &self.placement.signed_bytes(),
            &ed25519_dalek::Signature::from_bytes(&self.signature),
        )
        .map_err(|_| PlacementError::BadSignature)?;
        Ok(&self.placement)
    }

    /// Whether this placement replaces `current`. Call it on placements that
    /// verified for the same Host; equal sequence numbers must carry equal
    /// placements.
    pub fn newer_than(&self, current: Option<&SignedPlacement>) -> Result<bool, PlacementError> {
        let Some(current) = current else {
            return Ok(true);
        };
        match self.placement.seq.cmp(&current.placement.seq) {
            std::cmp::Ordering::Greater => Ok(true),
            std::cmp::Ordering::Less => Ok(false),
            std::cmp::Ordering::Equal if self.placement == current.placement => Ok(false),
            std::cmp::Ordering::Equal => Err(PlacementError::Conflict),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOST: [u8; 32] = [7; 32];
    const OTHER: [u8; 32] = [8; 32];

    fn host_key() -> [u8; 32] {
        crate::public_key(&HOST)
    }

    fn signed(seq: u64, node: &str) -> SignedPlacement {
        SignedPlacement::sign(&HOST, seq, node, format!("https://{node}.example")).unwrap()
    }

    #[test]
    fn the_signed_layout_is_fixed() {
        let placement = HostPlacement {
            host: [1; 32],
            seq: 0x0102,
            node_id: "n".into(),
            relay_url: "u".into(),
        };
        assert_eq!(
            crate::hex(&placement.signed_bytes()),
            format!(
                "{}{}{}{}{}",
                crate::hex(b"yttt host placement v1\0"),
                "01".repeat(32),
                "0000000000000102",
                "00016e",
                "000175"
            )
        );
    }

    #[test]
    fn a_placement_verifies_for_its_host_only() {
        let placement = signed(1, "tokyo-1");
        assert_eq!(placement.verify(&host_key()).unwrap().node_id, "tokyo-1");
        assert_eq!(
            placement.verify(&crate::public_key(&OTHER)),
            Err(PlacementError::WrongHost)
        );
    }

    #[test]
    fn a_changed_placement_does_not_verify() {
        let mut placement = signed(1, "tokyo-1");
        placement.placement.node_id = "evil-1".into();
        assert_eq!(
            placement.verify(&host_key()),
            Err(PlacementError::BadSignature)
        );
        let mut placement = signed(1, "tokyo-1");
        placement.placement.seq = 9;
        assert_eq!(
            placement.verify(&host_key()),
            Err(PlacementError::BadSignature)
        );
    }

    #[test]
    fn long_fields_are_refused() {
        assert_eq!(
            SignedPlacement::sign(&HOST, 1, "n".repeat(65), "u"),
            Err(PlacementError::TooLong)
        );
        assert_eq!(
            SignedPlacement::sign(&HOST, 1, "n", "u".repeat(257)),
            Err(PlacementError::TooLong)
        );
        let mut placement = signed(1, "tokyo-1");
        placement.placement.relay_url = "u".repeat(257);
        assert_eq!(placement.verify(&host_key()), Err(PlacementError::TooLong));
    }

    #[test]
    fn a_higher_seq_replaces_and_a_conflict_is_refused() {
        let first = signed(1, "tokyo-1");
        let second = signed(2, "osaka-1");
        assert_eq!(first.newer_than(None), Ok(true));
        assert_eq!(second.newer_than(Some(&first)), Ok(true));
        assert_eq!(first.newer_than(Some(&second)), Ok(false));
        assert_eq!(first.newer_than(Some(&first.clone())), Ok(false));
        assert_eq!(
            signed(2, "tokyo-1").newer_than(Some(&second)),
            Err(PlacementError::Conflict)
        );
    }

    #[test]
    fn it_survives_cbor_and_json() {
        let placement = signed(3, "tokyo-1");
        let mut cbor = Vec::new();
        ciborium::into_writer(&placement, &mut cbor).unwrap();
        let back: SignedPlacement = ciborium::from_reader(cbor.as_slice()).unwrap();
        assert_eq!(back, placement);
        let json = serde_json::to_string(&placement).unwrap();
        let back: SignedPlacement = serde_json::from_str(&json).unwrap();
        assert_eq!(back, placement);
        back.verify(&host_key()).unwrap();
    }
}
