#![forbid(unsafe_code)]
mod agent_hooks;
mod project;
pub mod runtime;
mod ssh_runtime;
pub mod terminal;

use std::{
    collections::HashSet,
    fs::{self, File, OpenOptions},
    io::{self, Read as _, Seek as _, SeekFrom, Write as _},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

use crate::{
    agent_hooks::HostAgentHookRuntime,
    project::{HostProjectError, HostProjectRuntime},
    runtime::{HostRuntime, HostRuntimeError},
    ssh_runtime::HostSshRuntime,
    terminal::{HostTerminalEvent, HostedTerminalError},
};
use fs2::FileExt as _;
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, watch};
use yttt_core::model::ids::{ClientInstanceId, HostId, ProfileId};
use yttt_protocol::{
    ClientRequest, ControlMessage, FailureCode, HostEvent, HostResponse, PROTOCOL_VERSION,
    ProtocolFailure, ProtocolRange, Request, ResourceCatalog, Response, ServerEvent,
    terminal::{AttachTerminal, TerminalLeaseMode, TerminalStreamUpdate, TerminationMode},
};
use yttt_transport_local::{
    AuthToken, HostIdentity, LocalEndpoint, LocalListener, LocalStream, receive_control,
    send_control, server_handshake,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostBootstrap {
    pub profile_id: ProfileId,
    pub runtime_root: PathBuf,
    pub auth_token_file: PathBuf,
    pub ssh_host_keys_file: PathBuf,
    pub credential_namespace: String,
    pub build_id: String,
}

impl HostBootstrap {
    pub fn endpoint(&self) -> LocalEndpoint {
        LocalEndpoint::for_profile(self.profile_id.clone(), self.runtime_root.clone())
    }

    pub fn lock_file(&self) -> PathBuf {
        self.runtime_root.join("host.lock")
    }

    pub fn pid_file(&self) -> PathBuf {
        self.runtime_root.join("host.pid")
    }

    pub fn ready_file(&self) -> PathBuf {
        self.runtime_root.join("host-ready.json")
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadyMetadata {
    pub profile_id: ProfileId,
    pub host_id: HostId,
    pub host_epoch: u64,
    pub pid: u32,
    pub build_id: String,
    pub started_millis: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum HostError {
    #[error("another Host owns the profile lock")]
    AlreadyRunning,
    #[error("invalid 32-byte Host authentication token")]
    InvalidAuthToken,
    #[error("Host bootstrap file permissions are not user-only")]
    InsecureBootstrapFile,
    #[error("Host I/O failed: {0}")]
    Io(#[from] io::Error),
    #[error("Host transport failed: {0}")]
    Transport(#[from] yttt_transport_local::TransportError),
    #[error("Host metadata failed: {0}")]
    Metadata(#[from] serde_json::Error),
    #[error("Host SSH runtime failed: {0}")]
    Ssh(String),
}

pub async fn run(bootstrap: HostBootstrap) -> Result<(), HostError> {
    fs::create_dir_all(&bootstrap.runtime_root)?;
    secure_runtime_root(&bootstrap.runtime_root)?;
    let guard = HostInstanceGuard::acquire(&bootstrap)?;
    let token = read_auth_token(&bootstrap.auth_token_file)?;
    let listener = LocalListener::bind(bootstrap.endpoint()).await?;
    let host_epoch = next_host_epoch(&bootstrap.runtime_root)?;
    let host_id = HostId::new(format!("host-{}-{host_epoch}", std::process::id()));
    let ready = ReadyMetadata {
        profile_id: bootstrap.profile_id.clone(),
        host_id: host_id.clone(),
        host_epoch,
        pid: std::process::id(),
        build_id: bootstrap.build_id.clone(),
        started_millis: now_millis(),
    };

    let identity = Arc::new(HostIdentity {
        supported: ProtocolRange::exact(PROTOCOL_VERSION),
        build_id: bootstrap.build_id,
        profile_id: bootstrap.profile_id.clone(),
        host_id,
        host_epoch,
        connection_sequence: 1,
    });
    let token = Arc::new(token);
    let next_connection = Arc::new(AtomicU64::new(1));
    let next_host_sequence = Arc::new(AtomicU64::new(1));
    let (stop_tx, mut stop_rx) = watch::channel(false);
    let runtime = HostRuntime::new();
    let projects = Arc::new(HostProjectRuntime::new());
    let ssh = HostSshRuntime::start(
        bootstrap.ssh_host_keys_file.clone(),
        bootstrap.credential_namespace.clone(),
    )
    .map_err(HostError::Ssh)?;
    let agent_hooks = HostAgentHookRuntime::start()?;
    guard.publish_ready(&ready)?;

    loop {
        tokio::select! {
            changed = stop_rx.changed() => {
                if changed.is_err() || *stop_rx.borrow() {
                    break;
                }
            }
            accepted = listener.accept() => {
                let stream = accepted?;
                let mut connection_identity = (*identity).clone();
                connection_identity.connection_sequence =
                    next_connection.fetch_add(1, Ordering::Relaxed);
                let token = token.clone();
                let context = ConnectionContext {
                    identity: connection_identity,
                    profile_id: bootstrap.profile_id.clone(),
                    host_sequence: next_host_sequence.clone(),
                    ssh: ssh.clone(),
                    agent_hooks: agent_hooks.clone(),
                    projects: projects.clone(),
                    runtime: runtime.clone(),
                    stop_tx: stop_tx.clone(),
                };
                tokio::spawn(async move {
                    let _ = serve_connection(stream, token, context).await;
                });
            }
        }
    }

    drop(listener);
    drop(guard);
    Ok(())
}

struct ConnectionContext {
    identity: HostIdentity,
    profile_id: ProfileId,
    host_sequence: Arc<AtomicU64>,
    ssh: Arc<HostSshRuntime>,
    agent_hooks: Arc<HostAgentHookRuntime>,
    projects: Arc<HostProjectRuntime>,
    runtime: Arc<HostRuntime>,
    stop_tx: watch::Sender<bool>,
}

async fn serve_connection(
    mut stream: LocalStream,
    token: Arc<AuthToken>,
    context: ConnectionContext,
) -> Result<(), ()> {
    let ConnectionContext {
        identity,
        profile_id: _,
        host_sequence,
        ssh,
        agent_hooks,
        projects,
        runtime,
        stop_tx,
    } = &context;
    let authenticated = server_handshake(&mut stream, identity, token.as_ref())
        .await
        .map_err(|_| ())?;
    let client_id = authenticated.client_instance_id;
    let mut ssh_events = ssh.subscribe();
    let mut agent_hook_events = agent_hooks.subscribe();
    let mut project_events = projects.subscribe();
    let mut runtime_events = runtime.subscribe();
    let mut subscriptions = HashSet::new();
    let result = async {
    loop {
        tokio::select! {
            message = receive_control(&mut stream) => {
                let ControlMessage::Request(request) = message.map_err(|_| ())? else {
                    return Err(());
                };
                let subscription = match &request.body {
                    Request::SpawnTerminal(spec) => Some(spec.session_id.clone()),
                    Request::AttachTerminal(attach) => Some(attach.session_id.clone()),
                    Request::AcquireTerminalLease { session_id, .. }
                    | Request::RequestCheckpoint { session_id, .. } => Some(session_id.clone()),
                    _ => None,
                };
                let unsubscription = match &request.body {
                    Request::DetachTerminal { session_id }
                    | Request::ReleaseTerminalLease { session_id }
                    | Request::TerminateTerminal { session_id, .. }
                    | Request::AcknowledgeTerminalExit {
                        session_id,
                        ..
                    } => Some(session_id.clone()),
                    _ => None,
                };
                let should_stop = matches!(&request.body, Request::DrainAndStop);
                let response = handle_request(
                    request,
                    &context,
                    &client_id,
                    host_sequence.fetch_add(1, Ordering::Relaxed),
                )
                .await;
                if response.result.is_ok()
                    && let Some(session_id) = subscription
                {
                    subscriptions.insert(session_id);
                }
                if response.result.is_ok()
                    && let Some(session_id) = unsubscription
                {
                    subscriptions.remove(&session_id);
                }
                send_control(&mut stream, &ControlMessage::Response(response))
                    .await
                    .map_err(|_| ())?;
                if should_stop {
                    let _ = send_control(
                        &mut stream,
                        &ControlMessage::Event(HostEvent {
                            host_sequence: host_sequence.fetch_add(1, Ordering::Relaxed),
                            body: ServerEvent::HostStopping,
                        }),
                    )
                    .await;
                    let _ = stop_tx.send(true);
                    return Ok(());
                }
            }
            event = runtime_events.recv() => {
                let server_event = match event {
                    Ok(HostTerminalEvent::Update { session_id, update })
                        if subscriptions.contains(&session_id) =>
                    {
                        Some(ServerEvent::Terminal(update))
                    }
                    Ok(HostTerminalEvent::Exited {
                        session_id,
                        session_epoch,
                        code,
                    }) if subscriptions.contains(&session_id) => {
                        Some(ServerEvent::TerminalExit {
                            session_id,
                            session_epoch,
                            code,
                        })
                    }
                    Ok(HostTerminalEvent::TitleChanged { .. } | HostTerminalEvent::Bell { .. }) => {
                        None
                    }
                    Ok(_) => None,
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        for session_id in &subscriptions {
                            let available_from_sequence = runtime
                                .terminal(session_id)
                                .and_then(|terminal| terminal.latest_viewport())
                                .map_or(0, |viewport| viewport.sequence);
                            let event = ControlMessage::Event(HostEvent {
                                host_sequence: host_sequence.fetch_add(1, Ordering::Relaxed),
                                body: ServerEvent::Terminal(TerminalStreamUpdate::ResyncRequired {
                                    session_id: session_id.clone(),
                                    available_from_sequence,
                                }),
                            });
                            send_control(&mut stream, &event).await.map_err(|_| ())?;
                        }
                        None
                    }
                    Err(broadcast::error::RecvError::Closed) => return Err(()),
                };
                if let Some(body) = server_event {
                    send_control(
                        &mut stream,
                        &ControlMessage::Event(HostEvent {
                            host_sequence: host_sequence.fetch_add(1, Ordering::Relaxed),
                            body,
                        }),
                    )
                    .await
                    .map_err(|_| ())?;
                }
            }
            event = ssh_events.recv() => {
                let body = match event {
                    Ok(event) => event,
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => return Err(()),
                };
                send_control(
                    &mut stream,
                    &ControlMessage::Event(HostEvent {
                        host_sequence: host_sequence.fetch_add(1, Ordering::Relaxed),
                        body,
                    }),
                )
                .await
                .map_err(|_| ())?;
            }
            event = project_events.recv() => {
                match event {
                    Ok(change) => {
                        send_control(
                            &mut stream,
                            &ControlMessage::Event(HostEvent {
                                host_sequence: host_sequence.fetch_add(1, Ordering::Relaxed),
                                body: ServerEvent::ProjectChanged(change),
                            }),
                        )
                        .await
                        .map_err(|_| ())?;
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        for change in projects.resync_changes() {
                            send_control(
                                &mut stream,
                                &ControlMessage::Event(HostEvent {
                                    host_sequence: host_sequence.fetch_add(1, Ordering::Relaxed),
                                    body: ServerEvent::ProjectChanged(change),
                                }),
                            )
                            .await
                            .map_err(|_| ())?;
                        }
                    }
                    Err(broadcast::error::RecvError::Closed) => return Err(()),
                }
            }
            event = agent_hook_events.recv() => {
                let body = match event {
                    Ok(event) => event,
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => return Err(()),
                };
                send_control(
                    &mut stream,
                    &ControlMessage::Event(HostEvent {
                        host_sequence: host_sequence.fetch_add(1, Ordering::Relaxed),
                        body,
                    }),
                )
                .await
                .map_err(|_| ())?;
            }
        }
    }
    }.await;
    runtime.release_client(&client_id);
    result
}

async fn handle_request(
    request: ClientRequest,
    context: &ConnectionContext,
    client_id: &ClientInstanceId,
    host_sequence: u64,
) -> HostResponse {
    let ConnectionContext {
        identity,
        profile_id,
        ssh,
        agent_hooks,
        projects,
        runtime,
        ..
    } = context;
    let ClientRequest { request_id, body } = request;
    let result = match body {
        Request::Ping { sent_millis } => Ok(Response::Pong {
            sent_millis,
            host_millis: now_millis(),
        }),
        Request::ListResources => Ok(Response::Resources(ResourceCatalog {
            profile_id: profile_id.clone(),
            host_id: identity.host_id.clone(),
            host_epoch: identity.host_epoch,
            revision: host_sequence,
            terminals: runtime.placements(),
            ssh_connections: ssh.connections(),
            projects: projects.projects(),
        })),
        Request::SpawnTerminal(spec) => (|| {
            let terminal = runtime.spawn_with_transport(spec, Some(ssh.transport()))?;
            runtime.acquire_lease(
                terminal.session_id(),
                client_id,
                TerminalLeaseMode::Interactive,
            )?;
            Ok(Response::TerminalSpawned {
                session_id: terminal.session_id().clone(),
                session_epoch: terminal.session_epoch(),
            })
        })()
        .map_err(runtime_failure),
        Request::AttachTerminal(attach) => {
            attach_terminal(runtime, client_id, attach).map_err(runtime_failure)
        }
        Request::DetachTerminal { session_id } | Request::ReleaseTerminalLease { session_id } => {
            runtime.release_lease(&session_id, client_id);
            Ok(Response::TerminalDetached)
        }
        Request::AcquireTerminalLease { session_id, mode } => runtime
            .acquire_lease(&session_id, client_id, mode)
            .map(Response::TerminalLease)
            .map_err(runtime_failure),
        Request::TerminalInput(input) => (|| {
            runtime.validate_lease(&input.session_id, client_id)?;
            let terminal = runtime
                .terminal(&input.session_id)
                .ok_or_else(|| HostRuntimeError::NotFound(input.session_id.clone()))?;
            terminal.input(input.bytes)?;
            Ok(Response::TerminalInputAccepted {
                client_sequence: input.client_sequence,
            })
        })()
        .map_err(runtime_failure),
        Request::ResizeTerminal {
            session_id,
            geometry,
            geometry_epoch,
        } => (|| {
            runtime.validate_lease(&session_id, client_id)?;
            let terminal = runtime
                .terminal(&session_id)
                .ok_or_else(|| HostRuntimeError::NotFound(session_id.clone()))?;
            terminal.resize(geometry, geometry_epoch)?;
            Ok(Response::TerminalResized { geometry_epoch })
        })()
        .map_err(runtime_failure),
        Request::ScrollTerminal {
            session_id,
            display_offset,
        } => (|| {
            runtime.validate_lease(&session_id, client_id)?;
            let terminal = runtime
                .terminal(&session_id)
                .ok_or_else(|| HostRuntimeError::NotFound(session_id.clone()))?;
            let viewport = terminal
                .latest_viewport()
                .ok_or_else(|| HostRuntimeError::NotFound(session_id.clone()))?;
            let delta = display_offset as i128 - viewport.display_offset as i128;
            terminal.scroll(
                delta.clamp(i32::MIN as i128, i32::MAX as i128) as i32,
                viewport.scrollback_epoch,
            )?;
            let actual_offset = terminal
                .latest_viewport()
                .map_or(0, |viewport| viewport.display_offset);
            Ok(Response::TerminalScrolled {
                display_offset: actual_offset,
            })
        })()
        .map_err(runtime_failure),
        Request::SetTerminalQueryPalette {
            session_id,
            colors,
            revision,
        } => (|| {
            runtime.validate_lease(&session_id, client_id)?;
            let terminal = runtime
                .terminal(&session_id)
                .ok_or_else(|| HostRuntimeError::NotFound(session_id.clone()))?;
            terminal.set_query_palette(colors, revision);
            Ok(Response::TerminalPaletteAccepted { revision })
        })()
        .map_err(runtime_failure),
        Request::RequestCheckpoint { session_id, .. } => runtime
            .terminal(&session_id)
            .ok_or_else(|| HostRuntimeError::NotFound(session_id.clone()))
            .and_then(|terminal| {
                terminal
                    .checkpoint()
                    .map(Response::TerminalCheckpoint)
                    .ok_or(HostRuntimeError::NotFound(session_id))
            })
            .map_err(runtime_failure),
        Request::AcknowledgeTerminalExit {
            session_id,
            session_epoch,
        } => runtime
            .acknowledge_terminal_exit(&session_id, session_epoch)
            .map(|()| Response::TerminalExitAcknowledged)
            .map_err(runtime_failure),
        Request::TerminateTerminal { session_id, mode } => match mode {
            TerminationMode::Detach => {
                runtime.release_lease(&session_id, client_id);
                Ok(Response::TerminalDetached)
            }
            TerminationMode::Terminate | TerminationMode::TerminateMany => (|| {
                runtime.validate_lease(&session_id, client_id)?;
                runtime.terminate(&session_id)?;
                Ok(Response::TerminalTerminated)
            })()
            .map_err(runtime_failure),
        },
        Request::TerminateMany { session_ids } => {
            let mut terminated = 0;
            let mut failure = None;
            for session_id in session_ids {
                let result = (|| {
                    runtime.validate_lease(&session_id, client_id)?;
                    let session_epoch = runtime
                        .terminal(&session_id)
                        .ok_or_else(|| HostRuntimeError::NotFound(session_id.clone()))?
                        .session_epoch();
                    runtime.terminate(&session_id)?;
                    runtime.acknowledge_terminal_exit(&session_id, session_epoch)
                })();
                match result {
                    Ok(()) => terminated += 1,
                    Err(error) => {
                        failure = Some(runtime_failure(error));
                        break;
                    }
                }
            }
            failure.map_or_else(|| Ok(Response::TerminalsTerminated { terminated }), Err)
        }
        Request::SshConnect(spec) => ssh.connect(spec).await.map_err(ssh_failure),
        Request::SshDisconnect { connection_id } => {
            ssh.disconnect(connection_id).await.map_err(ssh_failure)
        }
        Request::CredentialAnswer {
            challenge_id,
            answer,
        } => ssh
            .answer_credential(challenge_id, answer)
            .map_err(ssh_failure),
        Request::DeleteSshCredential { credential_id } => {
            ssh.delete_credential(credential_id).map_err(ssh_failure)
        }
        Request::RemoteFile(request) => {
            let ssh = ssh.clone();
            tokio::task::spawn_blocking(move || ssh.remote_file(request))
                .await
                .map_err(|error| ssh_failure(error.to_string()))
                .and_then(|result| result.map_err(ssh_failure))
        }
        Request::RemoteCommand(request) => {
            let ssh = ssh.clone();
            tokio::task::spawn_blocking(move || ssh.remote_command(request))
                .await
                .map_err(|error| ssh_failure(error.to_string()))
                .and_then(|result| result.map_err(ssh_failure))
        }
        Request::Project(request) => {
            let projects = projects.clone();
            tokio::task::spawn_blocking(move || projects.handle(request))
                .await
                .map_err(|error| {
                    ProtocolFailure::new(FailureCode::Internal, error.to_string(), false)
                })
                .and_then(|result| result.map(Response::Project).map_err(project_failure))
        }
        Request::AgentHookEnvironment(scope) => Ok(Response::AgentHookEnvironment(
            agent_hooks.environment(scope),
        )),
        Request::DrainAndStop => {
            runtime.terminate_all();
            Ok(Response::Draining)
        }
    };
    HostResponse { request_id, result }
}

fn attach_terminal(
    runtime: &Arc<HostRuntime>,
    client_id: &ClientInstanceId,
    attach: AttachTerminal,
) -> Result<Response, HostRuntimeError> {
    let terminal = runtime
        .terminal(&attach.session_id)
        .ok_or_else(|| HostRuntimeError::NotFound(attach.session_id.clone()))?;
    if let Some(known_epoch) = attach.known_session_epoch
        && known_epoch != terminal.session_epoch()
    {
        return Err(HostRuntimeError::StaleSessionEpoch {
            session_id: attach.session_id,
            expected: terminal.session_epoch(),
            actual: known_epoch,
        });
    }
    let lease = runtime.acquire_lease(
        terminal.session_id(),
        client_id,
        TerminalLeaseMode::Interactive,
    )?;
    let current_geometry_epoch = terminal
        .latest_viewport()
        .map_or(0, |viewport| viewport.geometry_epoch);
    if attach.geometry_epoch > current_geometry_epoch {
        terminal.resize(attach.geometry, attach.geometry_epoch)?;
    }
    let checkpoint = terminal
        .checkpoint()
        .ok_or_else(|| HostRuntimeError::NotFound(terminal.session_id().clone()))?;
    Ok(Response::TerminalAttached { lease, checkpoint })
}

fn runtime_failure(error: HostRuntimeError) -> ProtocolFailure {
    let code = match &error {
        HostRuntimeError::AlreadyExists(_) => FailureCode::AlreadyExists,
        HostRuntimeError::NotFound(_) => FailureCode::NotFound,
        HostRuntimeError::LeaseConflict { .. } | HostRuntimeError::TerminalStillRunning(_) => {
            FailureCode::Conflict
        }
        HostRuntimeError::LeaseRequired(_) => FailureCode::PermissionDenied,
        HostRuntimeError::StaleSessionEpoch { .. }
        | HostRuntimeError::Terminal(
            HostedTerminalError::StaleGeometry { .. } | HostedTerminalError::StaleScrollback { .. },
        ) => FailureCode::StaleEpoch,
        HostRuntimeError::Terminal(HostedTerminalError::Backpressure) => FailureCode::Backpressure,
        HostRuntimeError::Terminal(HostedTerminalError::Stopped) => FailureCode::TransportClosed,
        HostRuntimeError::Terminal(HostedTerminalError::InvalidGeometry) => {
            FailureCode::InvalidRequest
        }
        HostRuntimeError::Terminal(
            HostedTerminalError::UnsupportedExecution
            | HostedTerminalError::Io(_)
            | HostedTerminalError::Pty(_),
        ) => FailureCode::Internal,
    };
    ProtocolFailure::new(code, error.to_string(), false)
}

fn ssh_failure(error: String) -> ProtocolFailure {
    ProtocolFailure::new(FailureCode::TransportClosed, error, false)
}

fn project_failure(error: HostProjectError) -> ProtocolFailure {
    ProtocolFailure::new(error.code, error.to_string(), false)
}

pub fn read_ready_metadata(path: &Path) -> Result<ReadyMetadata, HostError> {
    let bytes = fs::read(path)?;
    Ok(serde_json::from_slice(&bytes)?)
}

fn read_auth_token(path: &Path) -> Result<AuthToken, HostError> {
    validate_user_only_file(path)?;
    let mut file = File::open(path)?;
    let mut bytes = [0_u8; 32];
    file.read_exact(&mut bytes)
        .map_err(|_| HostError::InvalidAuthToken)?;
    let mut trailing = [0_u8; 1];
    if file.read(&mut trailing)? != 0 {
        return Err(HostError::InvalidAuthToken);
    }
    Ok(AuthToken::from_bytes(bytes))
}

struct HostInstanceGuard {
    lock: File,
    pid_file: PathBuf,
    ready_file: PathBuf,
}

impl HostInstanceGuard {
    fn acquire(bootstrap: &HostBootstrap) -> Result<Self, HostError> {
        let mut options = OpenOptions::new();
        options.create(true).read(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        let mut lock = options.open(bootstrap.lock_file())?;
        lock.try_lock_exclusive()
            .map_err(|_| HostError::AlreadyRunning)?;
        lock.set_len(0)?;
        lock.seek(SeekFrom::Start(0))?;
        writeln!(lock, "{}", std::process::id())?;
        lock.sync_all()?;
        atomic_write(
            &bootstrap.pid_file(),
            std::process::id().to_string().as_bytes(),
        )?;
        Ok(Self {
            lock,
            pid_file: bootstrap.pid_file(),
            ready_file: bootstrap.ready_file(),
        })
    }

    fn publish_ready(&self, ready: &ReadyMetadata) -> Result<(), HostError> {
        atomic_write(&self.ready_file, &serde_json::to_vec(ready)?)?;
        Ok(())
    }
}

impl Drop for HostInstanceGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.ready_file);
        let _ = fs::remove_file(&self.pid_file);
        let _ = self.lock.unlock();
    }
}

fn next_host_epoch(runtime_root: &Path) -> Result<u64, HostError> {
    let path = runtime_root.join("host-epoch");
    let previous = fs::read_to_string(&path)
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .unwrap_or(0);
    let next = previous.saturating_add(1).max(1);
    atomic_write(&path, next.to_string().as_bytes())?;
    Ok(next)
}

fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let temporary = path.with_extension(format!("tmp-{}", std::process::id()));
    let mut options = OpenOptions::new();
    options.create(true).truncate(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options.open(&temporary)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    fs::rename(temporary, path)?;
    Ok(())
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

#[cfg(unix)]
fn secure_runtime_root(path: &Path) -> Result<(), HostError> {
    use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_dir() || metadata.uid() != rustix::process::geteuid().as_raw() {
        return Err(HostError::InsecureBootstrapFile);
    }
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    Ok(())
}

#[cfg(windows)]
fn secure_runtime_root(path: &Path) -> Result<(), HostError> {
    if !fs::metadata(path)?.is_dir() {
        return Err(HostError::InsecureBootstrapFile);
    }
    Ok(())
}

#[cfg(unix)]
fn validate_user_only_file(path: &Path) -> Result<(), HostError> {
    use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file()
        || metadata.uid() != rustix::process::geteuid().as_raw()
        || metadata.permissions().mode() & 0o077 != 0
    {
        return Err(HostError::InsecureBootstrapFile);
    }
    Ok(())
}

#[cfg(windows)]
fn validate_user_only_file(path: &Path) -> Result<(), HostError> {
    if !fs::metadata(path)?.is_file() {
        return Err(HostError::InsecureBootstrapFile);
    }
    Ok(())
}
