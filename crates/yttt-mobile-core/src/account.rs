//! The paid-relay account a phone signs in to, and the device keys it
//! registers there.
//!
//! The account only buys relay transport, as on the desktop: which Hosts the
//! phone trusts never depends on it. A relay node admits a device only with a
//! ticket, and a ticket is for one device key registered as a Client of one
//! Host, which it cannot change afterwards. So the phone has a key per Host,
//! derived from its one device secret, and registers that key before pairing
//! and on every connect.

use sha2::{Digest as _, Sha256};
use yttt_client_core::control::{
    BrowserLogin, ControlClient, ControlError, ControlSession, DeviceKind, RelayTicket,
};
use yttt_protocol::EndpointKey;

/// How long connecting waits for a ticket before it falls back to the
/// pairing relay, as the desktop does: the control plane's own timeout is
/// 20 s, which a network that cannot reach it would pay on every connect.
pub const TICKET_WAIT: std::time::Duration = std::time::Duration::from_secs(5);
/// The control plane's limit on a device label.
const MAX_LABEL_CHARS: usize = 64;

#[derive(Debug, thiserror::Error)]
pub enum AccountError {
    /// The session is no longer valid: sign in again.
    #[error("the relay account's sign-in expired")]
    SignInExpired,
    #[error("the stored relay account is unreadable: {0}")]
    Stored(String),
    #[error("{0}")]
    Control(String),
}

impl From<ControlError> for AccountError {
    fn from(error: ControlError) -> Self {
        if error.needs_login() {
            Self::SignInExpired
        } else {
            Self::Control(error.to_string())
        }
    }
}

/// The key this phone uses with `host`: its own for each Host, as the
/// control plane ties a key to one Host, and the same one each time, so
/// pairing again with a Host keeps its registration.
pub fn host_key(device_secret: &[u8; 32], host: &EndpointKey) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(b"yttt-mobile host key v1");
    digest.update(device_secret);
    digest.update(host.0);
    digest.finalize().into()
}

/// A signed-in account: what the app stores in the Keychain or Keystore.
#[derive(Clone, Debug)]
pub struct Account {
    pub session: ControlSession,
    pub email: String,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Stored {
    session: ControlSession,
    email: String,
}

impl Account {
    pub fn to_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(&Stored {
            session: self.session.clone(),
            email: self.email.clone(),
        })
        .expect("an account serializes")
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, AccountError> {
        let stored: Stored = serde_json::from_slice(bytes)
            .map_err(|error| AccountError::Stored(error.to_string()))?;
        Ok(Self {
            session: stored.session,
            email: stored.email,
        })
    }

    /// Signs in with the account's email and password.
    pub async fn sign_in(
        control_url: &str,
        email: &str,
        password: &str,
    ) -> Result<Self, AccountError> {
        let session = ControlClient::new(control_url)?
            .login(email, password)
            .await?;
        Ok(Self {
            session,
            email: email.to_owned(),
        })
    }

    /// The account behind a session the browser sign-in returned.
    pub async fn from_session(session: ControlSession) -> Result<Self, AccountError> {
        let view = session.account().await?;
        Ok(Self {
            session,
            email: view.email,
        })
    }

    /// Registers this phone's key for `host` and gets it a ticket for that
    /// Host's relay node.
    pub async fn ticket(
        &self,
        device_secret: &[u8; 32],
        host: &EndpointKey,
        label: &str,
    ) -> Result<RelayTicket, AccountError> {
        let key = host_key(device_secret, host);
        let kind = DeviceKind::Client { host: host.0 };
        self.session.register_device(&key, label, kind).await?;
        Ok(self.session.ticket(&key, kind).await?)
    }

    /// Removes this phone's key for `host` from the account, freeing a
    /// device slot of the plan.
    pub async fn release(
        &self,
        device_secret: &[u8; 32],
        host: &EndpointKey,
    ) -> Result<(), AccountError> {
        let key = yttt_transport_iroh::endpoint_key(&host_key(device_secret, host));
        Ok(self.session.remove_device(&key.0).await?)
    }
}

/// A browser sign-in: open `authorize_url` in a browser, and `finish` waits on
/// a loopback port for the console to send the browser back (RFC 8252).
pub async fn begin_browser_sign_in(
    control_url: &str,
    label: &str,
) -> Result<BrowserLogin, AccountError> {
    Ok(ControlClient::new(control_url)?
        .begin_browser_login(&truncate(label, MAX_LABEL_CHARS))
        .await?)
}

/// The label the console shows for this phone's key for a Host, e.g.
/// "iPhone → office"; a long phone name gives up its tail first.
pub fn device_label(phone: &str, host: &str) -> String {
    let host = truncate(host, MAX_LABEL_CHARS / 2);
    let separator = " → ";
    let room = MAX_LABEL_CHARS - host.chars().count() - separator.chars().count();
    format!("{}{separator}{host}", truncate(phone, room))
}

fn truncate(text: &str, chars: usize) -> String {
    text.trim().chars().take(chars).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_host_has_its_own_key_and_keeps_it() {
        let secret = [7; 32];
        let office = EndpointKey([1; 32]);
        let home = EndpointKey([2; 32]);
        assert_eq!(host_key(&secret, &office), host_key(&secret, &office));
        assert_ne!(host_key(&secret, &office), host_key(&secret, &home));
        assert_ne!(host_key(&secret, &office), host_key(&[8; 32], &office));
        assert_ne!(host_key(&secret, &office), secret);
    }

    #[test]
    fn an_account_survives_storage_without_its_token_in_debug() {
        let account = Account {
            session: ControlSession {
                control_url: "https://control.example".into(),
                account_id: 42,
                token: "secret-token".into(),
            },
            email: "owner@example.com".into(),
        };
        let stored = Account::from_bytes(&account.to_bytes()).unwrap();
        assert_eq!(stored.email, "owner@example.com");
        assert_eq!(stored.session.account_id, 42);
        assert_eq!(stored.session.token, "secret-token");
        assert!(!format!("{stored:?}").contains("secret-token"));
        assert!(matches!(
            Account::from_bytes(b"{}"),
            Err(AccountError::Stored(_))
        ));
    }

    #[test]
    fn an_expired_session_asks_to_sign_in_again() {
        let expired = ControlError::Refused {
            status: 401,
            message: "unauthorized".into(),
        };
        assert!(matches!(
            AccountError::from(expired),
            AccountError::SignInExpired
        ));
        let refused = ControlError::Refused {
            status: 403,
            message: "the plan's device limit is reached".into(),
        };
        assert!(
            matches!(AccountError::from(refused), AccountError::Control(message) if message.contains("device limit"))
        );
    }

    #[test]
    fn device_labels_fit_the_control_planes_limit() {
        assert_eq!(device_label("iPhone", "office"), "iPhone → office");
        let long = device_label(&"phone".repeat(30), &"office ".repeat(30));
        assert!(long.chars().count() <= MAX_LABEL_CHARS);
        assert!(long.contains(" → office"));
        assert!(long.starts_with("phonephone"));
    }
}
