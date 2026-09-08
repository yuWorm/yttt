use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use serde::{Deserialize, Serialize};
use yttt_core::model::ids::{HostId, TerminalSessionId};

use crate::config::storage::ConfigStorage;

const TERMINAL_PLACEMENTS_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum DurableTerminalPlacement {
    OpenPending {
        request_id: u64,
        spawn_fingerprint: u64,
    },
    Bound {
        host_id: HostId,
        host_epoch: u64,
        session_id: TerminalSessionId,
        session_epoch: u64,
        spawn_fingerprint: u64,
    },
    ClosePending {
        host_id: HostId,
        host_epoch: u64,
        session_id: TerminalSessionId,
        session_epoch: u64,
        spawn_fingerprint: u64,
        request_id: u64,
    },
    Lost {
        host_id: HostId,
        host_epoch: u64,
        session_id: TerminalSessionId,
        session_epoch: u64,
        spawn_fingerprint: u64,
        reason: String,
    },
    Closed,
}

impl DurableTerminalPlacement {
    pub fn spawn_fingerprint(&self) -> Option<u64> {
        match self {
            Self::OpenPending {
                spawn_fingerprint, ..
            }
            | Self::Bound {
                spawn_fingerprint, ..
            }
            | Self::ClosePending {
                spawn_fingerprint, ..
            }
            | Self::Lost {
                spawn_fingerprint, ..
            } => Some(*spawn_fingerprint),
            Self::Closed => None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct TerminalPlacementFile {
    #[serde(default = "placement_version")]
    version: u32,
    #[serde(default = "first_request_id")]
    next_request_id: u64,
    #[serde(default)]
    placements: BTreeMap<String, DurableTerminalPlacement>,
}

impl Default for TerminalPlacementFile {
    fn default() -> Self {
        Self {
            version: TERMINAL_PLACEMENTS_VERSION,
            next_request_id: first_request_id(),
            placements: BTreeMap::new(),
        }
    }
}

fn placement_version() -> u32 {
    TERMINAL_PLACEMENTS_VERSION
}

fn first_request_id() -> u64 {
    1
}

pub struct TerminalPlacementStore {
    path: PathBuf,
    state: Mutex<TerminalPlacementFile>,
    storage: PlacementStorage,
}

enum PlacementStorage {
    Host(Arc<dyn ConfigStorage>),
    #[cfg(test)]
    Local,
}

impl PlacementStorage {
    fn read(&self, path: &Path) -> std::io::Result<Vec<u8>> {
        match self {
            Self::Host(storage) => storage.read(path),
            #[cfg(test)]
            Self::Local => std::fs::read(path),
        }
    }

    fn write(&self, path: &Path, bytes: &[u8]) -> std::io::Result<()> {
        match self {
            Self::Host(storage) => storage.write(path, bytes),
            #[cfg(test)]
            Self::Local => {
                std::fs::create_dir_all(path.parent().unwrap())?;
                crate::config::atomic_write(path, bytes)
            }
        }
    }
}

impl TerminalPlacementStore {
    pub fn load(
        path: impl Into<PathBuf>,
        storage: Arc<dyn ConfigStorage>,
    ) -> Result<Self, TerminalPlacementStoreError> {
        Self::load_with_storage(path.into(), PlacementStorage::Host(storage))
    }

    #[cfg(test)]
    pub(crate) fn load_local(
        path: impl Into<PathBuf>,
    ) -> Result<Self, TerminalPlacementStoreError> {
        let path = path.into();
        crate::config::storage::allow_test_root(path.parent().unwrap());
        Self::load_with_storage(path, PlacementStorage::Local)
    }

    fn load_with_storage(
        path: PathBuf,
        storage: PlacementStorage,
    ) -> Result<Self, TerminalPlacementStoreError> {
        let state = match storage.read(&path) {
            Ok(bytes) => {
                let state: TerminalPlacementFile =
                    serde_json::from_slice(&bytes).map_err(|source| {
                        TerminalPlacementStoreError::Parse {
                            path: path.clone(),
                            source,
                        }
                    })?;
                if state.version != TERMINAL_PLACEMENTS_VERSION {
                    return Err(TerminalPlacementStoreError::Version {
                        path,
                        actual: state.version,
                    });
                }
                state
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                TerminalPlacementFile::default()
            }
            Err(source) => {
                return Err(TerminalPlacementStoreError::Read {
                    path: path.clone(),
                    source,
                });
            }
        };
        Ok(Self {
            path,
            state: Mutex::new(state),
            storage,
        })
    }

    pub fn placement(&self, session_id: &TerminalSessionId) -> Option<DurableTerminalPlacement> {
        self.state
            .lock()
            .expect("terminal placement store mutex poisoned")
            .placements
            .get(session_id.as_str())
            .cloned()
    }

    pub fn begin_open(
        &self,
        session_id: &TerminalSessionId,
        spawn_fingerprint: u64,
    ) -> Result<u64, TerminalPlacementStoreError> {
        self.mutate(|state| {
            let request_id = state.next_request_id;
            state.next_request_id = state.next_request_id.saturating_add(1);
            state.placements.insert(
                session_id.as_str().to_string(),
                DurableTerminalPlacement::OpenPending {
                    request_id,
                    spawn_fingerprint,
                },
            );
            request_id
        })
    }

    pub fn bind(
        &self,
        host_id: HostId,
        host_epoch: u64,
        session_id: TerminalSessionId,
        session_epoch: u64,
        spawn_fingerprint: u64,
    ) -> Result<(), TerminalPlacementStoreError> {
        self.mutate(|state| {
            state.placements.insert(
                session_id.as_str().to_string(),
                DurableTerminalPlacement::Bound {
                    host_id,
                    host_epoch,
                    session_id,
                    session_epoch,
                    spawn_fingerprint,
                },
            );
        })
    }

    pub fn begin_close(
        &self,
        session_id: &TerminalSessionId,
    ) -> Result<Option<u64>, TerminalPlacementStoreError> {
        self.mutate(|state| {
            let (host_id, host_epoch, bound_session_id, session_epoch, spawn_fingerprint) =
                match state.placements.get(session_id.as_str()).cloned() {
                    Some(DurableTerminalPlacement::Bound {
                        host_id,
                        host_epoch,
                        session_id,
                        session_epoch,
                        spawn_fingerprint,
                    }) => (
                        host_id,
                        host_epoch,
                        session_id,
                        session_epoch,
                        spawn_fingerprint,
                    ),
                    Some(DurableTerminalPlacement::ClosePending { request_id, .. }) => {
                        return Err(TerminalPlacementStoreError::CloseAlreadyPending {
                            session_id: session_id.clone(),
                            request_id,
                        });
                    }
                    Some(DurableTerminalPlacement::Closed) => return Ok(None),
                    _ => return Err(TerminalPlacementStoreError::NotBound(session_id.clone())),
                };
            let request_id = state.next_request_id;
            state.next_request_id = state.next_request_id.saturating_add(1);
            state.placements.insert(
                session_id.as_str().to_string(),
                DurableTerminalPlacement::ClosePending {
                    host_id,
                    host_epoch,
                    session_id: bound_session_id,
                    session_epoch,
                    spawn_fingerprint,
                    request_id,
                },
            );
            Ok(Some(request_id))
        })?
    }

    pub fn finish_close(
        &self,
        session_id: &TerminalSessionId,
        request_id: u64,
        succeeded: bool,
    ) -> Result<(), TerminalPlacementStoreError> {
        self.mutate(|state| {
            let Some(DurableTerminalPlacement::ClosePending {
                host_id,
                host_epoch,
                session_id: bound_session_id,
                session_epoch,
                spawn_fingerprint,
                request_id: pending_request_id,
            }) = state.placements.get(session_id.as_str()).cloned()
            else {
                return Err(TerminalPlacementStoreError::CloseRequestMismatch {
                    session_id: session_id.clone(),
                    request_id,
                });
            };
            if pending_request_id != request_id {
                return Err(TerminalPlacementStoreError::CloseRequestMismatch {
                    session_id: session_id.clone(),
                    request_id,
                });
            }
            let next = if succeeded {
                DurableTerminalPlacement::Closed
            } else {
                DurableTerminalPlacement::Bound {
                    host_id,
                    host_epoch,
                    session_id: bound_session_id,
                    session_epoch,
                    spawn_fingerprint,
                }
            };
            state
                .placements
                .insert(session_id.as_str().to_string(), next);
            Ok(())
        })?
    }

    pub fn mark_closed(
        &self,
        session_id: &TerminalSessionId,
    ) -> Result<(), TerminalPlacementStoreError> {
        self.mutate(|state| {
            state.placements.insert(
                session_id.as_str().to_string(),
                DurableTerminalPlacement::Closed,
            );
        })
    }

    fn mutate<T>(
        &self,
        mutation: impl FnOnce(&mut TerminalPlacementFile) -> T,
    ) -> Result<T, TerminalPlacementStoreError> {
        let mut state = self
            .state
            .lock()
            .expect("terminal placement store mutex poisoned");
        let mut next = state.clone();
        let result = mutation(&mut next);
        let bytes =
            serde_json::to_vec_pretty(&next).map_err(TerminalPlacementStoreError::Encode)?;
        self.storage.write(&self.path, &bytes).map_err(|source| {
            TerminalPlacementStoreError::Write {
                path: self.path.clone(),
                source,
            }
        })?;
        *state = next;
        Ok(result)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum TerminalPlacementStoreError {
    #[error("failed to read terminal placement state at {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to parse terminal placement state at {path}: {source}")]
    Parse {
        path: PathBuf,
        source: serde_json::Error,
    },
    #[error("terminal placement state at {path} has unsupported version {actual}")]
    Version { path: PathBuf, actual: u32 },
    #[error("failed to encode terminal placement state: {0}")]
    Encode(serde_json::Error),
    #[error("failed to write terminal placement state at {path}: {source}")]
    Write {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("terminal session {0} does not have a bound placement")]
    NotBound(TerminalSessionId),
    #[error("terminal session {session_id} already has close request {request_id} pending")]
    CloseAlreadyPending {
        session_id: TerminalSessionId,
        request_id: u64,
    },
    #[error("terminal session {session_id} does not have matching close request {request_id}")]
    CloseRequestMismatch {
        session_id: TerminalSessionId,
        request_id: u64,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_bound_close_pending_and_closed_round_trip() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("terminal-placements.json");
        let session_id = TerminalSessionId::new("project:tab:pane");
        let store = TerminalPlacementStore::load_local(&path).unwrap();
        let open_request = store.begin_open(&session_id, 41).unwrap();
        assert_eq!(open_request, 1);
        store
            .bind(HostId::new("host"), 7, session_id.clone(), 3, 41)
            .unwrap();
        let close_request = store.begin_close(&session_id).unwrap();
        assert_eq!(close_request, Some(2));
        assert!(matches!(
            store.placement(&session_id),
            Some(DurableTerminalPlacement::ClosePending { request_id: 2, .. })
        ));
        store.mark_closed(&session_id).unwrap();

        let reloaded = TerminalPlacementStore::load_local(path).unwrap();
        assert_eq!(
            reloaded.placement(&session_id),
            Some(DurableTerminalPlacement::Closed)
        );
    }

    #[test]
    fn duplicate_close_is_rejected_and_failure_restores_bound() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("terminal-placements.json");
        let session_id = TerminalSessionId::new("project:tab:pane");
        let store = TerminalPlacementStore::load_local(path).unwrap();
        store
            .bind(HostId::new("host"), 7, session_id.clone(), 3, 41)
            .unwrap();

        let request_id = store.begin_close(&session_id).unwrap().unwrap();
        assert!(matches!(
            store.begin_close(&session_id),
            Err(TerminalPlacementStoreError::CloseAlreadyPending {
                request_id: pending_request_id,
                ..
            }) if pending_request_id == request_id
        ));
        store.finish_close(&session_id, request_id, false).unwrap();
        assert_eq!(
            store.placement(&session_id),
            Some(DurableTerminalPlacement::Bound {
                host_id: HostId::new("host"),
                host_epoch: 7,
                session_id,
                session_epoch: 3,
                spawn_fingerprint: 41,
            })
        );
    }

    #[test]
    fn closing_an_already_closed_terminal_is_idempotent() {
        let temp = tempfile::tempdir().unwrap();
        let session_id = TerminalSessionId::new("project:agent:omp");
        let store =
            TerminalPlacementStore::load_local(temp.path().join("terminal-placements.json"))
                .unwrap();
        store.mark_closed(&session_id).unwrap();

        assert_eq!(
            store.begin_close(&session_id).unwrap(),
            None,
            "an exited manual-restart Agent pane must not block its tab from closing"
        );
    }
}
