use parking_lot::Mutex;
use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Weak},
    time::Duration,
};
use yttt_client_core::ClientCore;
use yttt_protocol::{
    Request, Response,
    session::{ProfileControlRequest, TransferPhase, WorkspaceRevision},
    workspace::{WorkspaceId, WorkspaceRequest, WorkspaceResponse, WorkspaceSummary},
};

#[derive(Default)]
struct Publication {
    transfer_id: Option<String>,
    revision: Option<u64>,
    error: Option<String>,
}

struct State {
    available: VecDeque<WorkspaceId>,
    views: HashMap<WorkspaceId, Publication>,
    submitted_transfer: Option<String>,
    local_flush: Option<String>,
}

/// One instance per Client process/profile, shared by all work windows.
pub struct SessionCoordinator {
    state: Mutex<State>,
}

pub struct WorkspaceViewLease {
    id: WorkspaceId,
    coordinator: Weak<SessionCoordinator>,
}

impl WorkspaceViewLease {
    pub fn id(&self) -> &WorkspaceId {
        &self.id
    }
    pub fn published(
        &self,
        transfer_id: Option<String>,
        revision: Option<u64>,
        error: Option<String>,
    ) {
        if let Some(coordinator) = self.coordinator.upgrade()
            && let Some(publication) = coordinator.state.lock().views.get_mut(&self.id)
        {
            *publication = Publication {
                transfer_id,
                revision,
                error,
            };
        }
    }
}
impl Drop for WorkspaceViewLease {
    fn drop(&mut self) {
        if let Some(coordinator) = self.coordinator.upgrade() {
            let mut state = coordinator.state.lock();
            state.views.remove(&self.id);
            state.available.push_front(self.id.clone());
        }
    }
}

impl SessionCoordinator {
    pub fn start(
        client: Arc<ClientCore>,
        runtime: &tokio::runtime::Handle,
        workspaces: Vec<WorkspaceSummary>,
    ) -> Arc<Self> {
        let coordinator = Arc::new(Self {
            state: Mutex::new(State {
                available: workspaces
                    .into_iter()
                    .map(|entry| entry.workspace_id)
                    .collect(),
                views: HashMap::new(),
                submitted_transfer: None,
                local_flush: None,
            }),
        });
        let weak = Arc::downgrade(&coordinator);
        runtime.spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_millis(50));
            loop {
                interval.tick().await;
                let Some(coordinator) = weak.upgrade() else {
                    return;
                };
                let Some(status) = client.control_status() else {
                    continue;
                };
                let Some(transfer) = status.transfer.filter(|transfer| {
                    transfer.previous_owner.as_ref() == Some(client.client_id())
                        && transfer.phase == TransferPhase::Preparing
                }) else {
                    coordinator.state.lock().submitted_transfer = None;
                    continue;
                };
                let readiness = {
                    let mut state = coordinator.state.lock();
                    if state.submitted_transfer.as_deref() == Some(&transfer.id) {
                        continue;
                    }
                    let mut revisions = Vec::with_capacity(state.views.len());
                    let mut failure = None;
                    let mut pending = false;
                    for (id, publication) in &state.views {
                        if publication.transfer_id.as_deref() != Some(&transfer.id) {
                            pending = true;
                            break;
                        }
                        if let Some(error) = &publication.error {
                            failure = Some(error.clone());
                            break;
                        }
                        if let Some(revision) = publication.revision {
                            revisions.push(WorkspaceRevision {
                                workspace_id: id.clone(),
                                revision,
                            });
                        } else {
                            pending = true;
                            break;
                        }
                    }
                    if pending {
                        continue;
                    }
                    state.submitted_transfer = Some(transfer.id.clone());
                    failure.map_or(Ok(revisions), Err)
                };
                let result = async {
                    let revisions = readiness?;
                    let Response::Workspace(WorkspaceResponse::Workspaces(index)) = client
                        .request(Request::Workspace(WorkspaceRequest::List))
                        .await
                        .map_err(|error| error.to_string())?
                    else {
                        return Err("Host returned an unexpected workspace index".to_string());
                    };
                    let mut confirmed = index
                        .into_iter()
                        .map(|entry| WorkspaceRevision {
                            workspace_id: entry.workspace_id,
                            revision: entry.revision,
                        })
                        .collect::<Vec<_>>();
                    for revision in revisions {
                        let Some(entry) = confirmed
                            .iter_mut()
                            .find(|entry| entry.workspace_id == revision.workspace_id)
                        else {
                            return Err(
                                "published workspace disappeared from Host index".to_string()
                            );
                        };
                        if entry.revision != revision.revision {
                            return Err(
                                "workspace changed while preparing control transfer".to_string()
                            );
                        }
                    }
                    client
                        .request(Request::ProfileControl(ProfileControlRequest::Ready {
                            transfer_id: transfer.id.clone(),
                            revisions: confirmed,
                        }))
                        .await
                        .map_err(|error| error.to_string())?;
                    Ok::<_, String>(())
                }
                .await;
                if let Err(error) = result {
                    eprintln!("control transfer publication failed: {error}");
                    let _ = client
                        .request(Request::ProfileControl(ProfileControlRequest::Cancel {
                            transfer_id: transfer.id,
                        }))
                        .await;
                }
            }
        });
        coordinator
    }

    pub fn claim(
        self: &Arc<Self>,
        preferred: Option<WorkspaceId>,
        restore_existing: bool,
    ) -> Result<WorkspaceViewLease, String> {
        let mut state = self.state.lock();
        let id = if let Some(id) = preferred {
            if state.views.contains_key(&id) {
                return Err("This workspace already has an open view in this Client".to_string());
            }
            state.available.retain(|entry| entry != &id);
            id
        } else if restore_existing {
            state.available.pop_front().unwrap_or_else(|| {
                WorkspaceId::new(uuid::Uuid::new_v4().to_string()).expect("UUID is a workspace ID")
            })
        } else {
            WorkspaceId::new(uuid::Uuid::new_v4().to_string()).expect("UUID is a workspace ID")
        };
        state.views.insert(id.clone(), Publication::default());
        Ok(WorkspaceViewLease {
            id,
            coordinator: Arc::downgrade(self),
        })
    }

    pub fn pending_workspace_count(&self) -> usize {
        self.state.lock().available.len()
    }

    pub fn local_flush(&self) -> Option<String> {
        self.state.lock().local_flush.clone()
    }
    pub fn begin_local_flush(&self) -> String {
        let id = format!("exit-{}", uuid::Uuid::new_v4());
        self.state.lock().local_flush = Some(id.clone());
        id
    }
    pub fn end_local_flush(&self) {
        self.state.lock().local_flush = None;
    }
    pub fn local_flush_ready(&self, id: &str) -> Result<bool, String> {
        let state = self.state.lock();
        for publication in state.views.values() {
            if publication.transfer_id.as_deref() != Some(id) {
                return Ok(false);
            }
            if let Some(error) = &publication.error {
                return Err(error.clone());
            }
            if publication.revision.is_none() {
                return Ok(false);
            }
        }
        Ok(true)
    }

    pub fn sharing_ready(&self) -> bool {
        self.state
            .lock()
            .views
            .values()
            .all(|publication| publication.revision.is_some() && publication.error.is_none())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reopening_a_closed_view_restores_its_identity_without_reusing_an_open_view() {
        let saved = WorkspaceId::new("saved-window").unwrap();
        let coordinator = Arc::new(SessionCoordinator {
            state: Mutex::new(State {
                available: VecDeque::from([saved.clone()]),
                views: HashMap::new(),
                submitted_transfer: None,
                local_flush: None,
            }),
        });
        let first = coordinator.claim(None, true).unwrap();
        let other = coordinator.claim(None, false).unwrap();
        drop(first);
        let restored = coordinator.claim(None, true).unwrap();
        assert_eq!(restored.id(), &saved);
        assert_ne!(restored.id(), other.id());
    }
}
