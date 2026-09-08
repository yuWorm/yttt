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
    lifecycle: DesktopHostLifecycle,
    placement_store: Arc<TerminalPlacementStore>,
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
        let placement_store = Arc::new(TerminalPlacementStore::load(
            profile.config_paths().terminal_placements_file(),
            storage.clone(),
        )?);
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
            placement_store,
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
        config_paths: &crate::config::paths::AppConfigPaths,
    ) -> Result<Arc<Self>, DesktopHostRuntimeError> {
        let placement_store = Arc::new(TerminalPlacementStore::load(
            config_paths.terminal_placements_file(),
            storage.clone(),
        )?);
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
            placement_store,
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
            managed_process.lock().unwrap().release_desktop_owner();
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
        self.client.is_controller() && self.preparing_transfer().is_none()
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
        self.coordinator
            .claim(None, restore_existing || self.is_remote())
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
            } => managed_process.lock().unwrap().actual_lifetime(),
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
    ) -> Result<Request, TerminalRecoveryError> {
        if !self.shared_editing_enabled() {
            let placement = catalog
                .terminals
                .iter()
                .find(|placement| placement.session_id == spec.session_id)
                .ok_or_else(|| {
                    TerminalRecoveryError::MissingObservedSession(spec.session_id.clone())
                })?;
            return Ok(terminal_attach_request(
                &spec,
                placement,
                yttt_protocol::terminal::TerminalLeaseMode::Observer,
            ));
        }
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
        let client = self.client.clone();
        self.runtime.spawn(async move {
            match pending.wait().await {
                Ok(Response::TerminalExitAcknowledged) => {
                    if client.is_controller()
                        && let Err(error) = placement_store.mark_closed(&session_id)
                    {
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
                Ok(Some(request_id)) => requests.push(TerminateTerminalRequest {
                    request_id,
                    session_id,
                }),
                Ok(None) => {}
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
        if requests.is_empty() {
            return Ok(Vec::new());
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
    #[error("terminal {0} is not running; only the controller can explicitly start it")]
    MissingObservedSession(TerminalSessionId),
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
        return Ok(terminal_attach_request(
            &spec,
            placement,
            yttt_protocol::terminal::TerminalLeaseMode::Interactive,
        ));
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
        let store = TerminalPlacementStore::load_local(&path).unwrap();
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

        let reloaded = TerminalPlacementStore::load_local(path).unwrap();
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
        let address_store =
            TerminalPlacementStore::load_local(temp.path().join("address.json")).unwrap();
        assert!(matches!(
            reconcile_terminal_start(
                &address_store,
                requested.clone(),
                &catalog(vec![conflicting_placement])
            ),
            Err(TerminalRecoveryError::AddressConflict { .. })
        ));

        let pending_store =
            TerminalPlacementStore::load_local(temp.path().join("pending.json")).unwrap();
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

        let close_store =
            TerminalPlacementStore::load_local(temp.path().join("close.json")).unwrap();
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
        let store = TerminalPlacementStore::load_local(path).unwrap();

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
