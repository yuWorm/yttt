use std::{fs, io, path::PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

const HOST_KEY_STORE_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum HostKeyVerification {
    Trusted,
    Unknown,
    Changed { previous_fingerprint: String },
}

#[derive(Debug)]
pub(crate) struct HostKeyStore {
    path: PathBuf,
    records: Vec<HostKeyRecord>,
}

impl HostKeyStore {
    pub(crate) fn load(path: impl Into<PathBuf>) -> Result<Self, HostKeyStoreError> {
        let path = path.into();
        if !path.exists() {
            return Ok(Self {
                path,
                records: Vec::new(),
            });
        }

        let source = fs::read_to_string(&path).map_err(|source| HostKeyStoreError::Read {
            path: path.clone(),
            source,
        })?;
        let mut stored: StoredHostKeys =
            toml::from_str(&source).map_err(|source| HostKeyStoreError::Parse {
                path: path.clone(),
                source,
            })?;
        if stored.version != HOST_KEY_STORE_VERSION {
            return Err(HostKeyStoreError::UnsupportedVersion {
                path,
                version: stored.version,
            });
        }

        for (index, record) in stored.keys.iter_mut().enumerate() {
            record.host = normalize_host(&record.host);
            if record.host.is_empty()
                || record.algorithm.trim().is_empty()
                || record.fingerprint.trim().is_empty()
            {
                return Err(HostKeyStoreError::InvalidRecord {
                    path,
                    index: index + 1,
                });
            }
        }
        stored.keys.sort_by(record_order);
        if stored
            .keys
            .windows(2)
            .any(|pair| same_identity(&pair[0], &pair[1]))
        {
            return Err(HostKeyStoreError::DuplicateRecord { path });
        }

        Ok(Self {
            path,
            records: stored.keys,
        })
    }

    pub(crate) fn verify(
        &self,
        host: &str,
        port: u16,
        algorithm: &str,
        fingerprint: &str,
    ) -> HostKeyVerification {
        let host = normalize_host(host);
        let Some(record) = self.records.iter().find(|record| {
            record.host == host && record.port == port && record.algorithm == algorithm
        }) else {
            return HostKeyVerification::Unknown;
        };
        if record.fingerprint == fingerprint {
            HostKeyVerification::Trusted
        } else {
            HostKeyVerification::Changed {
                previous_fingerprint: record.fingerprint.clone(),
            }
        }
    }

    pub(crate) fn remember(
        &mut self,
        host: &str,
        port: u16,
        algorithm: &str,
        fingerprint: &str,
    ) -> Result<(), HostKeyStoreError> {
        let host = normalize_host(host);
        if let Some(record) = self.records.iter_mut().find(|record| {
            record.host == host && record.port == port && record.algorithm == algorithm
        }) {
            if record.fingerprint == fingerprint {
                return Ok(());
            }
            record.fingerprint = fingerprint.to_string();
        } else {
            self.records.push(HostKeyRecord {
                host,
                port,
                algorithm: algorithm.to_string(),
                fingerprint: fingerprint.to_string(),
            });
        }
        self.records.sort_by(record_order);
        self.persist()
    }

    fn persist(&self) -> Result<(), HostKeyStoreError> {
        if let Some(parent) = self
            .path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent).map_err(|source| HostKeyStoreError::Write {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        let source = toml::to_string_pretty(&StoredHostKeys {
            version: HOST_KEY_STORE_VERSION,
            keys: self.records.clone(),
        })
        .map_err(|source| HostKeyStoreError::Serialize {
            path: self.path.clone(),
            source,
        })?;
        let temp_path = self.path.with_extension("toml.tmp");
        fs::write(&temp_path, source).map_err(|source| HostKeyStoreError::Write {
            path: temp_path.clone(),
            source,
        })?;
        fs::rename(&temp_path, &self.path).map_err(|source| HostKeyStoreError::Write {
            path: self.path.clone(),
            source,
        })
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
struct StoredHostKeys {
    #[serde(default = "default_version")]
    version: u32,
    #[serde(default)]
    keys: Vec<HostKeyRecord>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
struct HostKeyRecord {
    host: String,
    port: u16,
    algorithm: String,
    fingerprint: String,
}

fn default_version() -> u32 {
    HOST_KEY_STORE_VERSION
}

fn normalize_host(host: &str) -> String {
    host.trim().trim_end_matches('.').to_ascii_lowercase()
}

fn same_identity(left: &HostKeyRecord, right: &HostKeyRecord) -> bool {
    left.host == right.host && left.port == right.port && left.algorithm == right.algorithm
}

fn record_order(left: &HostKeyRecord, right: &HostKeyRecord) -> std::cmp::Ordering {
    left.host
        .cmp(&right.host)
        .then_with(|| left.port.cmp(&right.port))
        .then_with(|| left.algorithm.cmp(&right.algorithm))
}

#[derive(Debug, Error)]
pub(crate) enum HostKeyStoreError {
    #[error("failed to read SSH host key store {path}: {source}")]
    Read { path: PathBuf, source: io::Error },
    #[error("failed to parse SSH host key store {path}: {source}")]
    Parse {
        path: PathBuf,
        source: toml::de::Error,
    },
    #[error("unsupported SSH host key store version {version} in {path}")]
    UnsupportedVersion { path: PathBuf, version: u32 },
    #[error("invalid SSH host key record {index} in {path}")]
    InvalidRecord { path: PathBuf, index: usize },
    #[error("duplicate SSH host key records in {path}")]
    DuplicateRecord { path: PathBuf },
    #[error("failed to serialize SSH host key store {path}: {source}")]
    Serialize {
        path: PathBuf,
        source: toml::ser::Error,
    },
    #[error("failed to write SSH host key store {path}: {source}")]
    Write { path: PathBuf, source: io::Error },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_trust_persists_and_reload_verifies_the_endpoint() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("ssh-host-keys.toml");
        let mut store = HostKeyStore::load(&path).unwrap();

        assert_eq!(
            store.verify("Example.COM.", 22, "ssh-ed25519", "SHA256:first"),
            HostKeyVerification::Unknown
        );
        store
            .remember("Example.COM.", 22, "ssh-ed25519", "SHA256:first")
            .unwrap();

        let reloaded = HostKeyStore::load(&path).unwrap();
        assert_eq!(
            reloaded.verify("example.com", 22, "ssh-ed25519", "SHA256:first"),
            HostKeyVerification::Trusted
        );
        assert_eq!(
            reloaded.verify("example.com", 22, "ssh-rsa", "SHA256:first"),
            HostKeyVerification::Unknown
        );
    }

    #[test]
    fn remembering_a_changed_key_replaces_the_saved_fingerprint() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("ssh-host-keys.toml");
        let mut store = HostKeyStore::load(&path).unwrap();
        store
            .remember("example.com", 2222, "ssh-ed25519", "SHA256:old")
            .unwrap();

        assert_eq!(
            store.verify("example.com", 2222, "ssh-ed25519", "SHA256:new"),
            HostKeyVerification::Changed {
                previous_fingerprint: "SHA256:old".to_string(),
            }
        );
        store
            .remember("example.com", 2222, "ssh-ed25519", "SHA256:new")
            .unwrap();

        let reloaded = HostKeyStore::load(&path).unwrap();
        assert_eq!(
            reloaded.verify("example.com", 2222, "ssh-ed25519", "SHA256:new"),
            HostKeyVerification::Trusted
        );
        assert_eq!(
            reloaded.verify("example.com", 2222, "ssh-ed25519", "SHA256:old"),
            HostKeyVerification::Changed {
                previous_fingerprint: "SHA256:new".to_string(),
            }
        );
        assert_eq!(
            fs::read_to_string(path)
                .unwrap()
                .matches("[[keys]]")
                .count(),
            1
        );
    }
}
