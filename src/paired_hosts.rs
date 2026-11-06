//! Hosts this Client paired with over P2P, and the device key it holds for each.
//!
//! A record holds no secret: the Host's key, its IDs and address hints. Each
//! Host gets its own device key in the system credential store, so remote
//! Client processes for different Hosts never share an iroh identity, and a
//! Host revoking this device affects only itself.

use std::io::{Read as _, Write as _};

use rand::RngCore as _;
use serde::{Deserialize, Serialize};
use yttt_client_core::p2p::PairedHost;
use yttt_core::model::ids::CredentialId;
use yttt_protocol::EndpointKey;
use zeroize::Zeroizing;

use crate::config::profile::AppProfile;

pub(crate) const MAX_PAIRED_HOSTS: usize = 32;
const MAX_RECORDS_BYTES: usize = 64 * 1024;
const RECORDS_FILE: &str = "paired-hosts.json";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PairedHostRecord {
    #[serde(default)]
    pub name: String,
    pub host: PairedHost,
}

impl PairedHostRecord {
    /// What the list shows: the user's name, else a short form of the Host key.
    pub(crate) fn display_name(&self) -> String {
        let name = self.name.trim();
        if name.is_empty() {
            format!("P2P Host {}", &self.host.host.to_string()[..8])
        } else {
            name.to_string()
        }
    }
}

pub(crate) fn read_records(profile: &AppProfile) -> Result<Vec<PairedHostRecord>, String> {
    let path = profile
        .paths()
        .state
        .join("client-connections")
        .join(RECORDS_FILE);
    let file = match std::fs::File::open(&path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.to_string()),
    };
    let mut bytes = Vec::new();
    file.take(MAX_RECORDS_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    decode_records(&bytes)
}

fn decode_records(bytes: &[u8]) -> Result<Vec<PairedHostRecord>, String> {
    if bytes.len() > MAX_RECORDS_BYTES {
        return Err("Paired Host records exceed 64 KiB".into());
    }
    let records: Vec<PairedHostRecord> =
        serde_json::from_slice(bytes).map_err(|error| format!("Paired Host records: {error}"))?;
    if records.len() > MAX_PAIRED_HOSTS {
        return Err("Too many paired Hosts".into());
    }
    Ok(records)
}

pub(crate) fn save_records(
    profile: &AppProfile,
    records: &[PairedHostRecord],
) -> Result<(), String> {
    if records.len() > MAX_PAIRED_HOSTS {
        return Err(format!("At most {MAX_PAIRED_HOSTS} paired Hosts are kept"));
    }
    let bytes = serde_json::to_vec(records).map_err(|error| error.to_string())?;
    if bytes.len() > MAX_RECORDS_BYTES {
        return Err("Paired Host records exceed 64 KiB".into());
    }
    let directory = profile.paths().state.join("client-connections");
    std::fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    let mut temporary =
        tempfile::NamedTempFile::new_in(&directory).map_err(|error| error.to_string())?;
    temporary
        .write_all(&bytes)
        .map_err(|error| error.to_string())?;
    temporary
        .as_file()
        .sync_all()
        .map_err(|error| error.to_string())?;
    temporary
        .persist(directory.join(RECORDS_FILE))
        .map_err(|error| error.to_string())?;
    Ok(())
}

/// Adds `record`, replacing any record of the same Host: pairing again with a
/// Host supersedes the earlier pairing.
pub(crate) fn upsert(
    records: &mut Vec<PairedHostRecord>,
    record: PairedHostRecord,
) -> Result<(), String> {
    if let Some(existing) = records
        .iter_mut()
        .find(|existing| existing.host.host == record.host.host)
    {
        *existing = record;
        return Ok(());
    }
    if records.len() >= MAX_PAIRED_HOSTS {
        return Err(format!("At most {MAX_PAIRED_HOSTS} paired Hosts are kept"));
    }
    records.push(record);
    Ok(())
}

pub(crate) fn find(records: &[PairedHostRecord], host: EndpointKey) -> Option<&PairedHostRecord> {
    records.iter().find(|record| record.host.host == host)
}

/// Where device keys live; a trait so tests need no system keychain.
pub(crate) trait KeyStore {
    fn load(&self, id: &CredentialId) -> Result<Option<Zeroizing<String>>, String>;
    fn save(&self, id: &CredentialId, value: &str) -> Result<(), String>;
    fn delete(&self, id: &CredentialId) -> Result<(), String>;
}

impl KeyStore for yttt_ssh::CredentialStore {
    fn load(&self, id: &CredentialId) -> Result<Option<Zeroizing<String>>, String> {
        yttt_ssh::CredentialStore::load(self, id).map_err(|error| error.to_string())
    }
    fn save(&self, id: &CredentialId, value: &str) -> Result<(), String> {
        yttt_ssh::CredentialStore::save(self, id, value).map_err(|error| error.to_string())
    }
    fn delete(&self, id: &CredentialId) -> Result<(), String> {
        yttt_ssh::CredentialStore::delete(self, id).map_err(|error| error.to_string())
    }
}

pub(crate) fn device_key_store(profile: &AppProfile) -> yttt_ssh::CredentialStore {
    yttt_ssh::CredentialStore::new(format!("{}.p2p-device", profile.credential_namespace()))
}

fn key_id(host: EndpointKey) -> CredentialId {
    CredentialId::new(host.to_string())
}

/// A fresh device key for pairing with one Host; it is saved only once the
/// Host accepted it.
pub(crate) fn new_device_key() -> Zeroizing<[u8; 32]> {
    let mut secret = Zeroizing::new([0_u8; 32]);
    rand::rngs::OsRng.fill_bytes(secret.as_mut());
    secret
}

/// The device key for `host`. A missing key is an error, never a reason to
/// make a new one: the Host knows only the key it accepted at pairing.
pub(crate) fn load_device_key(
    store: &impl KeyStore,
    host: EndpointKey,
) -> Result<Zeroizing<[u8; 32]>, String> {
    let text = store
        .load(&key_id(host))?
        .ok_or("This device's key for the Host is missing; pair with the Host again.")?;
    let mut secret = Zeroizing::new([0_u8; 32]);
    let digits = text.trim().as_bytes();
    if digits.len() != 64 {
        return Err("The stored device key is malformed; pair with the Host again.".into());
    }
    for (byte, pair) in secret.iter_mut().zip(digits.chunks(2)) {
        let pair = std::str::from_utf8(pair).map_err(|_| "The stored device key is malformed")?;
        *byte = u8::from_str_radix(pair, 16).map_err(|_| "The stored device key is malformed")?;
    }
    Ok(secret)
}

pub(crate) fn save_device_key(
    store: &impl KeyStore,
    host: EndpointKey,
    secret: &[u8; 32],
) -> Result<(), String> {
    let mut text = Zeroizing::new(String::with_capacity(64));
    for byte in secret {
        use std::fmt::Write as _;
        let _ = write!(text, "{byte:02x}");
    }
    store.save(&key_id(host), &text)
}

pub(crate) fn delete_device_key(store: &impl KeyStore, host: EndpointKey) -> Result<(), String> {
    store.delete(&key_id(host))
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, collections::HashMap};

    use super::*;

    #[derive(Default)]
    struct MemoryStore(RefCell<HashMap<String, String>>);

    impl KeyStore for MemoryStore {
        fn load(&self, id: &CredentialId) -> Result<Option<Zeroizing<String>>, String> {
            Ok(self
                .0
                .borrow()
                .get(id.as_str())
                .cloned()
                .map(Zeroizing::new))
        }
        fn save(&self, id: &CredentialId, value: &str) -> Result<(), String> {
            self.0
                .borrow_mut()
                .insert(id.as_str().to_string(), value.to_string());
            Ok(())
        }
        fn delete(&self, id: &CredentialId) -> Result<(), String> {
            self.0.borrow_mut().remove(id.as_str());
            Ok(())
        }
    }

    fn host(byte: u8) -> PairedHost {
        PairedHost {
            host: EndpointKey([byte; 32]),
            environment_id: "environment".into(),
            profile_id: yttt_core::model::ids::ProfileId::new("default"),
            host_label: "environment".into(),
            relay_url: Some("https://relay.example".into()),
            direct_addrs: vec!["192.168.1.20:4433".parse().unwrap()],
        }
    }

    #[test]
    fn device_keys_are_kept_per_host_and_never_invented() {
        let store = MemoryStore::default();
        let first = new_device_key();
        let second = new_device_key();
        assert_ne!(*first, *second);
        save_device_key(&store, EndpointKey([1; 32]), &first).unwrap();
        save_device_key(&store, EndpointKey([2; 32]), &second).unwrap();
        assert_eq!(
            *load_device_key(&store, EndpointKey([1; 32])).unwrap(),
            *first
        );
        assert_eq!(
            *load_device_key(&store, EndpointKey([2; 32])).unwrap(),
            *second
        );
        delete_device_key(&store, EndpointKey([1; 32])).unwrap();
        assert!(load_device_key(&store, EndpointKey([1; 32])).is_err());
        store
            .save(&key_id(EndpointKey([3; 32])), "not a key")
            .unwrap();
        assert!(load_device_key(&store, EndpointKey([3; 32])).is_err());
    }

    #[test]
    fn pairing_again_replaces_the_hosts_record() {
        let mut records = Vec::new();
        upsert(
            &mut records,
            PairedHostRecord {
                name: "office".into(),
                host: host(1),
            },
        )
        .unwrap();
        upsert(
            &mut records,
            PairedHostRecord {
                name: String::new(),
                host: host(2),
            },
        )
        .unwrap();
        let mut moved = host(1);
        moved.direct_addrs.clear();
        upsert(
            &mut records,
            PairedHostRecord {
                name: "office again".into(),
                host: moved.clone(),
            },
        )
        .unwrap();
        assert_eq!(records.len(), 2);
        assert_eq!(find(&records, moved.host).unwrap().host, moved);
        assert_eq!(records[1].display_name(), "P2P Host 02020202");
    }

    #[test]
    fn records_have_bounds_and_a_damaged_file_is_an_error() {
        let mut records = Vec::new();
        for byte in 0..MAX_PAIRED_HOSTS as u8 {
            upsert(
                &mut records,
                PairedHostRecord {
                    name: String::new(),
                    host: host(byte),
                },
            )
            .unwrap();
        }
        assert!(
            upsert(
                &mut records,
                PairedHostRecord {
                    name: String::new(),
                    host: host(200),
                },
            )
            .is_err()
        );
        let bytes = serde_json::to_vec(&records).unwrap();
        assert_eq!(decode_records(&bytes).unwrap(), records);
        assert!(decode_records(b"{ not json").is_err());
        assert!(decode_records(&vec![b' '; MAX_RECORDS_BYTES + 1]).is_err());
    }
}
