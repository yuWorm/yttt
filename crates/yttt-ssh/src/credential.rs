use keyring::{Entry, Error as KeyringError};
use thiserror::Error;
use yttt_core::model::ids::CredentialId;
use zeroize::Zeroizing;

const CREDENTIAL_SERVICE: &str = "dev.yttt.ssh";

#[derive(Clone, Debug, Default)]
pub struct CredentialStore;

impl CredentialStore {
    pub fn load(
        &self,
        credential_id: &CredentialId,
    ) -> Result<Option<Zeroizing<String>>, CredentialStoreError> {
        let entry = credential_entry(credential_id)?;
        match entry.get_password() {
            Ok(password) => Ok(Some(Zeroizing::new(password))),
            Err(KeyringError::NoEntry) => Ok(None),
            Err(source) => Err(CredentialStoreError::Access {
                credential_id: credential_id.clone(),
                source,
            }),
        }
    }

    pub fn save(
        &self,
        credential_id: &CredentialId,
        secret: &str,
    ) -> Result<(), CredentialStoreError> {
        credential_entry(credential_id)?
            .set_password(secret)
            .map_err(|source| CredentialStoreError::Access {
                credential_id: credential_id.clone(),
                source,
            })
    }

    pub fn delete(&self, credential_id: &CredentialId) -> Result<(), CredentialStoreError> {
        match credential_entry(credential_id)?.delete_credential() {
            Ok(()) | Err(KeyringError::NoEntry) => Ok(()),
            Err(source) => Err(CredentialStoreError::Access {
                credential_id: credential_id.clone(),
                source,
            }),
        }
    }
}

fn credential_entry(credential_id: &CredentialId) -> Result<Entry, CredentialStoreError> {
    Entry::new(CREDENTIAL_SERVICE, credential_id.as_str()).map_err(|source| {
        CredentialStoreError::Access {
            credential_id: credential_id.clone(),
            source,
        }
    })
}

#[derive(Debug, Error)]
pub enum CredentialStoreError {
    #[error("failed to access credential {credential_id}: {source}")]
    Access {
        credential_id: CredentialId,
        #[source]
        source: KeyringError,
    },
}
