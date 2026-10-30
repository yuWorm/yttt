use parking_lot::Mutex;
use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Weak},
};
use yttt_protocol::workspace::{WorkspaceId, WorkspaceSummary};

/// Whether a window still has document edits the Host has not stored.
#[derive(Default)]
struct DocumentSync {
    unsynced: bool,
    error: Option<String>,
}

struct State {
    available: VecDeque<WorkspaceId>,
    views: HashMap<WorkspaceId, DocumentSync>,
    history_initialized: bool,
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

    /// Records whether this window has edits the Host has not stored yet,
    /// and the last upload failure.
    pub fn report_documents(&self, unsynced: bool, error: Option<String>) {
        if let Some(coordinator) = self.coordinator.upgrade()
            && let Some(sync) = coordinator.state.lock().views.get_mut(&self.id)
        {
            *sync = DocumentSync { unsynced, error };
        }
    }
}

impl Drop for WorkspaceViewLease {
    fn drop(&mut self) {
        if let Some(coordinator) = self.coordinator.upgrade() {
            coordinator.state.lock().views.remove(&self.id);
        }
    }
}

impl SessionCoordinator {
    pub fn new(workspaces: Vec<WorkspaceSummary>) -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(State {
                history_initialized: !workspaces.is_empty(),
                available: workspaces
                    .into_iter()
                    .filter(|entry| entry.restore_on_startup)
                    .map(|entry| entry.workspace_id)
                    .collect(),
                views: HashMap::new(),
            }),
        })
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
        state.views.insert(id.clone(), DocumentSync::default());
        state.history_initialized = true;
        Ok(WorkspaceViewLease {
            id,
            coordinator: Arc::downgrade(self),
        })
    }

    pub fn pending_workspace_count(&self) -> usize {
        self.state.lock().available.len()
    }

    pub fn is_open(&self, id: &WorkspaceId) -> bool {
        self.state.lock().views.contains_key(id)
    }

    pub(crate) fn has_workspace_history(&self) -> bool {
        self.state.lock().history_initialized
    }

    pub(crate) fn remove_pending_workspace(&self, id: &WorkspaceId) {
        self.state.lock().available.retain(|entry| entry != id);
    }

    /// `Ok(true)` once every window's document edits are stored on the Host;
    /// `Err` with the first upload failure.
    pub fn documents_synced(&self) -> Result<bool, String> {
        let state = self.state.lock();
        if let Some(error) = state.views.values().find_map(|sync| sync.error.clone()) {
            return Err(error);
        }
        Ok(state.views.values().all(|sync| !sync.unsynced))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closed_history_requires_explicit_selection_and_never_reuses_an_open_view() {
        let saved = WorkspaceId::new("saved-window").unwrap();
        let coordinator = Arc::new(SessionCoordinator {
            state: Mutex::new(State {
                available: VecDeque::from([saved.clone()]),
                views: HashMap::new(),
                history_initialized: true,
            }),
        });
        let first = coordinator.claim(None, true).unwrap();
        let other = coordinator.claim(None, false).unwrap();
        drop(first);
        assert_eq!(coordinator.pending_workspace_count(), 0);
        let restored = coordinator.claim(Some(saved.clone()), false).unwrap();
        assert_eq!(restored.id(), &saved);
        assert_ne!(restored.id(), other.id());
        assert!(coordinator.claim(Some(saved), false).is_err());
    }
}
