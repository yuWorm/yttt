use gpui::Global;
use parking_lot::Mutex;
use std::{
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
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
    config::profile::AppProfile,
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
    lifecycle: DesktopHostLifecycle,
    next_terminal_request_id: AtomicU64,
    storage: Arc<crate::host_storage::HostStorage>,
    coordinator: Arc<crate::session_coordinator::SessionCoordinator>,
}

enum DesktopHostLifecycle {
    Local {
        managed_process: Arc<Mutex<ManagedHostProcess>>,
        launcher: HostLauncher,
    },
    Remote {
        connector: yttt_transport::SharedConnector,
        identity: yttt_transport::ClientIdentity,
        token: yttt_transport::AuthToken,
        environment: yttt_protocol::workspace::WorkspaceEnvironment,
        label: String,
    },
}

fn shared_editing_available(
    connection_state: &ConnectionState,
    is_controller: bool,
    transfer_in_progress: bool,
) -> bool {
    matches!(connection_state, ConnectionState::Ready { .. })
        && is_controller
        && !transfer_in_progress
}

impl DesktopHostRuntime {
    pub fn start(profile: AppProfile) -> Result<Arc<Self>, DesktopHostRuntimeError> {
        let launcher = HostLauncher::for_current_desktop(profile)?;
        Self::start_with_launcher(launcher)
    }

    pub fn start_with_executable(
        profile: AppProfile,
        executable: impl Into<std::path::PathBuf>,
    ) -> Result<Arc<Self>, DesktopHostRuntimeError> {
        let launcher = HostLauncher::desktop_owned(profile, executable);
        Self::start_with_launcher(launcher)
    }

    fn start_with_launcher(launcher: HostLauncher) -> Result<Arc<Self>, DesktopHostRuntimeError> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .thread_name("yttt-client-core")
            .build()?;
        let managed_process = runtime.block_on(launcher.launch_or_attach())?;
        let launcher = managed_process.recovery_launcher();
        let managed_process = Arc::new(Mutex::new(managed_process));
        let (connector, identity, token) = launcher.client_core_config(ClientInstanceId::new(
            format!("desktop-{}", uuid::Uuid::new_v4()),
        ))?;
        let client = Arc::new(runtime.block_on(ClientCore::connect(connector, identity, token))?);
        if client
            .control_status()
            .is_some_and(|status| status.owner.is_none())
        {
            runtime.block_on(client.request(Request::ProfileControl(
                yttt_protocol::session::ProfileControlRequest::RequestControl,
            )))?;
        }
        let environment = match runtime.block_on(client.request(Request::Workspace(
            yttt_protocol::workspace::WorkspaceRequest::Environment,
        )))? {
            Response::Workspace(yttt_protocol::workspace::WorkspaceResponse::Environment(
                environment,
            )) => environment,
            _ => {
                return Err(std::io::Error::other(
                    "Host did not provide its environment descriptor",
                )
                .into());
            }
        };
        let config_root = environment
            .config_root
            .to_path()
            .map_err(std::io::Error::other)?;
        let storage = Arc::new(crate::host_storage::HostStorage::new(
            client.clone(),
            runtime.handle().clone(),
            config_root,
            environment,
            false,
        ));
        let mut state = client.subscribe_state();
        let recovery_launcher = launcher.clone();
        let recovered_process = managed_process.clone();
        runtime.spawn(async move {
            while state.changed().await.is_ok() {
                if matches!(&*state.borrow(), ConnectionState::Reconnecting { .. })
                    && let Ok(process) = recovery_launcher.launch_or_attach().await
                    && process.spawned()
                {
                    *recovered_process.lock() = process;
                }
            }
        });
        let index = match runtime.block_on(client.request(Request::Workspace(
            yttt_protocol::workspace::WorkspaceRequest::List,
        )))? {
            Response::Workspace(yttt_protocol::workspace::WorkspaceResponse::Workspaces(index)) => {
                index
            }
            _ => {
                return Err(
                    std::io::Error::other("Host did not provide its workspace index").into(),
                );
            }
        };
        let coordinator = crate::session_coordinator::SessionCoordinator::start(
            client.clone(),
            runtime.handle(),
            index,
        );
        Ok(Arc::new(Self {
            client,
            runtime,
            lifecycle: DesktopHostLifecycle::Local {
                managed_process,
                launcher,
            },
            next_terminal_request_id: AtomicU64::new(1),
            storage,
            coordinator,
        }))
    }

    pub(crate) fn from_remote(
        runtime: tokio::runtime::Runtime,
        client: Arc<ClientCore>,
        storage: Arc<crate::host_storage::HostStorage>,
        connector: yttt_transport::SharedConnector,
        identity: yttt_transport::ClientIdentity,
        token: yttt_transport::AuthToken,
        environment: yttt_protocol::workspace::WorkspaceEnvironment,
        label: String,
    ) -> Result<Arc<Self>, DesktopHostRuntimeError> {
        let index = match runtime.block_on(client.request(Request::Workspace(
            yttt_protocol::workspace::WorkspaceRequest::List,
        )))? {
            Response::Workspace(yttt_protocol::workspace::WorkspaceResponse::Workspaces(index)) => {
                index
            }
            _ => {
                return Err(
                    std::io::Error::other("Host did not provide its workspace index").into(),
                );
            }
        };
        let coordinator = crate::session_coordinator::SessionCoordinator::start(
            client.clone(),
            runtime.handle(),
            index,
        );
        Ok(Arc::new(Self {
            client,
            runtime,
            lifecycle: DesktopHostLifecycle::Remote {
                connector,
                identity,
                token,
                environment,
                label,
            },
            next_terminal_request_id: AtomicU64::new(1),
            storage,
            coordinator,
        }))
    }

    pub fn is_remote(&self) -> bool {
        matches!(self.lifecycle, DesktopHostLifecycle::Remote { .. })
    }

    pub fn environment_storage(&self) -> Arc<dyn crate::config::storage::ConfigStorage> {
        self.storage.clone()
    }

    pub fn remote_label(&self) -> Option<&str> {
        match &self.lifecycle {
            DesktopHostLifecycle::Remote { label, .. } => Some(label),
            DesktopHostLifecycle::Local { .. } => None,
        }
    }

    pub fn remote_environment(&self) -> Option<&yttt_protocol::workspace::WorkspaceEnvironment> {
        match &self.lifecycle {
            DesktopHostLifecycle::Remote { environment, .. } => Some(environment),
            DesktopHostLifecycle::Local { .. } => None,
        }
    }

    pub fn workspace_request(
        &self,
        request: yttt_protocol::workspace::WorkspaceRequest,
    ) -> Result<yttt_protocol::workspace::WorkspaceResponse, String> {
        match self.request_blocking(Request::Workspace(request))? {
            Response::Workspace(response) => Ok(response),
            _ => Err("Host returned an unexpected workspace response".to_string()),
        }
    }

    pub fn shutdown_client(&self) {
        self.runtime.block_on(self.client.shutdown());
        if let DesktopHostLifecycle::Local {
            managed_process, ..
        } = &self.lifecycle
        {
            managed_process.lock().release_desktop_owner();
        }
    }

    pub fn state(&self) -> ConnectionState {
        self.client.state()
    }

    pub fn control_status(&self) -> Option<yttt_protocol::session::ControlStatus> {
        self.client.control_status()
    }

    pub fn is_controller(&self) -> bool {
        self.client.is_controller()
    }

    pub fn shared_editing_enabled(&self) -> bool {
        shared_editing_available(
            &self.state(),
            self.client.is_controller(),
            self.preparing_transfer().is_some(),
        )
    }

    pub fn preparing_transfer(&self) -> Option<String> {
        self.client
            .control_status()
            .and_then(|status| status.transfer)
            .filter(|transfer| transfer.previous_owner.as_ref() == Some(self.client.client_id()))
            .map(|transfer| transfer.id)
            .or_else(|| self.coordinator.local_flush())
    }

    pub fn claim_workspace_view(
        &self,
        restore_existing: bool,
    ) -> Result<crate::session_coordinator::WorkspaceViewLease, String> {
        self.coordinator.claim(None, restore_existing)
    }

    pub fn sharing_ready(&self) -> bool {
        self.coordinator.sharing_ready()
    }
    pub(crate) fn begin_exit_publication(&self) -> String {
        self.coordinator.begin_local_flush()
    }
    pub(crate) fn cancel_exit_publication(&self) {
        self.coordinator.end_local_flush();
    }
    pub(crate) fn exit_publication_ready(&self, id: &str) -> Result<bool, String> {
        self.coordinator.local_flush_ready(id)
    }

    pub fn pending_workspace_count(&self) -> usize {
        self.coordinator.pending_workspace_count()
    }

    pub fn actual_lifetime(&self) -> yttt_host::HostLifetime {
        match &self.lifecycle {
            DesktopHostLifecycle::Local {
                managed_process, ..
            } => managed_process.lock().actual_lifetime(),
            DesktopHostLifecycle::Remote { .. } => yttt_host::HostLifetime::Independent,
        }
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
        match &self.lifecycle {
            DesktopHostLifecycle::Local { launcher, .. } => {
                let launcher = launcher.clone();
                self.runtime.spawn(async move {
                    let result = async {
                        let mut client = launcher.connect_lifecycle(can_force_stop).await?;
                        client.request(request).await
                    }
                    .await;
                    let _ = sender.send_async(result).await;
                });
            }
            DesktopHostLifecycle::Remote {
                connector,
                identity,
                token,
                ..
            } => {
                let connector = connector.clone();
                let mut identity = identity.clone();
                identity.channel = yttt_protocol::ConnectionChannel::Lifecycle;
                identity.supported =
                    yttt_protocol::ProtocolRange::exact(yttt_protocol::LIFECYCLE_PROTOCOL_VERSION);
                identity.can_force_stop = can_force_stop;
                let token = token.clone();
                self.runtime.spawn(async move {
                    use yttt_transport::TransportConnector;
                    let result = async {
                        let mut stream = connector
                            .connect()
                            .await
                            .map_err(|error| HostLaunchError::RequestFailed(error.to_string()))?;
                        yttt_transport::client_handshake(&mut stream, &identity, &token).await?;
                        yttt_transport::send_lifecycle(
                            &mut stream,
                            &yttt_protocol::LifecycleMessage::Request(
                                yttt_protocol::LifecycleRequestEnvelope {
                                    request_id: 1,
                                    body: request,
                                },
                            ),
                        )
                        .await?;
                        match yttt_transport::receive_lifecycle(&mut stream).await? {
                            yttt_protocol::LifecycleMessage::Response(response)
                                if response.request_id == 1 =>
                            {
                                Ok(response.result)
                            }
                            _ => Err(HostLaunchError::UnexpectedMessage),
                        }
                    }
                    .await;
                    let _ = sender.send_async(result).await;
                });
            }
        }
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
        intent: TerminalStartIntent,
        attempt: &TerminalStartAttempt,
    ) -> Result<Request, TerminalRecoveryError> {
        let observed_host_epoch = match self.state() {
            ConnectionState::Ready { host_epoch, .. } => host_epoch,
            _ => return Err(TerminalRecoveryError::HostUnavailable),
        };
        if observed_host_epoch != attempt.expected_host_epoch {
            return Err(TerminalRecoveryError::HostEpochChanged {
                expected: attempt.expected_host_epoch,
                actual: observed_host_epoch,
            });
        }
        if catalog.host_epoch != attempt.expected_host_epoch {
            return Err(TerminalRecoveryError::HostEpochChanged {
                expected: attempt.expected_host_epoch,
                actual: catalog.host_epoch,
            });
        }

        if !self.shared_editing_enabled() {
            let placement = catalog
                .terminals
                .iter()
                .find(|placement| placement.session_id == spec.session_id)
                .ok_or_else(|| {
                    TerminalRecoveryError::MissingObservedSession(spec.session_id.clone())
                })?;
            validate_terminal_placement(&spec, placement, intent)?;
            return Ok(terminal_attach_request(
                &spec,
                placement,
                yttt_protocol::terminal::TerminalLeaseMode::Observer,
            ));
        }
        reconcile_terminal_start(spec, catalog, intent, attempt)
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
                session_id,
                session_epoch,
                final_sequence,
            })?;
        self.runtime.spawn(async move {
            match pending.wait().await {
                Ok(Response::TerminalExitAcknowledged) => {}
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
        let Response::Resources(catalog) = self.request_blocking(Request::ListResources)? else {
            return Err("unexpected Host resource catalog response".to_string());
        };
        let mut requests = Vec::with_capacity(session_ids.len());
        for session_id in session_ids {
            let Some(placement) = catalog
                .terminals
                .iter()
                .find(|placement| placement.session_id == session_id)
            else {
                continue;
            };
            if matches!(
                placement.process_state,
                yttt_protocol::terminal::TerminalProcessState::Exited { .. }
                    | yttt_protocol::terminal::TerminalProcessState::Failed
            ) {
                continue;
            }
            requests.push(TerminateTerminalRequest {
                request_id: self
                    .next_terminal_request_id
                    .fetch_add(1, Ordering::Relaxed),
                session_id,
                host_epoch: catalog.host_epoch,
                session_epoch: placement.session_epoch,
            });
        }
        if requests.is_empty() {
            return Ok(Vec::new());
        }
        let response = self.request_blocking(Request::TerminateMany {
            requests: requests.clone(),
        })?;
        let Response::TerminalsTerminated { results } = response else {
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
            return Err("Host returned an incomplete terminate-many result".to_string());
        }
        Ok(results)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalStartAttempt {
    pub start_id: String,
    pub expected_host_epoch: u64,
}

impl TerminalStartAttempt {
    pub fn new(expected_host_epoch: u64) -> Self {
        Self {
            start_id: uuid::Uuid::new_v4().to_string(),
            expected_host_epoch,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerminalStartIntent {
    Fresh,
    Restore,
    Attach,
}

#[derive(Debug, thiserror::Error)]
pub enum TerminalRecoveryError {
    #[error(
        "terminal address conflicts with Host session {session_id}: expected fingerprint {expected:#x}, received {actual:#x}"
    )]
    AddressConflict {
        session_id: TerminalSessionId,
        expected: u64,
        actual: u64,
    },
    #[error("terminal {0} is not running; only the controller can explicitly start it")]
    MissingObservedSession(TerminalSessionId),
    #[error("Host is not connected to reconcile the terminal start")]
    HostUnavailable,
    #[error(
        "Host epoch changed while reconciling terminal start: expected {expected}, observed {actual}"
    )]
    HostEpochChanged { expected: u64, actual: u64 },
}

fn reconcile_terminal_start(
    spec: TerminalSpawnSpec,
    catalog: &ResourceCatalog,
    intent: TerminalStartIntent,
    attempt: &TerminalStartAttempt,
) -> Result<Request, TerminalRecoveryError> {
    if let Some(placement) = catalog
        .terminals
        .iter()
        .find(|placement| placement.session_id == spec.session_id)
    {
        validate_terminal_placement(&spec, placement, intent)?;
        return Ok(terminal_attach_request(
            &spec,
            placement,
            yttt_protocol::terminal::TerminalLeaseMode::Interactive,
        ));
    }
    if intent == TerminalStartIntent::Attach {
        return Err(TerminalRecoveryError::MissingObservedSession(
            spec.session_id,
        ));
    }

    Ok(Request::SpawnTerminal {
        spec,
        start_id: attempt.start_id.clone(),
        expected_host_epoch: attempt.expected_host_epoch,
    })
}

fn validate_terminal_placement(
    spec: &TerminalSpawnSpec,
    placement: &TerminalPlacement,
    intent: TerminalStartIntent,
) -> Result<(), TerminalRecoveryError> {
    let actual = spec.address_fingerprint();
    // A validated workspace restores an existing resource by stable identity,
    // not by replaying its old execution spec (resume args may have changed).
    if placement.project_id != spec.project_id
        || (intent == TerminalStartIntent::Fresh && placement.spawn_fingerprint != actual)
    {
        return Err(TerminalRecoveryError::AddressConflict {
            session_id: placement.session_id.clone(),
            expected: placement.spawn_fingerprint,
            actual,
        });
    }
    Ok(())
}

fn terminal_attach_request(
    spec: &TerminalSpawnSpec,
    placement: &TerminalPlacement,
    mode: yttt_protocol::terminal::TerminalLeaseMode,
) -> Request {
    Request::AttachTerminal(AttachTerminal {
        session_id: placement.session_id.clone(),
        known_session_epoch: Some(placement.session_epoch),
        after_sequence: (placement.last_sequence > 0).then_some(placement.last_sequence),
        mode,
        geometry: spec.geometry,
        geometry_epoch: placement.geometry_epoch.saturating_add(1).max(1),
        query_palette: spec.query_palette.clone(),
        palette_revision: spec.palette_revision,
    })
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
    fn shared_editing_requires_live_connection_control_and_no_transfer() {
        let ready = ConnectionState::Ready {
            host_epoch: 1,
            connection_sequence: 1,
        };

        assert!(shared_editing_available(&ready, true, false));
        assert!(!shared_editing_available(
            &ConnectionState::Disconnected,
            true,
            false
        ));
        assert!(!shared_editing_available(
            &ConnectionState::Reconnecting {
                attempt: 1,
                message: "reconnecting".to_string(),
            },
            true,
            false
        ));
        assert!(!shared_editing_available(&ready, false, false));
        assert!(!shared_editing_available(&ready, true, true));
    }

    fn attempt() -> TerminalStartAttempt {
        TerminalStartAttempt {
            start_id: "terminal-start-attempt".to_string(),
            expected_host_epoch: 7,
        }
    }

    #[test]
    fn workspace_restore_attaches_existing_process_without_replaying_changed_execution() {
        let original = spec();
        let placement = TerminalPlacement {
            session_id: original.session_id.clone(),
            session_epoch: 3,
            geometry_epoch: 1,
            project_id: original.project_id.clone(),
            geometry: original.geometry,
            last_sequence: 11,
            spawn_fingerprint: original.address_fingerprint(),
            owner: None,
            process_state: TerminalProcessState::Running,
        };
        let catalog = catalog(vec![placement]);
        let mut resumed = original;
        resumed.execution = TerminalExecutionSpec::Command {
            shell: "/bin/sh".into(),
            program: "codex".into(),
            args: vec!["resume".into(), "saved-session".into()],
            return_to_shell: false,
        };
        assert!(matches!(
            reconcile_terminal_start(
                resumed.clone(),
                &catalog,
                TerminalStartIntent::Restore,
                &attempt(),
            )
            .unwrap(),
            Request::AttachTerminal(AttachTerminal {
                known_session_epoch: Some(3),
                ..
            })
        ));
        assert!(matches!(
            reconcile_terminal_start(
                resumed.clone(),
                &catalog,
                TerminalStartIntent::Fresh,
                &attempt(),
            ),
            Err(TerminalRecoveryError::AddressConflict { .. })
        ));
        resumed.project_id = ProjectId::new("another-project");
        assert!(matches!(
            reconcile_terminal_start(resumed, &catalog, TerminalStartIntent::Restore, &attempt(),),
            Err(TerminalRecoveryError::AddressConflict { .. })
        ));
    }

    #[test]
    fn attach_only_restore_never_spawns_a_missing_command() {
        let requested = spec();
        assert!(matches!(
            reconcile_terminal_start(
                requested,
                &catalog(Vec::new()),
                TerminalStartIntent::Attach,
                &attempt(),
            ),
            Err(TerminalRecoveryError::MissingObservedSession(_))
        ));
    }

    #[test]
    fn recovery_rejects_conflicting_catalog_identity() {
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
        assert!(matches!(
            reconcile_terminal_start(
                requested,
                &catalog(vec![conflicting_placement]),
                TerminalStartIntent::Fresh,
                &attempt(),
            ),
            Err(TerminalRecoveryError::AddressConflict { .. })
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
}
