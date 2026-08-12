use std::sync::Arc;

use keyring::{Entry, Error as KeyringError};
use thiserror::Error;
use yttt_core::model::ids::CredentialId;
use zeroize::Zeroizing;

const CREDENTIAL_SERVICE: &str = "dev.yttt.ssh";

#[derive(Clone, Debug)]
pub struct CredentialStore {
    service: Arc<str>,
}

impl CredentialStore {
    pub fn new(service: impl Into<Arc<str>>) -> Self {
        Self {
            service: service.into(),
        }
    }

    pub fn load(
        &self,
        credential_id: &CredentialId,
    ) -> Result<Option<Zeroizing<String>>, CredentialStoreError> {
        let entry = self.credential_entry(credential_id)?;
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
        self.credential_entry(credential_id)?
            .set_password(secret)
            .map_err(|source| CredentialStoreError::Access {
                credential_id: credential_id.clone(),
                source,
            })
    }

    pub fn delete(&self, credential_id: &CredentialId) -> Result<(), CredentialStoreError> {
        match self.credential_entry(credential_id)?.delete_credential() {
            Ok(()) | Err(KeyringError::NoEntry) => Ok(()),
            Err(source) => Err(CredentialStoreError::Access {
                credential_id: credential_id.clone(),
                source,
            }),
        }
    }

    fn credential_entry(
        &self,
        credential_id: &CredentialId,
    ) -> Result<Entry, CredentialStoreError> {
        Entry::new(&self.service, credential_id.as_str()).map_err(|source| {
            CredentialStoreError::Access {
                credential_id: credential_id.clone(),
                source,
            }
        })
    }
}

impl Default for CredentialStore {
    fn default() -> Self {
        Self::new(CREDENTIAL_SERVICE)
    }
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
