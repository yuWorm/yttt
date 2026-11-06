//! Secrets the console stores, such as OAuth client secrets, sealed with a
//! key from the configuration so a copy of the database alone does not
//! reveal them.

use aes_gcm::{
    Aes256Gcm, KeyInit as _, Nonce,
    aead::{Aead as _, Payload},
};
use rand::RngCore as _;
use sha2::Sha256;
use zeroize::Zeroizing;

const NONCE_BYTES: usize = 12;

/// Seals and opens console secrets with a key derived from `secrets_key`.
pub(crate) struct Sealer {
    cipher: Aes256Gcm,
}

/// A sealed secret: the nonce and the ciphertext with its tag.
pub(crate) struct Sealed {
    pub nonce: Vec<u8>,
    pub ciphertext: Vec<u8>,
}

impl Sealer {
    /// `None` when `secrets_key` is not 32 bytes of hex.
    pub(crate) fn new(secrets_key: &str) -> Option<Self> {
        let ikm = Zeroizing::new(yttt_relay_ticket::parse_key(secrets_key)?);
        let mut key = Zeroizing::new([0_u8; 32]);
        hkdf::Hkdf::<Sha256>::new(None, &ikm[..])
            .expand(b"yttt-control console secrets", &mut key[..])
            .expect("32 bytes is a valid HKDF-SHA256 length");
        Some(Self {
            cipher: Aes256Gcm::new_from_slice(&key[..]).expect("a 32-byte key"),
        })
    }

    /// Seals `secret`, bound to `label` (e.g. the provider): a ciphertext
    /// moved to another label does not open.
    pub(crate) fn seal(&self, label: &str, secret: &str) -> Sealed {
        let mut nonce = [0_u8; NONCE_BYTES];
        rand::rngs::OsRng.fill_bytes(&mut nonce);
        let ciphertext = self
            .cipher
            .encrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: secret.as_bytes(),
                    aad: label.as_bytes(),
                },
            )
            .expect("sealing a short secret cannot fail");
        Sealed {
            nonce: nonce.to_vec(),
            ciphertext,
        }
    }

    pub(crate) fn open(&self, label: &str, sealed: &Sealed) -> Option<Zeroizing<String>> {
        if sealed.nonce.len() != NONCE_BYTES {
            return None;
        }
        let plain = self
            .cipher
            .decrypt(
                Nonce::from_slice(&sealed.nonce),
                Payload {
                    msg: &sealed.ciphertext,
                    aad: label.as_bytes(),
                },
            )
            .ok()?;
        String::from_utf8(plain).ok().map(Zeroizing::new)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sealed_secret_opens_only_with_its_key_and_label() {
        let key = yttt_relay_ticket::hex(&[7; 32]);
        let sealer = Sealer::new(&key).unwrap();
        let sealed = sealer.seal("github", "client-secret");
        assert_ne!(sealed.ciphertext, b"client-secret");
        assert_eq!(
            sealer
                .open("github", &sealed)
                .as_deref()
                .map(String::as_str),
            Some("client-secret")
        );
        assert!(sealer.open("google", &sealed).is_none());
        let other = Sealer::new(&yttt_relay_ticket::hex(&[8; 32])).unwrap();
        assert!(other.open("github", &sealed).is_none());
        assert!(Sealer::new("short").is_none());
    }
}
