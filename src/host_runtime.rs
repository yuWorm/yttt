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
    LifecycleRequest, LifecycleResponse, Request, ResourceCatalog, Response,
    TerminalTerminationResult,
    terminal::{TerminalInput, TerminalStreamUpdate, TerminateTerminalRequest},
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
        ClientEvent::ResourceCatalogUpdated(_) => true,
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

async fn forward_client_events(
    mut events: tokio::sync::broadcast::Receiver<ClientEvent>,
    sender: flume::Sender<ClientEvent>,
    latest_agents: impl Fn() -> Option<(u64, Vec<yttt_protocol::agent::AgentSnapshotUpdate>)>,
) {
    loop {
        match events.recv().await {
            Ok(event) => {
                if sender.send_async(event).await.is_err() {
                    break;
                }
            }
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                // A quiet Agent's final update may be the one that was dropped.
                // Replay the authoritative cache, not just the surviving ring entries.
                if let Some((host_epoch, snapshots)) = latest_agents() {
                    if sender
                        .send_async(ClientEvent::AgentSnapshotsReset { host_epoch })
                        .await
                        .is_err()
                    {
                        break;
                    }
                    for snapshot in snapshots {
                        if sender
                            .send_async(ClientEvent::AgentSnapshotUpdated(Box::new(snapshot)))
                            .await
                            .is_err()
                        {
                            return;
                        }
                    }
                }
            }
            Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
        }
    }
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
        let coordinator = crate::session_coordinator::SessionCoordinator::new(index);
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
        let coordinator = crate::session_coordinator::SessionCoordinator::new(index);
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

    /// The newest Host layout received for a workspace this Client opened.
    pub fn workspace_layout(
        &self,
        workspace_id: &yttt_protocol::workspace::WorkspaceId,
    ) -> Option<crate::model::host_workspace::HostWorkspaceState> {
        self.client.workspace_layout(workspace_id)
    }

    /// The revision of that layout; `None` while the Host holds no valid
    /// layout for the workspace.
    pub fn workspace_layout_revision(
        &self,
        workspace_id: &yttt_protocol::workspace::WorkspaceId,
    ) -> Option<u64> {
        self.client.workspace_layout_revision(workspace_id)
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

    /// Whether the Client is connected to the Host, so Host state can change.
    pub fn is_connected(&self) -> bool {
        matches!(self.state(), ConnectionState::Ready { .. })
    }

    pub fn claim_workspace_view(
        &self,
        restore_existing: bool,
    ) -> Result<crate::session_coordinator::WorkspaceViewLease, String> {
        self.coordinator.claim(None, restore_existing)
    }

    pub(crate) fn claim_history_workspace(
        &self,
        id: yttt_protocol::workspace::WorkspaceId,
    ) -> Result<crate::session_coordinator::WorkspaceViewLease, String> {
        self.coordinator.claim(Some(id), false)
    }

    pub(crate) fn workspace_has_view(&self, id: &yttt_protocol::workspace::WorkspaceId) -> bool {
        self.coordinator.is_open(id)
    }

    pub(crate) fn has_workspace_history(&self) -> bool {
        use crate::config::storage::ConfigStorage as _;
        self.storage.environment().workspace_history_initialized
            || self.coordinator.has_workspace_history()
    }

    /// Takes the workspaces no window restored out of the next startup.
    pub(crate) fn archive_unclaimed_workspaces(&self) -> Result<(), String> {
        use yttt_protocol::workspace::{WorkspaceRequest, WorkspaceResponse};
        let WorkspaceResponse::Workspaces(entries) =
            self.workspace_request(WorkspaceRequest::List)?
        else {
            return Err("Host did not return its workspace index".into());
        };
        for entry in entries {
            if !entry.restore_on_startup || self.coordinator.is_open(&entry.workspace_id) {
                continue;
            }
            self.workspace_request(WorkspaceRequest::SetRestoreOnStartup {
                workspace_id: entry.workspace_id.clone(),
                expected_revision: entry.revision,
                restore: false,
            })?;
            self.coordinator
                .remove_pending_workspace(&entry.workspace_id);
        }
        Ok(())
    }

    /// `Ok(true)` once every window's document edits are stored on the Host.
    pub(crate) fn documents_synced(&self) -> Result<bool, String> {
        self.coordinator.documents_synced()
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

    /// Signals Host layout changes of `workspace_id`. A change that arrives
    /// while a signal is pending coalesces into it; read the layout itself
    /// from [`Self::workspace_layout`].
    pub fn workspace_layout_changes(
        &self,
        workspace_id: yttt_protocol::workspace::WorkspaceId,
    ) -> flume::Receiver<()> {
        let (sender, receiver) = flume::bounded(1);
        let mut events = self.client.subscribe_events();
        self.runtime.spawn(async move {
            loop {
                match events.recv().await {
                    Ok(ClientEvent::WorkspaceLayoutChanged(update))
                        if update.workspace_id == workspace_id => {}
                    Ok(_) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
                if let Err(flume::TrySendError::Disconnected(())) = sender.try_send(()) {
                    break;
                }
            }
        });
        receiver
    }

    /// Lease and draft changes of the documents of `workspace_id`. `None`
    /// means changes were dropped and the states must be read again.
    pub fn document_changes(
        &self,
        workspace_id: yttt_protocol::workspace::WorkspaceId,
    ) -> flume::Receiver<Option<yttt_protocol::workspace::DocumentState>> {
        let (sender, receiver) = flume::bounded(256);
        let mut events = self.client.subscribe_events();
        self.runtime.spawn(async move {
            loop {
                let change = match events.recv().await {
                    Ok(ClientEvent::Server(event)) => match event.body {
                        yttt_protocol::ServerEvent::DocumentChanged(update)
                            if update.workspace_id == workspace_id =>
                        {
                            Some(update.state)
                        }
                        _ => continue,
                    },
                    Ok(_) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => None,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                };
                if sender.send_async(change).await.is_err() {
                    break;
                }
            }
        });
        receiver
    }

    pub fn client_id(&self) -> ClientInstanceId {
        self.client.client_id().clone()
    }

    pub fn events(&self) -> flume::Receiver<ClientEvent> {
        let (sender, receiver) = flume::bounded(256);
        let events = self.client.subscribe_events();
        let client = Arc::downgrade(&self.client);
        self.runtime.spawn(async move {
            forward_client_events(events, sender, move || {
                let client = client.upgrade()?;
                let host_epoch = client.host_epoch()?;
                Some((host_epoch, client.agent_snapshots()))
            })
            .await;
        });
        receiver
    }

    pub fn terminal_snapshot(
        &self,
        session_id: &TerminalSessionId,
    ) -> Option<yttt_protocol::terminal::SemanticViewport> {
        self.client.terminal_snapshot(session_id)
    }

    pub fn terminal_metadata(
        &self,
        session_id: &TerminalSessionId,
    ) -> Option<TerminalMirrorMetadata> {
        self.client.terminal_metadata(session_id)
    }

    pub fn set_visible_terminals(
        &self,
        scope: u64,
        sessions: impl IntoIterator<Item = TerminalSessionId>,
    ) {
        self.client.set_visible_terminals(scope, sessions);
    }

    pub fn clear_visible_terminals(&self, scope: u64) {
        self.client.clear_visible_terminals(scope);
    }

    pub fn resource_catalog(&self) -> Option<Arc<ResourceCatalog>> {
        self.client.resource_catalog()
    }
    pub fn agent_snapshots(&self) -> Vec<yttt_protocol::agent::AgentSnapshotUpdate> {
        self.client.agent_snapshots()
    }

    pub fn project_token_usage(
        &self,
        project_id: &str,
    ) -> Option<yttt_protocol::agent::ProjectTokenUsage> {
        self.client.project_token_usage(project_id)
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
    use yttt_core::model::ids::ProfileId;

    #[tokio::test]
    async fn lag_recovers_a_quiet_agents_final_snapshot() {
        use yttt_agent_core::{AgentInstanceId, AgentReducer, ProviderId};
        use yttt_protocol::agent::{AgentHookScope, AgentSnapshotUpdate};
        let snapshot = |session: &str, sequence| AgentSnapshotUpdate {
            scope: AgentHookScope {
                project_id: "project".into(),
                tab_id: session.into(),
                pane_id: "pane".into(),
                generation: 1,
            },
            terminal_session_id: TerminalSessionId::new(session),
            host_epoch: 2,
            sequence,
            snapshot: AgentReducer::new(
                AgentInstanceId::random(),
                ProviderId::from_static("omp"),
                1,
            )
            .snapshot()
            .clone(),
        };
        let quiet = snapshot("quiet", 7);
        let noisy = snapshot("noisy", 20);
        let (broadcast, events) = tokio::sync::broadcast::channel(2);
        broadcast
            .send(ClientEvent::AgentSnapshotUpdated(Box::new(quiet.clone())))
            .unwrap();
        for sequence in 1..=20 {
            broadcast
                .send(ClientEvent::AgentSnapshotUpdated(Box::new(snapshot(
                    "noisy", sequence,
                ))))
                .unwrap();
        }
        drop(broadcast);
        let (sender, receiver) = flume::bounded(1);
        let bridge = tokio::spawn(async move {
            forward_client_events(events, sender, || {
                Some((2, vec![quiet.clone(), noisy.clone()]))
            })
            .await;
        });
        let mut latest = std::collections::HashMap::new();
        let mut epoch = None;
        while let Ok(event) = receiver.recv_async().await {
            match event {
                ClientEvent::AgentSnapshotsReset { host_epoch } => epoch = Some(host_epoch),
                ClientEvent::AgentSnapshotUpdated(update) => {
                    let sequence = latest.entry(update.terminal_session_id).or_insert(0);
                    *sequence = (*sequence).max(update.sequence);
                }
                _ => {}
            }
        }
        bridge.await.unwrap();
        assert_eq!(epoch, Some(2));
        assert_eq!(latest.get(&TerminalSessionId::new("quiet")), Some(&7));
        assert_eq!(latest.get(&TerminalSessionId::new("noisy")), Some(&20));
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
