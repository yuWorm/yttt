use yttt_core::model::ids::CredentialId;
use zeroize::Zeroizing;

const PREFIX: &str = "yttt-connection-chunks-v1:";
// Windows permits 2560 bytes per UTF-16 password. Connection codes are ASCII.
const CHUNK_BYTES: usize = 1200;
const MAX_CHUNKS: usize = super::MAX_CONNECTION_CODE_BYTES.div_ceil(CHUNK_BYTES);

pub(super) trait SecretStore {
    fn load(&self, id: &CredentialId) -> Result<Option<Zeroizing<String>>, String>;
    fn save(&self, id: &CredentialId, value: &str) -> Result<(), String>;
    fn delete(&self, id: &CredentialId) -> Result<(), String>;
}

impl SecretStore for yttt_ssh::CredentialStore {
    fn load(&self, id: &CredentialId) -> Result<Option<Zeroizing<String>>, String> {
        self.load(id).map_err(|error| error.to_string())
    }
    fn save(&self, id: &CredentialId, value: &str) -> Result<(), String> {
        self.save(id, value).map_err(|error| error.to_string())
    }
    fn delete(&self, id: &CredentialId) -> Result<(), String> {
        self.delete(id).map_err(|error| error.to_string())
    }
}

pub(super) struct ConnectionCredentialStore<S = yttt_ssh::CredentialStore>(S);

impl ConnectionCredentialStore {
    pub(super) fn new(namespace: String) -> Self {
        Self(yttt_ssh::CredentialStore::new(namespace))
    }
}

fn chunks(value: &str) -> Result<Option<(String, usize)>, String> {
    let Some(manifest) = value.strip_prefix(PREFIX) else {
        return Ok(None);
    };
    let (generation, count) = manifest
        .split_once(':')
        .ok_or("Invalid credential manifest")?;
    let count = count
        .parse::<usize>()
        .map_err(|_| "Invalid credential chunk count")?;
    if generation.is_empty()
        || generation.len() > 64
        || !generation
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        || !(1..=MAX_CHUNKS).contains(&count)
    {
        return Err("Invalid credential manifest".into());
    }
    Ok(Some((generation.to_string(), count)))
}

fn chunk_id(id: &CredentialId, generation: &str, index: usize) -> CredentialId {
    CredentialId::new(format!("{}.{}.{}", id.as_str(), generation, index))
}

impl<S: SecretStore> ConnectionCredentialStore<S> {
    pub(super) fn load(&self, id: &CredentialId) -> Result<Option<Zeroizing<String>>, String> {
        let Some(value) = self.0.load(id)? else {
            return Ok(None);
        };
        let Some((generation, count)) = chunks(&value)? else {
            return Ok(Some(value));
        };
        let mut value = Zeroizing::new(String::new());
        for index in 0..count {
            let part = self.0.load(&chunk_id(id, &generation, index))?.ok_or(
                "Saved connection credentials are incomplete; paste the connection code again.",
            )?;
            if !part.is_ascii() || part.len() > CHUNK_BYTES {
                return Err("Invalid credential chunk".into());
            }
            value.push_str(&part);
        }
        if value.len() > super::MAX_CONNECTION_CODE_BYTES {
            return Err("Saved connection code is too large".into());
        }
        Ok(Some(value))
    }

    pub(super) fn save(&self, id: &CredentialId, value: &str) -> Result<(), String> {
        if !value.is_ascii() || value.len() > super::MAX_CONNECTION_CODE_BYTES {
            return Err("Invalid connection code".into());
        }
        let previous = self.0.load(id)?;
        let previous_chunks = previous
            .as_deref()
            .and_then(|value| chunks(value).ok().flatten());
        if value.len() <= CHUNK_BYTES {
            self.0.save(id, value)?;
        } else {
            let generation = CredentialId::random();
            let count = value.len().div_ceil(CHUNK_BYTES);
            let result = (|| {
                for (index, part) in value.as_bytes().chunks(CHUNK_BYTES).enumerate() {
                    self.0.save(
                        &chunk_id(id, generation.as_str(), index),
                        std::str::from_utf8(part).unwrap(),
                    )?;
                }
                // Publish only after every chunk is durable, preserving the old code on failure.
                self.0
                    .save(id, &format!("{PREFIX}{}:{count}", generation.as_str()))
            })();
            if result.is_err() {
                self.cleanup(id, generation.as_str(), count);
                return result;
            }
        }
        if let Some((generation, count)) = previous_chunks {
            self.cleanup(id, &generation, count);
        }
        Ok(())
    }

    fn cleanup(&self, id: &CredentialId, generation: &str, count: usize) {
        for index in 0..count {
            let _ = self.0.delete(&chunk_id(id, generation, index));
        }
    }

    pub(super) fn delete(&self, id: &CredentialId) -> Result<(), String> {
        if let Some(value) = self.0.load(id)?
            && let Ok(Some((generation, count))) = chunks(&value)
        {
            for index in 0..count {
                self.0.delete(&chunk_id(id, &generation, index))?;
            }
        }
        self.0.delete(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::RefCell, collections::HashMap, rc::Rc};

    #[derive(Clone, Default)]
    struct MemoryStore {
        values: Rc<RefCell<HashMap<String, String>>>,
        fail_at: Rc<RefCell<Option<String>>>,
    }
    impl SecretStore for MemoryStore {
        fn load(&self, id: &CredentialId) -> Result<Option<Zeroizing<String>>, String> {
            Ok(self
                .values
                .borrow()
                .get(id.as_str())
                .cloned()
                .map(Zeroizing::new))
        }
        fn save(&self, id: &CredentialId, value: &str) -> Result<(), String> {
            if self
                .fail_at
                .borrow()
                .as_deref()
                .is_some_and(|suffix| id.as_str().ends_with(suffix))
            {
                return Err("storage unavailable".into());
            }
            assert!(value.encode_utf16().count() * 2 <= 2560);
            self.values
                .borrow_mut()
                .insert(id.as_str().into(), value.into());
            Ok(())
        }
        fn delete(&self, id: &CredentialId) -> Result<(), String> {
            self.values.borrow_mut().remove(id.as_str());
            Ok(())
        }
    }

    #[test]
    fn long_codes_survive_reopening_replacement_and_deletion() {
        let backend = MemoryStore::default();
        let id = CredentialId::new("connection");
        let code = "A".repeat(super::super::MAX_CONNECTION_CODE_BYTES);
        ConnectionCredentialStore(backend.clone())
            .save(&id, &code)
            .unwrap();
        let reopened = ConnectionCredentialStore(backend.clone());
        assert_eq!(reopened.load(&id).unwrap().unwrap().as_str(), code);
        reopened.save(&id, &"B".repeat(2401)).unwrap();
        assert_eq!(backend.values.borrow().len(), 4);
        reopened.save(&id, "legacy-compatible-code").unwrap();
        assert_eq!(backend.values.borrow().len(), 1);
        assert_eq!(
            reopened.load(&id).unwrap().unwrap().as_str(),
            "legacy-compatible-code"
        );
        reopened.save(&id, &code).unwrap();
        reopened.delete(&id).unwrap();
        reopened.delete(&id).unwrap();
        assert!(backend.values.borrow().is_empty());
    }

    #[test]
    fn connection_code_round_trip_keeps_authentication_and_metadata_route() {
        use super::super::{ConnectionCode, RememberedConnection, decode_saved_connection};
        let backend = MemoryStore::default();
        let record = RememberedConnection {
            name: "Remote".into(),
            address: "override.example:43123".into(),
            environment_id: "remote".into(),
            credential_id: CredentialId::new("connection"),
        };
        let info = yttt_protocol::remote_access::RemoteConnectionInfo {
            environment_id: record.environment_id.clone(),
            profile_id: yttt_core::model::ids::ProfileId::new("remote-profile"),
            server_name: "yttt-host.local".into(),
            certificate_der: (0..512).map(|index| index as u8).collect(),
            certificate_sha256: "ab".repeat(32),
            credential_generation: 7,
            work_secret: [42; 32],
        };
        let code = ConnectionCode::encode("original.example:43123".into(), info.clone()).unwrap();
        assert!(code.encode_utf16().count() * 2 > 2560);
        ConnectionCredentialStore(backend.clone())
            .save(&record.credential_id, &code)
            .unwrap();
        let saved = ConnectionCredentialStore(backend)
            .load(&record.credential_id)
            .unwrap()
            .unwrap();
        let (address, restored) = decode_saved_connection(&record, &saved).unwrap();
        assert_eq!(address, record.address);
        assert_eq!(restored, info);
    }

    #[test]
    fn failed_chunk_or_manifest_write_preserves_previous_credentials() {
        for suffix in [".1", "connection"] {
            let backend = MemoryStore::default();
            let store = ConnectionCredentialStore(backend.clone());
            let id = CredentialId::new("connection");
            store.save(&id, "old-code").unwrap();
            *backend.fail_at.borrow_mut() = Some(suffix.into());
            assert!(store.save(&id, &"A".repeat(3000)).is_err());
            assert_eq!(store.load(&id).unwrap().unwrap().as_str(), "old-code");
            assert_eq!(backend.values.borrow().len(), 1);
        }
    }

    #[test]
    fn legacy_json_and_missing_or_invalid_chunks_are_handled() {
        let backend = MemoryStore::default();
        let store = ConnectionCredentialStore(backend.clone());
        let id = CredentialId::new("connection");
        backend.save(&id, "{\"legacy\":true}").unwrap();
        assert_eq!(
            store.load(&id).unwrap().unwrap().as_str(),
            "{\"legacy\":true}"
        );
        backend.save(&id, &format!("{PREFIX}generation:1")).unwrap();
        assert!(store.load(&id).is_err());
        backend
            .save(&id, &format!("{PREFIX}generation:99999"))
            .unwrap();
        assert!(store.load(&id).is_err());
        store.save(&id, "replacement-code").unwrap();
        assert_eq!(
            store.load(&id).unwrap().unwrap().as_str(),
            "replacement-code"
        );
        store.delete(&id).unwrap();
        assert!(store.load(&id).unwrap().is_none());
    }
}
