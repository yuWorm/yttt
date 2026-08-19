use gpui::Global;
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use yttt_client_core::{
    ClientCore, ClientCoreError, ClientEvent, ConnectionState, TerminalMirrorMetadata,
};
use yttt_core::model::ids::{ClientInstanceId, TerminalSessionId};
use yttt_protocol::{
    LifecycleRequest, LifecycleResponse, Request, ResourceCatalog, Response, TerminalPlacement,
    TerminalTerminationResult,
    terminal::{
        AttachTerminal, TerminalInput, TerminalSpawnSpec, TerminalStreamUpdate,
        TerminateTerminalRequest,
    },
};

use crate::{
    config::{
        profile::AppProfile,
        terminal_placements::{
            DurableTerminalPlacement, TerminalPlacementStore, TerminalPlacementStoreError,
        },
    },
    host_launcher::{HostLaunchError, HostLauncher, ManagedHostProcess},
};

#[derive(Clone, Debug)]
pub enum TerminalPaneHostEvent {
    Metadata(TerminalMirrorMetadata),
    Client(ClientEvent),
}

#[derive(PartialEq, Eq)]
struct TerminalPaneMetadataKey {
    title: Option<String>,
    process_state: yttt_protocol::terminal::TerminalProcessState,
}

impl From<&TerminalMirrorMetadata> for TerminalPaneMetadataKey {
    fn from(metadata: &TerminalMirrorMetadata) -> Self {
        Self {
            title: metadata.title.clone(),
            process_state: metadata.process_state,
        }
    }
}

fn terminal_client_event_matches(event: &ClientEvent, session_id: &TerminalSessionId) -> bool {
    match event {
        ClientEvent::TerminalUnavailable(unavailable) => unavailable == session_id,
        ClientEvent::Connection(ConnectionState::HostLost { .. }) => true,
        ClientEvent::Server(event) => match &event.body {
            yttt_protocol::ServerEvent::TerminalExit {
                session_id: event_session_id,
                ..
            }
            | yttt_protocol::ServerEvent::TerminalLeaseRevoked {
                session_id: event_session_id,
                ..
            }
            | yttt_protocol::ServerEvent::TerminalLeaseReleased {
                session_id: event_session_id,
                ..
            }
            | yttt_protocol::ServerEvent::TerminalLeaseExpired {
                session_id: event_session_id,
                ..
            }
            | yttt_protocol::ServerEvent::TerminalControlRequested {
                session_id: event_session_id,
                ..
            }
            | yttt_protocol::ServerEvent::TerminalControlDenied {
                session_id: event_session_id,
                ..
            } => event_session_id == session_id,
            yttt_protocol::ServerEvent::TerminalControlGranted { lease } => {
                &lease.session_id == session_id
            }
            _ => false,
        },
        _ => false,
    }
}
const TERMINAL_UPDATE_QUEUE_CAPACITY: usize = 64;
const TERMINAL_PANE_EVENT_QUEUE_CAPACITY: usize = 16;

pub struct DesktopHostRuntime {
    client: Arc<ClientCore>,
    runtime: tokio::runtime::Runtime,
    _managed_process: Arc<Mutex<ManagedHostProcess>>,
    launcher: HostLauncher,
    placement_store: Arc<TerminalPlacementStore>,
}

impl DesktopHostRuntime {
    pub fn start(profile: AppProfile) -> Result<Arc<Self>, DesktopHostRuntimeError> {
        let launcher = HostLauncher::for_current_desktop(profile.clone())?;
        Self::start_with_launcher(profile, launcher)
    }

    pub fn start_with_executable(
        profile: AppProfile,
        executable: impl Into<std::path::PathBuf>,
    ) -> Result<Arc<Self>, DesktopHostRuntimeError> {
        let launcher = HostLauncher::desktop_owned(profile.clone(), executable);
        Self::start_with_launcher(profile, launcher)
    }

    fn start_with_launcher(
        profile: AppProfile,
        launcher: HostLauncher,
    ) -> Result<Arc<Self>, DesktopHostRuntimeError> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .thread_name("yttt-client-core")
            .build()?;
        let placement_store = Arc::new(TerminalPlacementStore::load(
            profile.config_paths().terminal_placements_file(),
        )?);
        let managed_process = runtime.block_on(launcher.launch_or_attach())?;
        let launcher = managed_process.recovery_launcher();
        let managed_process = Arc::new(Mutex::new(managed_process));
        let (connector, identity, token) = launcher.client_core_config(ClientInstanceId::new(
            format!("desktop-{}", uuid::Uuid::new_v4()),
        ))?;
        let client = Arc::new(runtime.block_on(ClientCore::connect(connector, identity, token))?);
        let mut state = client.subscribe_state();
        let recovery_launcher = launcher.clone();
        let recovered_process = managed_process.clone();
        runtime.spawn(async move {
            while state.changed().await.is_ok() {
                if matches!(&*state.borrow(), ConnectionState::Reconnecting { .. })
                    && let Ok(process) = recovery_launcher.launch_or_attach().await
                    && process.spawned()
                {
                    *recovered_process.lock().unwrap() = process;
                }
            }
        });
        Ok(Arc::new(Self {
            client,
            runtime,
            _managed_process: managed_process,
            launcher,
            placement_store,
        }))
    }

    pub fn shutdown_client(&self) {
        self.runtime.block_on(self.client.shutdown());
        self._managed_process
            .lock()
            .unwrap()
            .release_desktop_owner();
    }

    pub fn state(&self) -> ConnectionState {
        self.client.state()
    }

    pub fn request(&self, request: Request) -> flume::Receiver<Result<Response, ClientCoreError>> {
        let (sender, receiver) = flume::bounded(1);
        match self.client.enqueue_request(request) {
            Ok(pending) => {
                self.runtime.spawn(async move {
                    let _ = sender.send_async(pending.wait().await).await;
                });
            }
            Err(error) => {
                let _ = sender.send(Err(error));
            }
        }
        receiver
    }

    pub fn request_lifecycle(
        &self,
        request: LifecycleRequest,
        can_force_stop: bool,
    ) -> flume::Receiver<Result<LifecycleResponse, HostLaunchError>> {
        let (sender, receiver) = flume::bounded(1);
        let launcher = self.launcher.clone();
        self.runtime.spawn(async move {
            let result = async {
                let mut client = launcher.connect_lifecycle(can_force_stop).await?;
                client.request(request).await
            }
            .await;
            let _ = sender.send_async(result).await;
        });
        receiver
    }

    pub fn request_detached(&self, request: Request) -> Result<(), ClientCoreError> {
        let pending = self.client.enqueue_request(request)?;
        self.runtime.spawn(async move {
            if let Err(error) = pending.wait().await {
                eprintln!("detached Host request failed: {error}");
            }
        });
        Ok(())
    }

    pub fn send_terminal_input(&self, input: TerminalInput) -> Result<(), ClientCoreError> {
        self.client.send_terminal_input(input)
    }

    pub fn request_blocking_typed(&self, request: Request) -> Result<Response, ClientCoreError> {
        let timeout = match &request {
            Request::RemoteFile(_) | Request::RemoteCommand(_) | Request::Project(_) => {
                Duration::from_secs(125)
            }
            _ => Duration::from_secs(15),
        };
        self.request(request)
            .recv_timeout(timeout)
            .map_err(|error| match error {
                flume::RecvTimeoutError::Timeout => ClientCoreError::RequestTimeout,
                flume::RecvTimeoutError::Disconnected => ClientCoreError::SupervisorStopped,
            })?
    }

    pub fn request_blocking(&self, request: Request) -> Result<Response, String> {
        self.request_blocking_typed(request)
            .map_err(|error| error.to_string())
    }
    pub fn terminal_updates(
        &self,
        session_id: TerminalSessionId,
    ) -> flume::Receiver<Arc<TerminalStreamUpdate>> {
        let (sender, receiver) = flume::bounded(TERMINAL_UPDATE_QUEUE_CAPACITY);
        let mut events = self.client.subscribe_events();
        let client = self.client.clone();
        self.runtime.spawn(async move {
            loop {
                let update = match events.recv().await {
                    Ok(ClientEvent::TerminalUpdated(update))
                        if update.session_id() == &session_id =>
                    {
                        update
                    }
                    Ok(_) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        let Some(viewport) = client.terminal_snapshot(&session_id) else {
                            continue;
                        };
                        Arc::new(TerminalStreamUpdate::Snapshot(viewport))
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                };
                if sender.send_async(update).await.is_err() {
                    break;
                }
            }
        });
        receiver
    }

    pub fn terminal_pane_events(
        &self,
        session_id: TerminalSessionId,
    ) -> flume::Receiver<TerminalPaneHostEvent> {
        let (sender, receiver) = flume::bounded(TERMINAL_PANE_EVENT_QUEUE_CAPACITY);
        let mut events = self.client.subscribe_events();
        let client = self.client.clone();
        self.runtime.spawn(async move {
            let mut last_metadata = None::<TerminalPaneMetadataKey>;
            loop {
                let refresh_metadata = match events.recv().await {
                    Ok(ClientEvent::TerminalUpdated(update))
                        if update.session_id() == &session_id =>
                    {
                        true
                    }
                    Ok(event) if terminal_client_event_matches(&event, &session_id) => {
                        if sender
                            .send_async(TerminalPaneHostEvent::Client(event))
                            .await
                            .is_err()
                        {
                            break;
                        }
                        false
                    }
                    Ok(_) => false,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => true,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                };
                if !refresh_metadata {
                    continue;
                }
                let Some(metadata) = client.terminal_metadata(&session_id) else {
                    continue;
                };
                let metadata_key = TerminalPaneMetadataKey::from(&metadata);
                if last_metadata.as_ref() == Some(&metadata_key) {
                    continue;
                }
                last_metadata = Some(metadata_key);
                if sender
                    .send_async(TerminalPaneHostEvent::Metadata(metadata))
                    .await
                    .is_err()
                {
                    break;
                }
            }
        });
        receiver
    }

    pub fn events(&self) -> flume::Receiver<ClientEvent> {
        let (sender, receiver) = flume::bounded(256);
        let mut events = self.client.subscribe_events();
        self.runtime.spawn(async move {
            loop {
                match events.recv().await {
                    Ok(event) => {
                        if sender.send_async(event).await.is_err() {
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        });
        receiver
    }

    pub fn terminal_metadata(
        &self,
        session_id: &TerminalSessionId,
    ) -> Option<TerminalMirrorMetadata> {
        self.client.terminal_metadata(session_id)
    }

    pub fn resource_catalog(&self) -> Option<Arc<ResourceCatalog>> {
        self.client.resource_catalog()
    }
    pub fn agent_snapshots(&self) -> Vec<yttt_protocol::agent::AgentSnapshotUpdate> {
        self.client.agent_snapshots()
    }

    pub fn terminal_start_request(
        &self,
        spec: TerminalSpawnSpec,
        catalog: &ResourceCatalog,
    ) -> Result<Request, TerminalRecoveryError> {
        reconcile_terminal_start(&self.placement_store, spec, catalog)
    }

    pub fn bind_terminal(
        &self,
        catalog: &ResourceCatalog,
        session_id: TerminalSessionId,
        session_epoch: u64,
        spawn_fingerprint: u64,
    ) -> Result<(), String> {
        self.placement_store
            .bind(
                catalog.host_id.clone(),
                catalog.host_epoch,
                session_id,
                session_epoch,
                spawn_fingerprint,
            )
            .map_err(|error| error.to_string())
    }
    pub fn acknowledge_terminal_exit(
        &self,
        session_id: TerminalSessionId,
        session_epoch: u64,
        final_sequence: u64,
    ) -> Result<(), ClientCoreError> {
        let pending = self
            .client
            .enqueue_request(Request::AcknowledgeTerminalExit {
                session_id: session_id.clone(),
                session_epoch,
                final_sequence,
            })?;
        let placement_store = self.placement_store.clone();
        self.runtime.spawn(async move {
            match pending.wait().await {
                Ok(Response::TerminalExitAcknowledged) => {
                    if let Err(error) = placement_store.mark_closed(&session_id) {
                        eprintln!("failed to close acknowledged terminal placement: {error}");
                    }
                }
                Ok(response) => {
                    eprintln!("unexpected terminal exit acknowledgement response: {response:?}");
                }
                Err(error) => {
                    eprintln!("terminal exit acknowledgement failed: {error}");
                }
            }
        });
        Ok(())
    }

    pub fn terminate_many_confirmed(
        &self,
        session_ids: Vec<TerminalSessionId>,
    ) -> Result<Vec<TerminalTerminationResult>, String> {
        let mut requests = Vec::with_capacity(session_ids.len());
        for session_id in session_ids {
            match self.placement_store.begin_close(&session_id) {
                Ok(request_id) => requests.push(TerminateTerminalRequest {
                    request_id,
                    session_id,
                }),
                Err(error) => {
                    for request in &requests {
                        let _ = self.placement_store.finish_close(
                            &request.session_id,
                            request.request_id,
                            false,
                        );
                    }
                    return Err(error.to_string());
                }
            }
        }
        let response = match self.request_blocking(Request::TerminateMany {
            requests: requests.clone(),
        }) {
            Ok(response) => response,
            Err(error) => {
                for request in &requests {
                    let _ = self.placement_store.finish_close(
                        &request.session_id,
                        request.request_id,
                        false,
                    );
                }
                return Err(error);
            }
        };
        let Response::TerminalsTerminated { results } = response else {
            for request in &requests {
                let _ = self.placement_store.finish_close(
                    &request.session_id,
                    request.request_id,
                    false,
                );
            }
            return Err(format!(
                "unexpected Host terminate-many response: {response:?}"
            ));
        };
        if results.len() != requests.len()
            || requests.iter().any(|request| {
                !results.iter().any(|result| {
                    result.request_id == request.request_id
                        && result.session_id == request.session_id
                })
            })
        {
            for request in &requests {
                let _ = self.placement_store.finish_close(
                    &request.session_id,
                    request.request_id,
                    false,
                );
            }
            return Err("Host returned an incomplete terminate-many result".to_string());
        }
        for result in &results {
            self.placement_store
                .finish_close(&result.session_id, result.request_id, result.result.is_ok())
                .map_err(|error| error.to_string())?;
        }
        Ok(results)
    }
    pub fn durable_terminal_placement(
        &self,
        session_id: &TerminalSessionId,
    ) -> Option<DurableTerminalPlacement> {
        self.placement_store.placement(session_id)
    }

    pub fn recovered_terminals(&self, catalog: &ResourceCatalog) -> Vec<RecoveredTerminal> {
        catalog
            .terminals
            .iter()
            .cloned()
            .map(|placement| RecoveredTerminal {
                durable: self.placement_store.placement(&placement.session_id),
                placement,
            })
            .collect()
    }
}

#[derive(Clone, Debug)]
pub struct RecoveredTerminal {
    pub placement: TerminalPlacement,
    pub durable: Option<DurableTerminalPlacement>,
}

#[derive(Debug, thiserror::Error)]
pub enum TerminalRecoveryError {
    #[error(
        "terminal placement address conflicts with Host session {session_id}: expected fingerprint {expected:#x}, received {actual:#x}"
    )]
    AddressConflict {
        session_id: TerminalSessionId,
        expected: u64,
        actual: u64,
    },
    #[error("terminal placement address conflicts with pending session {0}")]
    PendingAddressConflict(TerminalSessionId),
    #[error("terminal session {0} has an unfinished close request")]
    ClosePending(TerminalSessionId),
    #[error(transparent)]
    Persistence(#[from] TerminalPlacementStoreError),
}

fn reconcile_terminal_start(
    store: &TerminalPlacementStore,
    spec: TerminalSpawnSpec,
    catalog: &ResourceCatalog,
) -> Result<Request, TerminalRecoveryError> {
    let durable = store.placement(&spec.session_id);
    if let Some(placement) = catalog
        .terminals
        .iter()
        .find(|placement| placement.session_id == spec.session_id)
    {
        let actual = spec.address_fingerprint();
        if placement.spawn_fingerprint != actual {
            return Err(TerminalRecoveryError::AddressConflict {
                session_id: placement.session_id.clone(),
                expected: placement.spawn_fingerprint,
                actual,
            });
        }
        let close_pending_on_current_host = matches!(
            &durable,
            Some(DurableTerminalPlacement::ClosePending {
                host_id,
                host_epoch,
                ..
            }) if host_id == &catalog.host_id && *host_epoch == catalog.host_epoch
        );
        if close_pending_on_current_host {
            return Err(TerminalRecoveryError::ClosePending(
                placement.session_id.clone(),
            ));
        }
        let geometry_epoch = placement.geometry_epoch.saturating_add(1).max(1);
        return Ok(Request::AttachTerminal(AttachTerminal {
            session_id: placement.session_id.clone(),
            known_session_epoch: Some(placement.session_epoch),
            after_sequence: (placement.last_sequence > 0).then_some(placement.last_sequence),
            mode: yttt_protocol::terminal::TerminalLeaseMode::Interactive,
            geometry: spec.geometry,
            geometry_epoch,
            query_palette: spec.query_palette.clone(),
            palette_revision: spec.palette_revision,
        }));
    }

    if let Some(DurableTerminalPlacement::OpenPending {
        spawn_fingerprint, ..
    }) = &durable
        && *spawn_fingerprint != spec.address_fingerprint()
    {
        return Err(TerminalRecoveryError::PendingAddressConflict(
            spec.session_id,
        ));
    }

    store.begin_open(&spec.session_id, spec.address_fingerprint())?;
    Ok(Request::SpawnTerminal(spec))
}

#[derive(Clone)]
pub struct HostRuntimeGlobal {
    runtime: Option<Arc<DesktopHostRuntime>>,
    error: Option<Arc<str>>,
}

impl HostRuntimeGlobal {
    pub fn start(profile: AppProfile) -> Self {
        match DesktopHostRuntime::start(profile) {
            Ok(runtime) => Self::ready(runtime),
            Err(error) => Self::unavailable(format!("Host runtime unavailable: {error}")),
        }
    }

    pub fn ready(runtime: Arc<DesktopHostRuntime>) -> Self {
        Self {
            runtime: Some(runtime),
            error: None,
        }
    }
    pub fn disabled() -> Self {
        Self {
            runtime: None,
            error: None,
        }
    }

    pub fn unavailable(error: impl Into<Arc<str>>) -> Self {
        Self {
            runtime: None,
            error: Some(error.into()),
        }
    }

    pub fn runtime(&self) -> Option<&Arc<DesktopHostRuntime>> {
        self.runtime.as_ref()
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
}

impl Global for HostRuntimeGlobal {}

#[derive(Debug, thiserror::Error)]
pub enum DesktopHostRuntimeError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Launch(#[from] HostLaunchError),
    #[error(transparent)]
    Client(#[from] ClientCoreError),
    #[error(transparent)]
    Placement(#[from] TerminalPlacementStoreError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::profile::{
        EnvironmentKind, HostConnectPolicy, ProfilePersistence, ProjectConfigPolicy,
    };
    use yttt_core::model::ids::{HostId, ProfileId, ProjectId};
    use yttt_protocol::terminal::{TerminalExecutionSpec, TerminalGeometry, TerminalProcessState};

    fn spec() -> TerminalSpawnSpec {
        TerminalSpawnSpec {
            session_id: TerminalSessionId::new("project:tab:pane"),
            project_id: ProjectId::new("project"),
            cwd: yttt_protocol::ProjectRelativePath::root(),
            execution: TerminalExecutionSpec::Shell {
                program: "/bin/sh".to_string(),
                args: Vec::new(),
                initial_command: None,
            },
            geometry: TerminalGeometry {
                cols: 80,
                rows: 24,
                cell_width: 8,
                cell_height: 16,
            },
            query_palette: Vec::new(),
            palette_revision: 1,
            geometry_epoch: 1,
            scrollback_limit: 1_000,
            environment: Vec::new(),
            removed_environment: Vec::new(),
        }
    }

    fn catalog(terminals: Vec<TerminalPlacement>) -> ResourceCatalog {
        ResourceCatalog {
            profile_id: ProfileId::new("profile"),
            host_id: HostId::new("host"),
            host_epoch: 7,
            revision: 1,
            terminals,
            ssh_connections: Vec::new(),
            projects: Vec::new(),
        }
    }

    #[test]
    fn startup_reconciles_open_bound_and_missing_placements() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("terminal-placements.json");
        let store = TerminalPlacementStore::load(&path).unwrap();
        let spec = spec();
        assert!(matches!(
            reconcile_terminal_start(&store, spec.clone(), &catalog(Vec::new())).unwrap(),
            Request::SpawnTerminal(_)
        ));
        assert!(matches!(
            store.placement(&spec.session_id),
            Some(DurableTerminalPlacement::OpenPending { .. })
        ));

        store
            .bind(
                HostId::new("host"),
                7,
                spec.session_id.clone(),
                3,
                spec.address_fingerprint(),
            )
            .unwrap();
        let placement = TerminalPlacement {
            session_id: spec.session_id.clone(),
            session_epoch: 3,
            geometry_epoch: spec.geometry_epoch,
            project_id: spec.project_id.clone(),
            geometry: spec.geometry,
            last_sequence: 11,
            spawn_fingerprint: spec.address_fingerprint(),
            owner: None,
            process_state: TerminalProcessState::Running,
        };
        assert!(matches!(
            reconcile_terminal_start(&store, spec.clone(), &catalog(vec![placement])).unwrap(),
            Request::AttachTerminal(AttachTerminal {
                known_session_epoch: Some(3),
                ..
            })
        ));

        assert!(matches!(
            reconcile_terminal_start(&store, spec.clone(), &catalog(Vec::new())).unwrap(),
            Request::SpawnTerminal(_)
        ));
        assert!(matches!(
            store.placement(&spec.session_id),
            Some(DurableTerminalPlacement::OpenPending { .. })
        ));

        let reloaded = TerminalPlacementStore::load(path).unwrap();
        assert!(matches!(
            reloaded.placement(&spec.session_id),
            Some(DurableTerminalPlacement::OpenPending { .. })
        ));
    }

    #[test]
    fn recovery_reports_address_pending_and_close_conflicts() {
        let temp = tempfile::tempdir().unwrap();
        let requested = spec();
        let conflicting_placement = TerminalPlacement {
            session_id: requested.session_id.clone(),
            session_epoch: 3,
            project_id: requested.project_id.clone(),
            geometry_epoch: requested.geometry_epoch,
            geometry: requested.geometry,
            last_sequence: 11,
            spawn_fingerprint: requested.address_fingerprint().wrapping_add(1),
            owner: None,
            process_state: TerminalProcessState::Running,
        };
        let address_store = TerminalPlacementStore::load(temp.path().join("address.json")).unwrap();
        assert!(matches!(
            reconcile_terminal_start(
                &address_store,
                requested.clone(),
                &catalog(vec![conflicting_placement])
            ),
            Err(TerminalRecoveryError::AddressConflict { .. })
        ));

        let pending_store = TerminalPlacementStore::load(temp.path().join("pending.json")).unwrap();
        pending_store
            .begin_open(
                &requested.session_id,
                requested.address_fingerprint().wrapping_add(1),
            )
            .unwrap();
        assert!(matches!(
            reconcile_terminal_start(&pending_store, requested.clone(), &catalog(Vec::new())),
            Err(TerminalRecoveryError::PendingAddressConflict(_))
        ));

        let close_store = TerminalPlacementStore::load(temp.path().join("close.json")).unwrap();
        close_store
            .bind(
                HostId::new("host"),
                7,
                requested.session_id.clone(),
                3,
                requested.address_fingerprint(),
            )
            .unwrap();
        close_store.begin_close(&requested.session_id).unwrap();
        let live_placement = TerminalPlacement {
            session_id: requested.session_id.clone(),
            session_epoch: 3,
            project_id: requested.project_id.clone(),
            geometry: requested.geometry,
            geometry_epoch: requested.geometry_epoch,
            last_sequence: 11,
            spawn_fingerprint: requested.address_fingerprint(),
            owner: None,
            process_state: TerminalProcessState::Running,
        };
        assert!(matches!(
            reconcile_terminal_start(&close_store, requested, &catalog(vec![live_placement])),
            Err(TerminalRecoveryError::ClosePending(_))
        ));
    }

    #[test]
    fn host_start_failure_becomes_an_unavailable_recovery_state() {
        let temp = tempfile::tempdir().unwrap();
        let occupied_root = temp.path().join("not-a-directory");
        std::fs::write(&occupied_root, b"occupied").unwrap();
        let profile = AppProfile::scoped(
            ProfileId::new("unavailable-host"),
            EnvironmentKind::Test,
            ProfilePersistence::Ephemeral,
            &occupied_root,
            ProjectConfigPolicy::Overlay,
            HostConnectPolicy::ExplicitEndpoint(occupied_root.join("runtime/host.sock")),
        );

        let status = HostRuntimeGlobal::start(profile);

        assert!(status.runtime().is_none());
        assert!(
            status
                .error()
                .is_some_and(|error| error.starts_with("Host runtime unavailable:"))
        );
    }

    #[test]
    fn lost_placement_from_previous_host_starts_a_fresh_session() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("terminal-placements.json");
        let requested = spec();
        std::fs::write(
            &path,
            serde_json::to_vec_pretty(&serde_json::json!({
                "version": 1,
                "next_request_id": 2,
                "placements": {
                    (requested.session_id.as_str()): {
                        "state": "lost",
                        "host_id": "previous-host",
                        "host_epoch": 6,
                        "session_id": requested.session_id.as_str(),
                        "session_epoch": 3,
                        "spawn_fingerprint": requested.address_fingerprint(),
                        "reason": "previous desktop owner exited"
                    }
                }
            }))
            .unwrap(),
        )
        .unwrap();
        let store = TerminalPlacementStore::load(path).unwrap();

        assert!(matches!(
            reconcile_terminal_start(&store, requested.clone(), &catalog(Vec::new())).unwrap(),
            Request::SpawnTerminal(_)
        ));
        assert!(matches!(
            store.placement(&requested.session_id),
            Some(DurableTerminalPlacement::OpenPending { .. })
        ));
    }
}
