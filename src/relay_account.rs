//! The relay account this profile is signed in to.
//!
//! The account only buys relay transport: which Hosts this device trusts and
//! which devices a Host admits never depend on it. The session token lives in
//! the system credential store; the file beside the paired Hosts holds only
//! what the settings page shows.

use std::io::{Read as _, Write as _};

use serde::{Deserialize, Serialize};
use yttt_control_client::{ControlError, ControlSession, RelayTicket};
use yttt_core::model::ids::CredentialId;

use crate::{config::profile::AppProfile, paired_hosts::KeyStore};

const DISPLAY_FILE: &str = "relay-account.json";
const SESSION_ID: &str = "session";
const MAX_DISPLAY_BYTES: u64 = 4096;
/// The control plane's limit on a device label.
const MAX_LABEL_CHARS: usize = 64;

/// What the settings page shows about the account; no secret.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct AccountDisplay {
    pub control_url: String,
    pub email: String,
    pub account_id: u64,
}

pub(crate) fn session_store(profile: &AppProfile) -> yttt_ssh::CredentialStore {
    yttt_ssh::CredentialStore::new(format!("{}.relay-account", profile.credential_namespace()))
}

fn display_path(profile: &AppProfile) -> std::path::PathBuf {
    profile
        .paths()
        .state
        .join("client-connections")
        .join(DISPLAY_FILE)
}

pub(crate) fn read_display(profile: &AppProfile) -> Result<Option<AccountDisplay>, String> {
    let file = match std::fs::File::open(display_path(profile)) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };
    let mut bytes = Vec::new();
    file.take(MAX_DISPLAY_BYTES)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|error| format!("relay account: {error}"))
}

/// The signed-in session, if there is one and the store still has it.
pub(crate) fn load_session(store: &impl KeyStore) -> Result<Option<ControlSession>, String> {
    let Some(text) = store.load(&CredentialId::new(SESSION_ID))? else {
        return Ok(None);
    };
    serde_json::from_str(&text)
        .map(Some)
        .map_err(|_| "The stored relay account session is malformed; sign in again.".into())
}

/// Remembers a signed-in account: the session in the credential store first,
/// so the page never shows an account the app cannot use.
pub(crate) fn save(
    profile: &AppProfile,
    store: &impl KeyStore,
    email: &str,
    session: &ControlSession,
) -> Result<AccountDisplay, String> {
    let text = zeroize::Zeroizing::new(serde_json::to_string(session).map_err(|e| e.to_string())?);
    store.save(&CredentialId::new(SESSION_ID), &text)?;
    let display = AccountDisplay {
        control_url: session.control_url.clone(),
        email: email.trim().to_string(),
        account_id: session.account_id,
    };
    let path = display_path(profile);
    let directory = path.parent().expect("the display file has a directory");
    std::fs::create_dir_all(directory).map_err(|error| error.to_string())?;
    let mut temporary =
        tempfile::NamedTempFile::new_in(directory).map_err(|error| error.to_string())?;
    temporary
        .write_all(&serde_json::to_vec(&display).map_err(|error| error.to_string())?)
        .map_err(|error| error.to_string())?;
    temporary
        .persist(&path)
        .map_err(|error| error.to_string())?;
    Ok(display)
}

pub(crate) fn forget(profile: &AppProfile, store: &impl KeyStore) -> Result<(), String> {
    store.delete(&CredentialId::new(SESSION_ID))?;
    match std::fs::remove_file(display_path(profile)) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error.to_string()),
        _ => Ok(()),
    }
}

/// How the account lists a device key this Client holds for one Host.
pub(crate) fn device_label(host_name: &str) -> String {
    let local = gethostname::gethostname()
        .to_string_lossy()
        .trim()
        .to_string();
    let local = if local.is_empty() {
        "yttt".into()
    } else {
        local
    };
    format!("{local} → {host_name}")
        .chars()
        .take(MAX_LABEL_CHARS)
        .collect()
}

/// Registers `device_secret` with the account (repeating it is harmless) and
/// gets it a ticket for the account's relay node.
pub(crate) async fn ticket_for(
    session: &ControlSession,
    device_secret: &[u8; 32],
    label: &str,
) -> Result<RelayTicket, ControlError> {
    session.register_device(device_secret, label).await?;
    session.ticket(device_secret).await
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, collections::HashMap};

    use super::*;

    #[derive(Default)]
    struct MemoryStore(RefCell<HashMap<String, String>>);

    impl KeyStore for MemoryStore {
        fn load(&self, id: &CredentialId) -> Result<Option<zeroize::Zeroizing<String>>, String> {
            Ok(self
                .0
                .borrow()
                .get(id.as_str())
                .cloned()
                .map(zeroize::Zeroizing::new))
        }
        fn save(&self, id: &CredentialId, value: &str) -> Result<(), String> {
            self.0.borrow_mut().insert(id.as_str().into(), value.into());
            Ok(())
        }
        fn delete(&self, id: &CredentialId) -> Result<(), String> {
            self.0.borrow_mut().remove(id.as_str());
            Ok(())
        }
    }

    fn profile(root: &std::path::Path) -> AppProfile {
        use crate::config::profile::{
            EnvironmentKind, HostConnectPolicy, ProfilePersistence, ProjectConfigPolicy,
        };
        AppProfile::scoped(
            yttt_core::model::ids::ProfileId::new("relay-account"),
            EnvironmentKind::Test,
            ProfilePersistence::Ephemeral,
            root,
            ProjectConfigPolicy::Overlay,
            HostConnectPolicy::ProfileDiscovery,
        )
    }

    #[test]
    fn a_signed_in_account_keeps_its_token_out_of_the_file_and_signs_out_cleanly() {
        let temp = tempfile::tempdir().unwrap();
        let profile = profile(temp.path());
        let store = MemoryStore::default();
        assert_eq!(read_display(&profile).unwrap(), None);
        assert!(load_session(&store).unwrap().is_none());

        let session = ControlSession {
            control_url: "https://control.example".into(),
            account_id: 7,
            token: "secret-session-token".into(),
        };
        let display = save(&profile, &store, " owner@example.com ", &session).unwrap();
        assert_eq!(display.email, "owner@example.com");
        assert_eq!(read_display(&profile).unwrap(), Some(display));
        let file = std::fs::read_to_string(display_path(&profile)).unwrap();
        assert!(!file.contains("secret-session-token"));
        let loaded = load_session(&store).unwrap().unwrap();
        assert_eq!(loaded.token, "secret-session-token");
        assert_eq!(loaded.account_id, 7);

        forget(&profile, &store).unwrap();
        assert_eq!(read_display(&profile).unwrap(), None);
        assert!(load_session(&store).unwrap().is_none());
        forget(&profile, &store).unwrap();
    }

    #[test]
    fn device_labels_fit_the_control_planes_limit() {
        let label = device_label(&"office ".repeat(30));
        assert!(label.chars().count() <= MAX_LABEL_CHARS);
        assert!(label.contains(" → office"));
    }
}
