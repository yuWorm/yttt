use std::{
    collections::HashMap,
    fmt,
    net::SocketAddr,
    path::PathBuf,
    sync::{Arc, Mutex, mpsc as blocking_mpsc},
    thread,
    time::Duration,
};

use async_channel::{Receiver as EventReceiver, Sender as EventSender};
use russh::{
    ChannelMsg, client,
    keys::{
        PrivateKeyWithHashAlg,
        agent::client::AgentClient,
        ssh_key::{HashAlg, PublicKey},
    },
};
use russh_sftp::client::SftpSession;
use thiserror::Error;
use tokio::sync::{mpsc, oneshot};
use yttt_core::model::{
    ids::{ConnectionId, CredentialId},
    project::{RemotePathBuf, RemoteRelativePathBuf},
};
use zeroize::Zeroizing;

use crate::credential::CredentialStore;
use crate::host_keys::{HostKeyStore, HostKeyVerification};
use crate::sftp::{
    RemoteDirectoryEntry, RemoteDirectorySnapshot, RemoteEntryKind, RemoteEntryMutation,
    RemoteFingerprint, RemoteLoadedFile, RemoteSaveOutcome, SftpError, SftpOperation, SftpResponse,
};
use crate::terminal::{
    RemoteCommandOutput, RemoteCommandRequest, RemoteTerminalCommand, RemoteTerminalEndpoint,
    RemoteTerminalExecution, RemoteTerminalRequest, RemoteTerminalSession, finish_remote_terminal,
    remote_exec_command, remote_exec_command_with_environment, remote_shell_startup,
    terminal_error_output,
};

const AGENT_HOOK_ENDPOINT_ENV: &str = "YTTT_AGENT_HOOK_ENDPOINT";
const AGENT_HOOK_ENVIRONMENT_VARIABLES: [&str; 3] = [
    AGENT_HOOK_ENDPOINT_ENV,
    "YTTT_AGENT_HOOK_TOKEN",
    "YTTT_AGENT_HOOK_SCOPE",
];
const REMOTE_FORWARD_ADDRESS: &str = "127.0.0.1";

type ReverseForwardTargets = Arc<Mutex<HashMap<u32, SocketAddr>>>;

#[derive(Clone, Debug)]
struct RemoteAgentHookForward {
    address: &'static str,
    port: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SshEndpoint {
    pub host: String,
    pub port: u16,
    pub user: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredCredential {
    pub id: CredentialId,
    pub effective_user: String,
    pub resolved_host: String,
    pub port: u16,
    pub host_key_sha256: String,
    pub private_key_identity: Option<String>,
}

pub enum Authentication {
    Auto {
        identity_file: Option<PathBuf>,
        passphrase: Option<Zeroizing<String>>,
        credential: Option<StoredCredential>,
    },
    Agent,
    Password {
        secret: Zeroizing<String>,
        save_as: Option<CredentialId>,
    },
    StoredPassword(StoredCredential),
    PrivateKey {
        path: PathBuf,
        passphrase: Option<Zeroizing<String>>,
    },
}

impl fmt::Debug for Authentication {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Auto {
                identity_file,
                credential,
                ..
            } => formatter
                .debug_struct("Auto")
                .field("identity_file", identity_file)
                .field("credential", credential)
                .finish(),
            Self::Agent => formatter.write_str("Agent"),
            Self::Password { .. } => formatter.write_str("Password([REDACTED])"),
            Self::StoredPassword(credential) => formatter
                .debug_tuple("StoredPassword")
                .field(credential)
                .finish(),
            Self::PrivateKey { path, passphrase } => formatter
                .debug_struct("PrivateKey")
                .field("path", path)
                .field("passphrase", &passphrase.as_ref().map(|_| "[REDACTED]"))
                .finish(),
        }
    }
}

#[derive(Debug)]
pub struct ConnectRequest {
    pub connection_id: ConnectionId,
    pub endpoint: SshEndpoint,
    pub authentication: Authentication,
    pub reconnect: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ConnectionEpoch(u64);

impl ConnectionEpoch {
    pub fn get(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConnectionState {
    Disconnected,
    Connecting,
    VerifyingHostKey,
    Authenticating,
    Connected,
    Reconnecting,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConnectionStatus {
    pub connection_id: ConnectionId,
    pub epoch: ConnectionEpoch,
    pub state: ConnectionState,
    pub error: Option<String>,
}

#[derive(Debug)]
pub enum TransportEvent {
    StateChanged(ConnectionStatus),
    HostKeyChallenge(HostKeyChallenge),
    CredentialSaved {
        connection_id: ConnectionId,
        epoch: ConnectionEpoch,
        credential: StoredCredential,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HostKeyDecision {
    pub accept: bool,
    pub remember: bool,
}

pub struct HostKeyChallenge {
    pub connection_id: ConnectionId,
    pub epoch: ConnectionEpoch,
    pub host: String,
    pub port: u16,
    pub algorithm: String,
    pub fingerprint: String,
    pub previous_fingerprint: Option<String>,
    response: Option<oneshot::Sender<HostKeyDecision>>,
}

impl HostKeyChallenge {
    pub fn respond(mut self, decision: HostKeyDecision) -> Result<(), HostKeyDecision> {
        self.response
            .take()
            .expect("host-key challenge response sender is missing")
            .send(decision)
    }
}

impl fmt::Debug for HostKeyChallenge {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HostKeyChallenge")
            .field("connection_id", &self.connection_id)
            .field("epoch", &self.epoch)
            .field("host", &self.host)
            .field("port", &self.port)
            .field("algorithm", &self.algorithm)
            .field("fingerprint", &self.fingerprint)
            .field("previous_fingerprint", &self.previous_fingerprint)
            .finish_non_exhaustive()
    }
}

pub struct ConnectAttempt {
    epoch: ConnectionEpoch,
    completion: oneshot::Receiver<Result<ConnectionEpoch, TransportError>>,
}

impl ConnectAttempt {
    pub fn epoch(&self) -> ConnectionEpoch {
        self.epoch
    }

    pub async fn wait(self) -> Result<ConnectionEpoch, TransportError> {
        self.completion
            .await
            .map_err(|_| TransportError::RuntimeStopped)?
    }
}

pub trait HostTransportProxy: Send + Sync {
    fn request(&self, request: yttt_protocol::Request) -> Result<yttt_protocol::Response, String>;
    fn events(&self) -> flume::Receiver<yttt_protocol::ServerEvent>;
}

#[derive(Clone)]
pub struct TransportService {
    inner: Arc<TransportServiceInner>,
}

enum TransportServiceInner {
    Direct {
        commands: mpsc::UnboundedSender<RuntimeCommand>,
        events: EventReceiver<TransportEvent>,
        thread: Mutex<Option<thread::JoinHandle<()>>>,
    },
    Host {
        proxy: Arc<dyn HostTransportProxy>,
        events: EventReceiver<TransportEvent>,
        state: Arc<Mutex<HostProxyState>>,
        shutdown: Arc<std::sync::atomic::AtomicBool>,
        thread: Mutex<Option<thread::JoinHandle<()>>>,
    },
}

#[derive(Default)]
struct HostProxyState {
    attempts: HashMap<
        (ConnectionId, ConnectionEpoch),
        oneshot::Sender<Result<ConnectionEpoch, TransportError>>,
    >,
    statuses: HashMap<(ConnectionId, ConnectionEpoch), ConnectionStatus>,
}

impl TransportService {
    pub fn start(host_keys_path: impl Into<PathBuf>) -> Result<Self, TransportError> {
        Self::start_with_credential_store(host_keys_path, CredentialStore::default())
    }

    pub fn start_with_credential_namespace(
        host_keys_path: impl Into<PathBuf>,
        credential_namespace: impl Into<Arc<str>>,
    ) -> Result<Self, TransportError> {
        Self::start_with_credential_store(
            host_keys_path,
            CredentialStore::new(credential_namespace),
        )
    }

    pub fn start_with_credential_store(
        host_keys_path: impl Into<PathBuf>,
        credential_store: CredentialStore,
    ) -> Result<Self, TransportError> {
        let host_keys = Arc::new(Mutex::new(
            HostKeyStore::load(host_keys_path)
                .map_err(|error| TransportError::HostKeyStore(error.to_string()))?,
        ));
        let (commands, command_rx) = mpsc::unbounded_channel();
        let (events_tx, events) = async_channel::unbounded();
        let runtime_commands = commands.clone();
        let thread = thread::Builder::new()
            .name("yttt-ssh".to_string())
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("failed to create SSH Tokio runtime");
                runtime.block_on(runtime_loop(
                    command_rx,
                    runtime_commands,
                    events_tx,
                    host_keys,
                    credential_store,
                ));
            })
            .map_err(|source| TransportError::RuntimeStart(source.to_string()))?;
        Ok(Self {
            inner: Arc::new(TransportServiceInner::Direct {
                commands,
                events,
                thread: Mutex::new(Some(thread)),
            }),
        })
    }

    pub fn from_host(proxy: Arc<dyn HostTransportProxy>) -> Result<Self, TransportError> {
        let source = proxy.events();
        let (events_tx, events) = async_channel::unbounded();
        let state = Arc::new(Mutex::new(HostProxyState::default()));
        let shutdown = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let bridge_proxy = Arc::downgrade(&proxy);
        let bridge_state = state.clone();
        let bridge_shutdown = shutdown.clone();
        let thread = thread::Builder::new()
            .name("yttt-ssh-host-events".to_string())
            .spawn(move || {
                while !bridge_shutdown.load(std::sync::atomic::Ordering::Acquire) {
                    let event = match source.recv_timeout(Duration::from_millis(100)) {
                        Ok(event) => event,
                        Err(flume::RecvTimeoutError::Timeout) => continue,
                        Err(flume::RecvTimeoutError::Disconnected) => break,
                    };
                    let Some(proxy) = bridge_proxy.upgrade() else {
                        break;
                    };
                    bridge_host_event(event, &proxy, &bridge_state, &events_tx);
                }
            })
            .map_err(|source| TransportError::RuntimeStart(source.to_string()))?;
        Ok(Self {
            inner: Arc::new(TransportServiceInner::Host {
                proxy,
                events,
                state,
                shutdown,
                thread: Mutex::new(Some(thread)),
            }),
        })
    }

    pub fn events(&self) -> EventReceiver<TransportEvent> {
        match self.inner.as_ref() {
            TransportServiceInner::Direct { events, .. }
            | TransportServiceInner::Host { events, .. } => events.clone(),
        }
    }

    pub async fn start_connect(
        &self,
        request: ConnectRequest,
    ) -> Result<ConnectAttempt, TransportError> {
        match self.inner.as_ref() {
            TransportServiceInner::Direct { commands, .. } => {
                let (started_reply, started) = oneshot::channel();
                let (completion_reply, completion) = oneshot::channel();
                commands
                    .send(RuntimeCommand::Connect {
                        request,
                        started_reply,
                        completion_reply,
                    })
                    .map_err(|_| TransportError::RuntimeStopped)?;
                let epoch = started.await.map_err(|_| TransportError::RuntimeStopped)?;
                Ok(ConnectAttempt { epoch, completion })
            }
            TransportServiceInner::Host { proxy, state, .. } => {
                let connection_id = request.connection_id.clone();
                let response = proxy
                    .request(yttt_protocol::Request::SshConnect(wire_connect_request(
                        request,
                    )?))
                    .map_err(TransportError::Connection)?;
                let yttt_protocol::Response::SshConnected { epoch, .. } = response else {
                    return Err(TransportError::Connection(
                        "Host returned an unexpected SSH connect response".to_string(),
                    ));
                };
                let epoch = ConnectionEpoch(epoch);
                let (completion_reply, completion) = oneshot::channel();
                let key = (connection_id, epoch);
                let mut state = state.lock().map_err(|_| TransportError::RuntimeStopped)?;
                if let Some(status) = state.statuses.get(&key) {
                    if let Some(result) = completion_for_status(status) {
                        let _ = completion_reply.send(result);
                    } else {
                        state.attempts.insert(key, completion_reply);
                    }
                } else {
                    state.attempts.insert(key, completion_reply);
                }
                Ok(ConnectAttempt { epoch, completion })
            }
        }
    }

    pub async fn connect(
        &self,
        request: ConnectRequest,
    ) -> Result<ConnectionEpoch, TransportError> {
        self.start_connect(request).await?.wait().await
    }

    pub async fn disconnect(&self, connection_id: ConnectionId) -> Result<(), TransportError> {
        self.disconnect_inner(connection_id, None).await
    }

    pub async fn disconnect_attempt(
        &self,
        connection_id: ConnectionId,
        epoch: ConnectionEpoch,
    ) -> Result<(), TransportError> {
        self.disconnect_inner(connection_id, Some(epoch)).await
    }

    async fn disconnect_inner(
        &self,
        connection_id: ConnectionId,
        expected_epoch: Option<ConnectionEpoch>,
    ) -> Result<(), TransportError> {
        match self.inner.as_ref() {
            TransportServiceInner::Direct { commands, .. } => {
                let (reply, result) = oneshot::channel();
                commands
                    .send(RuntimeCommand::Disconnect {
                        connection_id,
                        expected_epoch,
                        reply,
                    })
                    .map_err(|_| TransportError::RuntimeStopped)?;
                result.await.map_err(|_| TransportError::RuntimeStopped)?
            }
            TransportServiceInner::Host { proxy, state, .. } => {
                if let Some(expected_epoch) = expected_epoch {
                    let state = state.lock().map_err(|_| TransportError::RuntimeStopped)?;
                    let latest = state
                        .statuses
                        .keys()
                        .filter(|(id, _)| id == &connection_id)
                        .map(|(_, epoch)| *epoch)
                        .max_by_key(|epoch| epoch.get());
                    if latest.is_some_and(|latest| latest != expected_epoch) {
                        return Err(TransportError::Superseded);
                    }
                }
                let response = proxy
                    .request(yttt_protocol::Request::SshDisconnect {
                        connection_id: connection_id.as_str().to_string(),
                    })
                    .map_err(TransportError::Connection)?;
                if matches!(response, yttt_protocol::Response::SshDisconnected) {
                    Ok(())
                } else {
                    Err(TransportError::Connection(
                        "Host returned an unexpected SSH disconnect response".to_string(),
                    ))
                }
            }
        }
    }

    pub fn sftp_project(&self, connection_id: ConnectionId, root: RemotePathBuf) -> SftpProject {
        SftpProject {
            inner: self.inner.clone(),
            connection_id,
            root,
        }
    }

    pub fn terminal_session(
        &self,
        request: RemoteTerminalRequest,
    ) -> Result<RemoteTerminalSession, TransportError> {
        let TransportServiceInner::Direct { commands, .. } = self.inner.as_ref() else {
            return Err(TransportError::Connection(
                "Host-backed SSH terminals must be spawned through the Host terminal API"
                    .to_string(),
            ));
        };
        let connection_id = request.connection_id.clone();
        let (session, endpoint) = RemoteTerminalSession::channel();
        commands
            .send(RuntimeCommand::Terminal {
                connection_id,
                request,
                endpoint,
            })
            .map_err(|_| TransportError::RuntimeStopped)?;
        Ok(session)
    }
}

fn wire_connect_request(
    request: ConnectRequest,
) -> Result<yttt_protocol::ssh::SshConnectSpec, TransportError> {
    Ok(yttt_protocol::ssh::SshConnectSpec {
        connection_id: request.connection_id.as_str().to_string(),
        endpoint: yttt_protocol::ssh::SshEndpoint {
            host: request.endpoint.host,
            port: request.endpoint.port,
            username: request.endpoint.user,
        },
        authentication: match request.authentication {
            Authentication::Auto {
                identity_file,
                passphrase,
                credential,
            } => yttt_protocol::ssh::SshAuthentication::Auto {
                identity_file: identity_file.map(|path| path.to_string_lossy().into_owned()),
                passphrase: passphrase.map(|secret| {
                    yttt_protocol::ssh::SensitiveBytes::new(secret.as_bytes().to_vec())
                }),
                credential: credential.map(wire_stored_credential),
            },
            Authentication::Agent => yttt_protocol::ssh::SshAuthentication::Agent,
            Authentication::Password { secret, save_as } => {
                yttt_protocol::ssh::SshAuthentication::Password {
                    secret: yttt_protocol::ssh::SensitiveBytes::new(secret.as_bytes().to_vec()),
                    save_as: save_as.map(|id| id.as_str().to_string()),
                }
            }
            Authentication::StoredPassword(credential) => {
                yttt_protocol::ssh::SshAuthentication::StoredPassword(wire_stored_credential(
                    credential,
                ))
            }
            Authentication::PrivateKey { path, passphrase } => {
                yttt_protocol::ssh::SshAuthentication::PrivateKey {
                    path: path.to_string_lossy().into_owned(),
                    passphrase: passphrase.map(|secret| {
                        yttt_protocol::ssh::SensitiveBytes::new(secret.as_bytes().to_vec())
                    }),
                }
            }
        },
        reconnect: request.reconnect,
    })
}

fn wire_stored_credential(credential: StoredCredential) -> yttt_protocol::ssh::StoredSshCredential {
    yttt_protocol::ssh::StoredSshCredential {
        id: credential.id.as_str().to_string(),
        effective_user: credential.effective_user,
        private_key_identity: credential.private_key_identity,
    }
}

fn stored_credential(credential: yttt_protocol::ssh::StoredSshCredential) -> StoredCredential {
    StoredCredential {
        id: CredentialId::new(credential.id),
        effective_user: credential.effective_user,
        resolved_host: String::new(),
        port: 0,
        host_key_sha256: String::new(),
        private_key_identity: credential.private_key_identity,
    }
}

fn bridge_host_event(
    event: yttt_protocol::ServerEvent,
    proxy: &Arc<dyn HostTransportProxy>,
    state: &Arc<Mutex<HostProxyState>>,
    events: &EventSender<TransportEvent>,
) {
    match event {
        yttt_protocol::ServerEvent::SshStateChanged(status) => {
            let status = ConnectionStatus {
                connection_id: ConnectionId::new(status.connection_id),
                epoch: ConnectionEpoch(status.epoch),
                state: connection_state(status.state),
                error: status.error,
            };
            let key = (status.connection_id.clone(), status.epoch);
            let completion = state.lock().ok().and_then(|mut state| {
                state.statuses.insert(key.clone(), status.clone());
                state.attempts.remove(&key)
            });
            if let Some(completion) = completion
                && let Some(result) = completion_for_status(&status)
            {
                let _ = completion.send(result);
            }
            let _ = events.try_send(TransportEvent::StateChanged(status));
        }
        yttt_protocol::ServerEvent::CredentialChallenge(challenge) => {
            let yttt_protocol::ssh::CredentialChallengeKind::HostKey {
                host,
                port,
                algorithm,
                fingerprint,
                previous_fingerprint,
            } = challenge.kind
            else {
                return;
            };
            let (response, decision) = oneshot::channel();
            let epoch = current_epoch(state, challenge.connection_id.as_str());
            let event = TransportEvent::HostKeyChallenge(HostKeyChallenge {
                connection_id: ConnectionId::new(challenge.connection_id),
                epoch,
                host,
                port,
                algorithm,
                fingerprint,
                previous_fingerprint,
                response: Some(response),
            });
            let _ = events.try_send(event);
            let proxy = Arc::downgrade(proxy);
            thread::spawn(move || {
                let Ok(decision) = decision.blocking_recv() else {
                    return;
                };
                let Some(proxy) = proxy.upgrade() else {
                    return;
                };
                let answer = if !decision.accept {
                    yttt_protocol::ssh::HostKeyDecision::Reject
                } else if decision.remember {
                    yttt_protocol::ssh::HostKeyDecision::AcceptAndStore
                } else {
                    yttt_protocol::ssh::HostKeyDecision::AcceptOnce
                };
                let _ = proxy.request(yttt_protocol::Request::CredentialAnswer {
                    challenge_id: challenge.challenge_id,
                    answer: yttt_protocol::ssh::CredentialAnswer::HostKey(answer),
                });
            });
        }
        yttt_protocol::ServerEvent::SshCredentialSaved {
            connection_id,
            epoch,
            credential,
        } => {
            let _ = events.try_send(TransportEvent::CredentialSaved {
                connection_id: ConnectionId::new(connection_id),
                epoch: ConnectionEpoch(epoch),
                credential: stored_credential(credential),
            });
        }
        _ => {}
    }
}

fn current_epoch(state: &Arc<Mutex<HostProxyState>>, connection_id: &str) -> ConnectionEpoch {
    state
        .lock()
        .ok()
        .and_then(|state| {
            state
                .statuses
                .keys()
                .filter(|(id, _)| id.as_str() == connection_id)
                .map(|(_, epoch)| *epoch)
                .max_by_key(|epoch| epoch.get())
        })
        .unwrap_or(ConnectionEpoch(0))
}

fn completion_for_status(
    status: &ConnectionStatus,
) -> Option<Result<ConnectionEpoch, TransportError>> {
    match status.state {
        ConnectionState::Connected => Some(Ok(status.epoch)),
        ConnectionState::Disconnected | ConnectionState::Failed => {
            Some(Err(TransportError::Connection(
                status
                    .error
                    .clone()
                    .unwrap_or_else(|| "SSH connection closed".to_string()),
            )))
        }
        ConnectionState::Connecting
        | ConnectionState::VerifyingHostKey
        | ConnectionState::Authenticating
        | ConnectionState::Reconnecting => None,
    }
}

fn connection_state(state: yttt_protocol::ssh::SshConnectionState) -> ConnectionState {
    match state {
        yttt_protocol::ssh::SshConnectionState::Disconnected => ConnectionState::Disconnected,
        yttt_protocol::ssh::SshConnectionState::Connecting => ConnectionState::Connecting,
        yttt_protocol::ssh::SshConnectionState::VerifyingHostKey => {
            ConnectionState::VerifyingHostKey
        }
        yttt_protocol::ssh::SshConnectionState::Authenticating => ConnectionState::Authenticating,
        yttt_protocol::ssh::SshConnectionState::Connected => ConnectionState::Connected,
        yttt_protocol::ssh::SshConnectionState::Reconnecting => ConnectionState::Reconnecting,
        yttt_protocol::ssh::SshConnectionState::Failed => ConnectionState::Failed,
    }
}

#[derive(Clone)]
pub struct SftpProject {
    inner: Arc<TransportServiceInner>,
    connection_id: ConnectionId,
    root: RemotePathBuf,
}

impl SftpProject {
    pub fn root(&self) -> &RemotePathBuf {
        &self.root
    }
    pub fn resolve_home(&self) -> Result<RemotePathBuf, SftpError> {
        match self.request(SftpOperation::ResolveHome)? {
            SftpResponse::Path(path) => Ok(path),
            _ => Err(SftpError::UnexpectedResponse),
        }
    }

    pub fn run_command(
        &self,
        program: impl Into<String>,
        args: Vec<String>,
    ) -> Result<RemoteCommandOutput, TransportError> {
        match self.inner.as_ref() {
            TransportServiceInner::Direct { commands, .. } => {
                let (reply, result) = blocking_mpsc::channel();
                commands
                    .send(RuntimeCommand::Execute {
                        connection_id: self.connection_id.clone(),
                        request: RemoteCommandRequest {
                            cwd: self.root.clone(),
                            program: program.into(),
                            args,
                        },
                        reply,
                    })
                    .map_err(|_| TransportError::RuntimeStopped)?;
                result
                    .recv_timeout(Duration::from_secs(120))
                    .map_err(|error| match error {
                        blocking_mpsc::RecvTimeoutError::Timeout => TransportError::RequestTimedOut,
                        blocking_mpsc::RecvTimeoutError::Disconnected => {
                            TransportError::RuntimeStopped
                        }
                    })?
            }
            TransportServiceInner::Host { .. } => Err(TransportError::Connection(
                "Host-backed SSH project commands must use the Host project API".to_string(),
            )),
        }
    }

    pub fn scan_directory(
        &self,
        relative_directory: RemoteRelativePathBuf,
        show_hidden: bool,
    ) -> Result<RemoteDirectorySnapshot, SftpError> {
        match self.request(SftpOperation::ScanDirectory {
            relative_directory,
            show_hidden,
        })? {
            SftpResponse::Directory(snapshot) => Ok(snapshot),
            _ => Err(SftpError::UnexpectedResponse),
        }
    }

    pub fn read_file(
        &self,
        relative_path: RemoteRelativePathBuf,
        max_bytes: u64,
    ) -> Result<RemoteLoadedFile, SftpError> {
        match self.request(SftpOperation::ReadFile {
            relative_path,
            max_bytes,
        })? {
            SftpResponse::File(file) => Ok(file),
            _ => Err(SftpError::UnexpectedResponse),
        }
    }

    pub fn save_file(
        &self,
        relative_path: RemoteRelativePathBuf,
        bytes: Vec<u8>,
        expected: Option<RemoteFingerprint>,
        force: bool,
        max_bytes: u64,
    ) -> Result<RemoteSaveOutcome, SftpError> {
        match self.request(SftpOperation::SaveFile {
            relative_path,
            bytes,
            expected,
            force,
            max_bytes,
        })? {
            SftpResponse::Save(outcome) => Ok(outcome),
            _ => Err(SftpError::UnexpectedResponse),
        }
    }

    pub fn create_entry(
        &self,
        relative_path: RemoteRelativePathBuf,
        directory: bool,
    ) -> Result<RemoteEntryMutation, SftpError> {
        match self.request(SftpOperation::CreateEntry {
            relative_path,
            directory,
        })? {
            SftpResponse::Mutation(mutation) => Ok(mutation),
            _ => Err(SftpError::UnexpectedResponse),
        }
    }

    pub fn rename_entry(
        &self,
        relative_path: RemoteRelativePathBuf,
        new_name: String,
    ) -> Result<RemoteEntryMutation, SftpError> {
        match self.request(SftpOperation::RenameEntry {
            relative_path,
            new_name,
        })? {
            SftpResponse::Mutation(mutation) => Ok(mutation),
            _ => Err(SftpError::UnexpectedResponse),
        }
    }

    pub fn delete_entry(&self, relative_path: RemoteRelativePathBuf) -> Result<(), SftpError> {
        match self.request(SftpOperation::DeleteEntry { relative_path })? {
            SftpResponse::Deleted => Ok(()),
            _ => Err(SftpError::UnexpectedResponse),
        }
    }

    fn request(&self, operation: SftpOperation) -> Result<SftpResponse, SftpError> {
        match self.inner.as_ref() {
            TransportServiceInner::Direct { commands, .. } => {
                let (reply, result) = blocking_mpsc::channel();
                commands
                    .send(RuntimeCommand::Sftp {
                        connection_id: self.connection_id.clone(),
                        root: self.root.clone(),
                        operation,
                        reply,
                    })
                    .map_err(|_| SftpError::RuntimeStopped)?;
                result
                    .recv_timeout(Duration::from_secs(120))
                    .map_err(|error| match error {
                        blocking_mpsc::RecvTimeoutError::Timeout => SftpError::TimedOut,
                        blocking_mpsc::RecvTimeoutError::Disconnected => SftpError::RuntimeStopped,
                    })?
            }
            TransportServiceInner::Host { proxy, .. } => {
                host_sftp_request(proxy, &self.connection_id, &self.root, operation)
            }
        }
    }
}

fn host_sftp_request(
    proxy: &Arc<dyn HostTransportProxy>,
    connection_id: &ConnectionId,
    root: &RemotePathBuf,
    operation: SftpOperation,
) -> Result<SftpResponse, SftpError> {
    let request = match operation {
        SftpOperation::ResolveHome => yttt_protocol::ssh::RemoteFileRequest::ResolveHome {
            connection_id: connection_id.as_str().to_string(),
        },
        SftpOperation::ScanDirectory {
            relative_directory,
            show_hidden,
        } => yttt_protocol::ssh::RemoteFileRequest::BrowseDirectory {
            connection_id: connection_id.as_str().to_string(),
            root: root.as_str().to_string(),
            relative_directory: relative_directory.as_str().to_string(),
            show_hidden,
        },
        SftpOperation::ReadFile { .. }
        | SftpOperation::SaveFile { .. }
        | SftpOperation::CreateEntry { .. }
        | SftpOperation::RenameEntry { .. }
        | SftpOperation::DeleteEntry { .. } => {
            return Err(SftpError::Protocol(
                "Host-backed SSH project operations must use the Host project API".to_string(),
            ));
        }
    };
    let response = proxy
        .request(yttt_protocol::Request::RemoteFile(request))
        .map_err(SftpError::Protocol)?;
    let yttt_protocol::Response::RemoteFile(response) = response else {
        return Err(SftpError::UnexpectedResponse);
    };
    match response {
        yttt_protocol::ssh::RemoteFileResponse::Home(path) => RemotePathBuf::new(path)
            .map(SftpResponse::Path)
            .map_err(|error| SftpError::InvalidPath(error.to_string())),
        yttt_protocol::ssh::RemoteFileResponse::Directory(directory) => {
            let relative_directory = RemoteRelativePathBuf::new(directory.relative_directory)
                .map_err(|error| SftpError::InvalidPath(error.to_string()))?;
            let entries = directory
                .entries
                .into_iter()
                .map(|entry| {
                    Ok(RemoteDirectoryEntry {
                        name: entry.name,
                        relative_path: RemoteRelativePathBuf::new(entry.relative_path)
                            .map_err(|error| SftpError::InvalidPath(error.to_string()))?,
                        kind: remote_entry_kind(entry.kind),
                    })
                })
                .collect::<Result<Vec<_>, SftpError>>()?;
            Ok(SftpResponse::Directory(RemoteDirectorySnapshot {
                relative_directory,
                entries,
            }))
        }
        _ => Err(SftpError::UnexpectedResponse),
    }
}

fn remote_entry_kind(kind: yttt_protocol::ssh::RemoteFileKind) -> RemoteEntryKind {
    match kind {
        yttt_protocol::ssh::RemoteFileKind::File => RemoteEntryKind::File,
        yttt_protocol::ssh::RemoteFileKind::Directory => RemoteEntryKind::Directory,
        yttt_protocol::ssh::RemoteFileKind::SymlinkFile => RemoteEntryKind::SymlinkFile,
        yttt_protocol::ssh::RemoteFileKind::SymlinkDirectory => RemoteEntryKind::SymlinkDirectory,
    }
}

impl Drop for TransportServiceInner {
    fn drop(&mut self) {
        match self {
            Self::Direct {
                commands, thread, ..
            } => {
                let _ = commands.send(RuntimeCommand::Shutdown);
                if let Some(thread) = thread.get_mut().ok().and_then(|thread| thread.take()) {
                    let _ = thread.join();
                }
            }
            Self::Host {
                shutdown, thread, ..
            } => {
                shutdown.store(true, std::sync::atomic::Ordering::Release);
                if let Some(thread) = thread.get_mut().ok().and_then(|thread| thread.take()) {
                    let _ = thread.join();
                }
            }
        }
    }
}

struct ConnectionSlot {
    epoch: ConnectionEpoch,
    actor: Option<mpsc::UnboundedSender<ConnectionCommand>>,
}

enum RuntimeCommand {
    Connect {
        request: ConnectRequest,
        started_reply: oneshot::Sender<ConnectionEpoch>,
        completion_reply: oneshot::Sender<Result<ConnectionEpoch, TransportError>>,
    },
    ConnectCompleted {
        connection_id: ConnectionId,
        epoch: ConnectionEpoch,
        outcome: Result<mpsc::UnboundedSender<ConnectionCommand>, TransportError>,
        reply: oneshot::Sender<Result<ConnectionEpoch, TransportError>>,
    },
    Disconnect {
        connection_id: ConnectionId,
        expected_epoch: Option<ConnectionEpoch>,
        reply: oneshot::Sender<Result<(), TransportError>>,
    },
    ActorExited {
        connection_id: ConnectionId,
        epoch: ConnectionEpoch,
        error: Option<String>,
    },
    Sftp {
        connection_id: ConnectionId,
        root: RemotePathBuf,
        operation: SftpOperation,
        reply: blocking_mpsc::Sender<Result<SftpResponse, SftpError>>,
    },
    Execute {
        connection_id: ConnectionId,
        request: RemoteCommandRequest,
        reply: blocking_mpsc::Sender<Result<RemoteCommandOutput, TransportError>>,
    },
    Terminal {
        connection_id: ConnectionId,
        request: RemoteTerminalRequest,
        endpoint: RemoteTerminalEndpoint,
    },
    Shutdown,
}

enum ConnectionCommand {
    Sftp {
        root: RemotePathBuf,
        operation: SftpOperation,
        reply: blocking_mpsc::Sender<Result<SftpResponse, SftpError>>,
    },
    Execute {
        request: RemoteCommandRequest,
        reply: blocking_mpsc::Sender<Result<RemoteCommandOutput, TransportError>>,
    },
    Terminal {
        request: RemoteTerminalRequest,
        endpoint: RemoteTerminalEndpoint,
    },
    CancelReverseForward(RemoteAgentHookForward),
    Disconnect,
}

#[derive(Clone)]
struct AuthenticationRuntime {
    credential_store: CredentialStore,
    events: EventSender<TransportEvent>,
}

async fn runtime_loop(
    mut commands: mpsc::UnboundedReceiver<RuntimeCommand>,
    runtime_commands: mpsc::UnboundedSender<RuntimeCommand>,
    events: EventSender<TransportEvent>,
    host_keys: Arc<Mutex<HostKeyStore>>,
    credential_store: CredentialStore,
) {
    let mut slots = HashMap::<ConnectionId, ConnectionSlot>::new();
    while let Some(command) = commands.recv().await {
        match command {
            RuntimeCommand::Connect {
                request,
                started_reply,
                completion_reply,
            } => {
                let connection_id = request.connection_id.clone();
                let epoch = ConnectionEpoch(
                    slots
                        .get(&connection_id)
                        .map_or(1, |slot| slot.epoch.0.saturating_add(1)),
                );
                if let Some(actor) = slots
                    .get(&connection_id)
                    .and_then(|slot| slot.actor.as_ref())
                {
                    let _ = actor.send(ConnectionCommand::Disconnect);
                }
                slots.insert(connection_id.clone(), ConnectionSlot { epoch, actor: None });
                let _ = started_reply.send(epoch);
                send_state(
                    &events,
                    &connection_id,
                    epoch,
                    if request.reconnect {
                        ConnectionState::Reconnecting
                    } else {
                        ConnectionState::Connecting
                    },
                    None,
                );
                let runtime_commands = runtime_commands.clone();
                let host_keys = host_keys.clone();
                let authentication_runtime = AuthenticationRuntime {
                    credential_store: credential_store.clone(),
                    events: events.clone(),
                };
                tokio::spawn(async move {
                    let outcome = connect_one(
                        connection_id.clone(),
                        epoch,
                        request.endpoint,
                        request.authentication,
                        host_keys,
                        runtime_commands.clone(),
                        authentication_runtime,
                    )
                    .await;
                    let _ = runtime_commands.send(RuntimeCommand::ConnectCompleted {
                        connection_id,
                        epoch,
                        outcome,
                        reply: completion_reply,
                    });
                });
            }
            RuntimeCommand::ConnectCompleted {
                connection_id,
                epoch,
                outcome,
                reply,
            } => {
                let current = slots
                    .get(&connection_id)
                    .is_some_and(|slot| slot.epoch == epoch);
                if !current {
                    if let Ok(actor) = outcome {
                        let _ = actor.send(ConnectionCommand::Disconnect);
                    }
                    let _ = reply.send(Err(TransportError::Superseded));
                    continue;
                }
                match outcome {
                    Ok(actor) => {
                        slots
                            .get_mut(&connection_id)
                            .expect("slot checked above")
                            .actor = Some(actor);
                        send_state(
                            &events,
                            &connection_id,
                            epoch,
                            ConnectionState::Connected,
                            None,
                        );
                        let _ = reply.send(Ok(epoch));
                    }
                    Err(error) => {
                        slots
                            .get_mut(&connection_id)
                            .expect("slot checked above")
                            .actor = None;
                        send_state(
                            &events,
                            &connection_id,
                            epoch,
                            ConnectionState::Failed,
                            Some(error.to_string()),
                        );
                        let _ = reply.send(Err(error));
                    }
                }
            }
            RuntimeCommand::Disconnect {
                connection_id,
                expected_epoch,
                reply,
            } => {
                if let Some(slot) = slots.get_mut(&connection_id)
                    && expected_epoch.is_none_or(|epoch| slot.epoch == epoch)
                {
                    slot.epoch = ConnectionEpoch(slot.epoch.0.saturating_add(1));
                    if let Some(actor) = slot.actor.take() {
                        let _ = actor.send(ConnectionCommand::Disconnect);
                    }
                    send_state(
                        &events,
                        &connection_id,
                        slot.epoch,
                        ConnectionState::Disconnected,
                        None,
                    );
                }
                let _ = reply.send(Ok(()));
            }
            RuntimeCommand::ActorExited {
                connection_id,
                epoch,
                error,
            } => {
                let Some(slot) = slots.get_mut(&connection_id) else {
                    continue;
                };
                if slot.epoch != epoch {
                    continue;
                }
                slot.actor = None;
                send_state(
                    &events,
                    &connection_id,
                    epoch,
                    if error.is_some() {
                        ConnectionState::Failed
                    } else {
                        ConnectionState::Disconnected
                    },
                    error,
                );
            }
            RuntimeCommand::Sftp {
                connection_id,
                root,
                operation,
                reply,
            } => {
                let Some(actor) = slots
                    .get(&connection_id)
                    .and_then(|slot| slot.actor.as_ref())
                else {
                    let _ = reply.send(Err(SftpError::NotConnected));
                    continue;
                };
                if let Err(error) = actor.send(ConnectionCommand::Sftp {
                    root,
                    operation,
                    reply,
                }) {
                    let ConnectionCommand::Sftp { reply, .. } = error.0 else {
                        unreachable!("only SFTP commands are sent in this branch");
                    };
                    let _ = reply.send(Err(SftpError::NotConnected));
                }
            }
            RuntimeCommand::Execute {
                connection_id,
                request,
                reply,
            } => {
                let Some(actor) = slots
                    .get(&connection_id)
                    .and_then(|slot| slot.actor.as_ref())
                else {
                    let _ = reply.send(Err(TransportError::NotConnected));
                    continue;
                };
                if let Err(error) = actor.send(ConnectionCommand::Execute { request, reply }) {
                    let ConnectionCommand::Execute { reply, .. } = error.0 else {
                        unreachable!("only command execution requests are sent in this branch");
                    };
                    let _ = reply.send(Err(TransportError::NotConnected));
                }
            }
            RuntimeCommand::Terminal {
                connection_id,
                request,
                endpoint,
            } => {
                let Some(actor) = slots
                    .get(&connection_id)
                    .and_then(|slot| slot.actor.as_ref())
                else {
                    fail_remote_terminal(endpoint, "SSH connection is not connected");
                    continue;
                };
                if let Err(error) = actor.send(ConnectionCommand::Terminal { request, endpoint }) {
                    let ConnectionCommand::Terminal { endpoint, .. } = error.0 else {
                        unreachable!("only terminal commands are sent in this branch");
                    };
                    fail_remote_terminal(endpoint, "SSH connection is not connected");
                }
            }
            RuntimeCommand::Shutdown => {
                for slot in slots.values_mut() {
                    if let Some(actor) = slot.actor.take() {
                        let _ = actor.send(ConnectionCommand::Disconnect);
                    }
                }
                break;
            }
        }
    }
}

async fn connect_one(
    connection_id: ConnectionId,
    epoch: ConnectionEpoch,
    endpoint: SshEndpoint,
    authentication: Authentication,
    host_keys: Arc<Mutex<HostKeyStore>>,
    runtime_commands: mpsc::UnboundedSender<RuntimeCommand>,
    authentication_runtime: AuthenticationRuntime,
) -> Result<mpsc::UnboundedSender<ConnectionCommand>, TransportError> {
    send_state(
        &authentication_runtime.events,
        &connection_id,
        epoch,
        ConnectionState::VerifyingHostKey,
        None,
    );
    let config = Arc::new(client::Config {
        inactivity_timeout: Some(Duration::from_secs(60)),
        keepalive_interval: Some(Duration::from_secs(15)),
        keepalive_max: 3,
        nodelay: true,
        ..client::Config::default()
    });
    let verified_host_key = Arc::new(Mutex::new(None));
    let reverse_forward_targets = ReverseForwardTargets::default();
    let handler = HostKeyHandler {
        connection_id: connection_id.clone(),
        epoch,
        endpoint: endpoint.clone(),
        events: authentication_runtime.events.clone(),
        host_keys,
        verified_host_key: verified_host_key.clone(),
        reverse_forward_targets: reverse_forward_targets.clone(),
    };
    let mut session = client::connect(config, (endpoint.host.as_str(), endpoint.port), handler)
        .await
        .map_err(normalize_connect_error)?;
    send_state(
        &authentication_runtime.events,
        &connection_id,
        epoch,
        ConnectionState::Authenticating,
        None,
    );
    let host_key_sha256 = verified_host_key
        .lock()
        .map_err(|_| TransportError::MissingVerifiedHostKey)?
        .clone()
        .ok_or(TransportError::MissingVerifiedHostKey)?;
    authenticate(
        &mut session,
        &connection_id,
        epoch,
        &endpoint,
        &host_key_sha256,
        authentication,
        &authentication_runtime,
    )
    .await?;

    let channel = session.channel_open_session().await?;
    channel.request_subsystem(true, "sftp").await?;
    let sftp = Arc::new(
        SftpSession::new(channel.into_stream())
            .await
            .map_err(|error| TransportError::Sftp(error.to_string()))?,
    );
    sftp.set_timeout(60);

    let (actor, actor_rx) = mpsc::unbounded_channel();
    tokio::spawn(connection_loop(
        session,
        sftp,
        actor_rx,
        ConnectionLoopContext {
            connection_id,
            epoch,
            connection_commands: actor.clone(),
            reverse_forward_targets,
            runtime_commands,
        },
    ));
    Ok(actor)
}

async fn authenticate(
    session: &mut client::Handle<HostKeyHandler>,
    connection_id: &ConnectionId,
    epoch: ConnectionEpoch,
    endpoint: &SshEndpoint,
    host_key_sha256: &str,
    authentication: Authentication,
    runtime: &AuthenticationRuntime,
) -> Result<(), TransportError> {
    let authenticated = match authentication {
        Authentication::Auto {
            identity_file,
            passphrase,
            credential,
        } => {
            let mut authenticated = authenticate_with_agent(session, &endpoint.user)
                .await
                .unwrap_or(false);
            if !authenticated && let Some(path) = identity_file {
                authenticated =
                    authenticate_with_private_key(session, &endpoint.user, path, passphrase)
                        .await
                        .unwrap_or(false);
            }
            if !authenticated && let Some(credential) = credential {
                authenticated = authenticate_with_stored_password(
                    session,
                    &endpoint.user,
                    credential,
                    endpoint,
                    host_key_sha256,
                    &runtime.credential_store,
                )
                .await?;
            }
            authenticated
        }
        Authentication::Password { secret, save_as } => {
            let authenticated = session
                .authenticate_password(&endpoint.user, secret.as_str())
                .await
                .map_err(TransportError::from)?
                .success();
            if authenticated && let Some(credential_id) = save_as {
                runtime
                    .credential_store
                    .save(&credential_id, secret.as_str())
                    .map_err(|source| TransportError::Credential(source.to_string()))?;
                runtime
                    .events
                    .send(TransportEvent::CredentialSaved {
                        connection_id: connection_id.clone(),
                        epoch,
                        credential: StoredCredential {
                            id: credential_id,
                            effective_user: endpoint.user.clone(),
                            resolved_host: endpoint.host.clone(),
                            port: endpoint.port,
                            host_key_sha256: host_key_sha256.to_string(),
                            private_key_identity: None,
                        },
                    })
                    .await
                    .map_err(|_| TransportError::RuntimeStopped)?;
            }
            authenticated
        }
        Authentication::StoredPassword(credential) => {
            authenticate_with_stored_password(
                session,
                &endpoint.user,
                credential,
                endpoint,
                host_key_sha256,
                &runtime.credential_store,
            )
            .await?
        }
        Authentication::PrivateKey { path, passphrase } => {
            authenticate_with_private_key(session, &endpoint.user, path, passphrase).await?
        }
        Authentication::Agent => authenticate_with_agent(session, &endpoint.user).await?,
    };
    if authenticated {
        Ok(())
    } else {
        Err(TransportError::AuthenticationRejected)
    }
}

async fn authenticate_with_stored_password(
    session: &mut client::Handle<HostKeyHandler>,
    user: &str,
    credential: StoredCredential,
    endpoint: &SshEndpoint,
    host_key_sha256: &str,
    credential_store: &CredentialStore,
) -> Result<bool, TransportError> {
    if !credential.resolved_host.is_empty()
        && (credential.effective_user != endpoint.user
            || credential.resolved_host != endpoint.host
            || credential.port != endpoint.port
            || credential.host_key_sha256 != host_key_sha256)
    {
        return Err(TransportError::CredentialBindingMismatch(credential.id));
    }
    let password = credential_store
        .load(&credential.id)
        .map_err(|source| TransportError::Credential(source.to_string()))?
        .ok_or_else(|| TransportError::CredentialMissing(credential.id.clone()))?;
    Ok(session
        .authenticate_password(user, password.as_str())
        .await
        .map_err(TransportError::from)?
        .success())
}

async fn authenticate_with_private_key(
    session: &mut client::Handle<HostKeyHandler>,
    user: &str,
    path: PathBuf,
    passphrase: Option<Zeroizing<String>>,
) -> Result<bool, TransportError> {
    let key = russh::keys::load_secret_key(&path, passphrase.as_deref().map(String::as_str))
        .map_err(|source| TransportError::PrivateKey {
            path,
            message: source.to_string(),
        })?;
    let hash = session
        .best_supported_rsa_hash()
        .await
        .map_err(TransportError::from)?
        .flatten();
    Ok(session
        .authenticate_publickey(user, PrivateKeyWithHashAlg::new(Arc::new(key), hash))
        .await
        .map_err(TransportError::from)?
        .success())
}

async fn authenticate_with_agent(
    session: &mut client::Handle<HostKeyHandler>,
    user: &str,
) -> Result<bool, TransportError> {
    #[cfg(unix)]
    let mut agent = AgentClient::connect_env()
        .await
        .map_err(|source| TransportError::Agent(source.to_string()))?;
    #[cfg(windows)]
    let mut agent = match AgentClient::connect_named_pipe(r"\\.\pipe\openssh-ssh-agent").await {
        Ok(agent) => agent.dynamic(),
        Err(named_pipe_error) => AgentClient::connect_pageant()
            .await
            .map(|agent| agent.dynamic())
            .map_err(|pageant_error| {
                TransportError::Agent(format!(
                    "OpenSSH agent: {named_pipe_error}; Pageant: {pageant_error}"
                ))
            })?,
    };
    let identities = agent
        .request_identities()
        .await
        .map_err(|source| TransportError::Agent(source.to_string()))?;
    for identity in identities {
        let public_key = identity.public_key().into_owned();
        let hash = session
            .best_supported_rsa_hash()
            .await
            .map_err(TransportError::from)?
            .flatten();
        let result = session
            .authenticate_publickey_with(user, public_key, hash, &mut agent)
            .await
            .map_err(|source| TransportError::Agent(source.to_string()))?;
        if result.success() {
            return Ok(true);
        }
    }
    Ok(false)
}

struct ConnectionLoopContext {
    connection_id: ConnectionId,
    epoch: ConnectionEpoch,
    connection_commands: mpsc::UnboundedSender<ConnectionCommand>,
    reverse_forward_targets: ReverseForwardTargets,
    runtime_commands: mpsc::UnboundedSender<RuntimeCommand>,
}

async fn connection_loop(
    session: client::Handle<HostKeyHandler>,
    sftp: Arc<SftpSession>,
    mut commands: mpsc::UnboundedReceiver<ConnectionCommand>,
    context: ConnectionLoopContext,
) {
    let mut session = Box::pin(session);
    let error = loop {
        tokio::select! {
            result = &mut session => break result.err().map(|error| error.to_string()),
            command = commands.recv() => match command {
                Some(ConnectionCommand::Sftp {
                    root,
                    operation,
                    reply,
                }) => {
                    let sftp = sftp.clone();
                    tokio::spawn(async move {
                        let _ = reply.send(crate::sftp::execute(&sftp, &root, operation).await);
                    });
                }
                Some(ConnectionCommand::Execute { request, reply }) => {
                    match session.as_ref().get_ref().channel_open_session().await {
                        Ok(channel) => {
                            tokio::spawn(async move {
                                let _ = reply.send(run_remote_command(channel, request).await);
                            });
                        }
                        Err(error) => {
                            let _ = reply.send(Err(TransportError::Connection(format!(
                                "failed to open remote command: {error}"
                            ))));
                        }
                    }
                }
                Some(ConnectionCommand::Terminal {
                    mut request,
                    endpoint,
                }) => {
                    let agent_hook_forward = prepare_remote_agent_hook_forward(
                        session.as_ref().get_ref(),
                        &mut request,
                        &context.reverse_forward_targets,
                    )
                    .await;
                    match session.as_ref().get_ref().channel_open_session().await {
                        Ok(channel) => {
                            let connection_commands = context.connection_commands.clone();
                            tokio::spawn(async move {
                                run_remote_terminal(channel, request, endpoint).await;
                                if let Some(forward) = agent_hook_forward {
                                    let _ = connection_commands
                                        .send(ConnectionCommand::CancelReverseForward(forward));
                                }
                            });
                        }
                        Err(error) => {
                            if let Some(forward) = agent_hook_forward {
                                cancel_remote_agent_hook_forward(
                                    session.as_ref().get_ref(),
                                    &context.reverse_forward_targets,
                                    &forward,
                                )
                                .await;
                            }
                            fail_remote_terminal(
                                endpoint,
                                &format!("failed to open remote terminal: {error}"),
                            );
                        }
                    }
                }
                Some(ConnectionCommand::CancelReverseForward(forward)) => {
                    cancel_remote_agent_hook_forward(
                        session.as_ref().get_ref(),
                        &context.reverse_forward_targets,
                        &forward,
                    )
                    .await;
                }
                Some(ConnectionCommand::Disconnect) | None => {
                    let _ = sftp.close().await;
                    let _ = session.as_ref().get_ref().disconnect(
                        russh::Disconnect::ByApplication,
                        "yttt disconnect",
                        "en",
                    ).await;
                    break None;
                }
            }
        }
    };
    let _ = context.runtime_commands.send(RuntimeCommand::ActorExited {
        connection_id: context.connection_id,
        epoch: context.epoch,
        error,
    });
}

fn loopback_http_target(endpoint: &str) -> Option<SocketAddr> {
    let target = endpoint
        .strip_prefix("http://")?
        .parse::<SocketAddr>()
        .ok()?;
    target.ip().is_loopback().then_some(target)
}

fn remove_agent_hook_environment(request: &mut RemoteTerminalRequest) {
    for name in AGENT_HOOK_ENVIRONMENT_VARIABLES {
        request.environment.remove(name);
    }
}

async fn prepare_remote_agent_hook_forward(
    session: &client::Handle<HostKeyHandler>,
    request: &mut RemoteTerminalRequest,
    targets: &ReverseForwardTargets,
) -> Option<RemoteAgentHookForward> {
    let Some(target) = request
        .environment
        .get(AGENT_HOOK_ENDPOINT_ENV)
        .and_then(|endpoint| loopback_http_target(endpoint))
    else {
        remove_agent_hook_environment(request);
        return None;
    };
    let port = match session.tcpip_forward(REMOTE_FORWARD_ADDRESS, 0).await {
        Ok(port) if port != 0 => port,
        Ok(_) | Err(_) => {
            remove_agent_hook_environment(request);
            return None;
        }
    };
    if let Ok(mut targets) = targets.lock() {
        targets.insert(port, target);
    } else {
        let _ = session
            .cancel_tcpip_forward(REMOTE_FORWARD_ADDRESS, port)
            .await;
        remove_agent_hook_environment(request);
        return None;
    }
    request.environment.insert(
        AGENT_HOOK_ENDPOINT_ENV.to_string(),
        format!("http://{REMOTE_FORWARD_ADDRESS}:{port}"),
    );
    Some(RemoteAgentHookForward {
        address: REMOTE_FORWARD_ADDRESS,
        port,
    })
}

async fn cancel_remote_agent_hook_forward(
    session: &client::Handle<HostKeyHandler>,
    targets: &ReverseForwardTargets,
    forward: &RemoteAgentHookForward,
) {
    if let Ok(mut targets) = targets.lock() {
        targets.remove(&forward.port);
    }
    let _ = session
        .cancel_tcpip_forward(forward.address, forward.port)
        .await;
}

async fn run_remote_command(
    mut channel: russh::Channel<russh::client::Msg>,
    request: RemoteCommandRequest,
) -> Result<RemoteCommandOutput, TransportError> {
    channel
        .exec(
            true,
            remote_exec_command(&request.cwd, &request.program, &request.args),
        )
        .await
        .map_err(|error| {
            TransportError::Connection(format!("failed to start remote command: {error}"))
        })?;
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut exit_status = None;
    while let Some(message) = channel.wait().await {
        match message {
            ChannelMsg::Data { data } => stdout.extend_from_slice(&data),
            ChannelMsg::ExtendedData { data, .. } => stderr.extend_from_slice(&data),
            ChannelMsg::ExitStatus {
                exit_status: status,
            } => exit_status = i32::try_from(status).ok(),
            ChannelMsg::Eof => {}
            ChannelMsg::Close => break,
            _ => {}
        }
    }
    Ok(RemoteCommandOutput {
        stdout,
        stderr,
        exit_status,
    })
}

fn fail_remote_terminal(endpoint: RemoteTerminalEndpoint, message: &str) {
    let _ = endpoint.output.send(terminal_error_output(message));
    finish_remote_terminal(&endpoint.state, None);
}

async fn run_remote_terminal(
    mut channel: russh::Channel<russh::client::Msg>,
    request: RemoteTerminalRequest,
    endpoint: RemoteTerminalEndpoint,
) {
    let RemoteTerminalEndpoint {
        mut commands,
        output,
        state,
    } = endpoint;
    let initialized = async {
        channel
            .request_pty(
                true,
                "xterm-256color",
                u32::from(request.cols),
                u32::from(request.rows),
                0,
                0,
                &[],
            )
            .await?;
        match &request.execution {
            RemoteTerminalExecution::Shell { command } => {
                channel.request_shell(true).await?;
                channel
                    .data_bytes(remote_shell_startup(
                        &request.cwd,
                        command,
                        &request.environment,
                    ))
                    .await?;
            }
            RemoteTerminalExecution::Command { program, args } => {
                channel
                    .exec(
                        true,
                        remote_exec_command_with_environment(
                            &request.cwd,
                            program,
                            args,
                            &request.environment,
                        ),
                    )
                    .await?;
            }
        }
        Ok::<(), russh::Error>(())
    }
    .await;
    if let Err(error) = initialized {
        let _ = output.send(terminal_error_output(&format!(
            "failed to initialize remote terminal: {error}"
        )));
        finish_remote_terminal(&state, None);
        return;
    }

    let mut exit_code = None;
    loop {
        tokio::select! {
            message = channel.wait() => match message {
                Some(ChannelMsg::Data { data }) | Some(ChannelMsg::ExtendedData { data, .. }) => {
                    if output.send(data.to_vec()).is_err() {
                        let _ = channel.close().await;
                        break;
                    }
                }
                Some(ChannelMsg::ExitStatus { exit_status }) => {
                    exit_code = i32::try_from(exit_status).ok();
                }
                Some(ChannelMsg::Eof) => {}
                Some(ChannelMsg::Close) | None => break,
                Some(_) => {}
            },
            command = commands.recv() => match command {
                Some(RemoteTerminalCommand::Write(bytes)) => {
                    if let Err(error) = channel.data_bytes(bytes).await {
                        let _ = output.send(terminal_error_output(&format!(
                            "failed to write remote terminal input: {error}"
                        )));
                        break;
                    }
                }
                Some(RemoteTerminalCommand::Resize { cols, rows }) => {
                    if let Err(error) = channel
                        .window_change(u32::from(cols), u32::from(rows), 0, 0)
                        .await
                    {
                        let _ = output.send(terminal_error_output(&format!(
                            "failed to resize remote terminal: {error}"
                        )));
                        break;
                    }
                }
                Some(RemoteTerminalCommand::Shutdown) | None => {
                    let _ = channel.eof().await;
                    let _ = channel.close().await;
                    break;
                }
            }
        }
    }
    finish_remote_terminal(&state, exit_code);
}

struct HostKeyHandler {
    connection_id: ConnectionId,
    epoch: ConnectionEpoch,
    endpoint: SshEndpoint,
    events: EventSender<TransportEvent>,
    verified_host_key: Arc<Mutex<Option<String>>>,
    host_keys: Arc<Mutex<HostKeyStore>>,
    reverse_forward_targets: ReverseForwardTargets,
}

impl HostKeyHandler {
    fn record_verified_host_key(&self, key: &PublicKey) -> Result<(), TransportError> {
        let fingerprint = host_key_fingerprint(key);
        *self
            .verified_host_key
            .lock()
            .map_err(|_| TransportError::MissingVerifiedHostKey)? = Some(fingerprint);
        Ok(())
    }
}

impl client::Handler for HostKeyHandler {
    type Error = TransportError;

    async fn check_server_key(
        &mut self,
        server_public_key: &PublicKey,
    ) -> Result<bool, Self::Error> {
        let algorithm = server_public_key.algorithm().as_str().to_string();
        let fingerprint = host_key_fingerprint(server_public_key);
        let verification = self
            .host_keys
            .lock()
            .map_err(|_| {
                TransportError::HostKeyStore("host key store lock is poisoned".to_string())
            })?
            .verify(
                &self.endpoint.host,
                self.endpoint.port,
                &algorithm,
                &fingerprint,
            );
        let previous_fingerprint = match verification {
            HostKeyVerification::Trusted => {
                self.record_verified_host_key(server_public_key)?;
                return Ok(true);
            }
            HostKeyVerification::Unknown => None,
            HostKeyVerification::Changed {
                previous_fingerprint,
            } => Some(previous_fingerprint),
        };

        let (response, decision) = oneshot::channel();
        let challenge = HostKeyChallenge {
            connection_id: self.connection_id.clone(),
            epoch: self.epoch,
            host: self.endpoint.host.clone(),
            port: self.endpoint.port,
            algorithm: algorithm.clone(),
            fingerprint: fingerprint.clone(),
            previous_fingerprint,
            response: Some(response),
        };
        self.events
            .send(TransportEvent::HostKeyChallenge(challenge))
            .await
            .map_err(|_| TransportError::RuntimeStopped)?;
        let decision = decision
            .await
            .map_err(|_| TransportError::HostKeyRejected)?;
        if !decision.accept {
            return Err(TransportError::HostKeyRejected);
        }
        if decision.remember {
            self.host_keys
                .lock()
                .map_err(|_| {
                    TransportError::HostKeyStore("host key store lock is poisoned".to_string())
                })?
                .remember(
                    &self.endpoint.host,
                    self.endpoint.port,
                    &algorithm,
                    &fingerprint,
                )
                .map_err(|error| TransportError::HostKeyStore(error.to_string()))?;
        }
        self.record_verified_host_key(server_public_key)?;
        Ok(true)
    }

    async fn server_channel_open_forwarded_tcpip(
        &mut self,
        channel: russh::Channel<client::Msg>,
        _connected_address: &str,
        connected_port: u32,
        _originator_address: &str,
        _originator_port: u32,
        reply: client::ChannelOpenHandle,
        _session: &mut client::Session,
    ) -> Result<(), Self::Error> {
        let target = self
            .reverse_forward_targets
            .lock()
            .ok()
            .and_then(|targets| targets.get(&connected_port).copied());
        let Some(target) = target else {
            return Ok(());
        };
        let Ok(mut target_stream) = tokio::net::TcpStream::connect(target).await else {
            return Ok(());
        };
        let _ = target_stream.set_nodelay(true);
        reply.accept().await;
        tokio::spawn(async move {
            let mut channel_stream = channel.into_stream();
            let _ = tokio::io::copy_bidirectional(&mut channel_stream, &mut target_stream).await;
        });
        Ok(())
    }
}

fn host_key_fingerprint(key: &PublicKey) -> String {
    key.fingerprint(HashAlg::Sha256).to_string()
}

fn normalize_connect_error(error: TransportError) -> TransportError {
    match error {
        TransportError::Russh(source) => TransportError::Connection(source.to_string()),
        error => error,
    }
}

fn send_state(
    events: &EventSender<TransportEvent>,
    connection_id: &ConnectionId,
    epoch: ConnectionEpoch,
    state: ConnectionState,
    error: Option<String>,
) {
    let _ = events.try_send(TransportEvent::StateChanged(ConnectionStatus {
        connection_id: connection_id.clone(),
        epoch,
        state,
        error,
    }));
}

#[derive(Debug, Error)]
pub enum TransportError {
    #[error("failed to start SSH runtime: {0}")]
    RuntimeStart(String),
    #[error("SSH runtime stopped")]
    RuntimeStopped,
    #[error("SSH connection is not connected")]
    NotConnected,
    #[error("SSH request timed out")]
    RequestTimedOut,
    #[error("SSH request was superseded by a newer connection attempt")]
    Superseded,
    #[error("SSH connection failed: {0}")]
    Connection(String),
    #[error("SSH authentication was rejected")]
    AuthenticationRejected,
    #[error("failed to use SSH agent: {0}")]
    Agent(String),
    #[error("stored credential {0} is missing")]
    CredentialMissing(CredentialId),
    #[error("failed to access stored credential: {0}")]
    Credential(String),
    #[error("stored credential {0} does not match the verified SSH endpoint")]
    CredentialBindingMismatch(CredentialId),
    #[error("SSH transport did not retain the verified host key")]
    MissingVerifiedHostKey,
    #[error("failed to load private key {path}: {message}")]
    PrivateKey { path: PathBuf, message: String },
    #[error("failed to access yttt SSH host key store: {0}")]
    HostKeyStore(String),
    #[error("host key was rejected")]
    HostKeyRejected,
    #[error("failed to initialize SFTP: {0}")]
    Sftp(String),
    #[error(transparent)]
    Russh(#[from] russh::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authentication_debug_never_contains_password() {
        let authentication = Authentication::Password {
            secret: Zeroizing::new("top-secret".to_string()),
            save_as: None,
        };
        let output = format!("{authentication:?}");
        assert!(!output.contains("top-secret"));
        assert!(output.contains("REDACTED"));
    }

    #[test]
    fn auto_authentication_debug_never_contains_key_passphrase() {
        let authentication = Authentication::Auto {
            identity_file: Some(PathBuf::from("/tmp/id_ed25519")),
            passphrase: Some(Zeroizing::new("top-secret".to_string())),
            credential: None,
        };
        let output = format!("{authentication:?}");
        assert!(!output.contains("top-secret"));
    }

    #[test]
    fn disconnect_attempt_does_not_cancel_a_newer_epoch() {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let connection_id = ConnectionId::new("epoch-cancel");
        let request = || ConnectRequest {
            connection_id: connection_id.clone(),
            endpoint: SshEndpoint {
                host: "127.0.0.1".to_string(),
                port,
                user: "nobody".to_string(),
            },
            authentication: Authentication::Password {
                secret: Zeroizing::new("unused".to_string()),
                save_as: None,
            },
            reconnect: false,
        };
        let service = TransportService::start(
            std::env::temp_dir().join("yttt-ssh-epoch-cancel-host-keys.toml"),
        )
        .unwrap();
        let events = service.events();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        let first = runtime.block_on(service.start_connect(request())).unwrap();
        let second = runtime.block_on(service.start_connect(request())).unwrap();
        assert!(second.epoch().get() > first.epoch().get());

        runtime
            .block_on(service.disconnect_attempt(connection_id.clone(), first.epoch()))
            .unwrap();
        let before_current_cancel = std::iter::from_fn(|| events.try_recv().ok())
            .filter_map(|event| match event {
                TransportEvent::StateChanged(status) => Some(status.state),
                TransportEvent::HostKeyChallenge(_) | TransportEvent::CredentialSaved { .. } => {
                    None
                }
            })
            .collect::<Vec<_>>();
        assert!(!before_current_cancel.contains(&ConnectionState::Disconnected));

        runtime
            .block_on(service.disconnect_attempt(connection_id, second.epoch()))
            .unwrap();
        let after_current_cancel = std::iter::from_fn(|| events.try_recv().ok())
            .filter_map(|event| match event {
                TransportEvent::StateChanged(status) => Some(status.state),
                TransportEvent::HostKeyChallenge(_) | TransportEvent::CredentialSaved { .. } => {
                    None
                }
            })
            .collect::<Vec<_>>();
        assert_eq!(
            after_current_cancel
                .iter()
                .filter(|state| **state == ConnectionState::Disconnected)
                .count(),
            1
        );
    }

    #[test]
    fn agent_hook_forward_only_accepts_loopback_http_targets() {
        assert_eq!(
            loopback_http_target("http://127.0.0.1:4242"),
            Some("127.0.0.1:4242".parse().unwrap())
        );
        assert_eq!(
            loopback_http_target("http://[::1]:4242"),
            Some("[::1]:4242".parse().unwrap())
        );
        assert!(loopback_http_target("https://127.0.0.1:4242").is_none());
        assert!(loopback_http_target("http://192.0.2.1:4242").is_none());
    }
    #[test]
    fn failed_connection_reports_fenced_state_transitions() {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);

        let service = TransportService::start(
            std::env::temp_dir().join("yttt-ssh-failed-connection-host-keys.toml"),
        )
        .unwrap();
        let events = service.events();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let result = runtime.block_on(service.connect(ConnectRequest {
            connection_id: ConnectionId::new("test-connection"),
            endpoint: SshEndpoint {
                host: "127.0.0.1".to_string(),
                port,
                user: "nobody".to_string(),
            },
            authentication: Authentication::Password {
                secret: Zeroizing::new("unused".to_string()),
                save_as: None,
            },
            reconnect: false,
        }));
        assert!(result.is_err());

        let states = std::iter::from_fn(|| events.try_recv().ok())
            .filter_map(|event| match event {
                TransportEvent::StateChanged(status) => Some(status.state),
                TransportEvent::HostKeyChallenge(_) => None,
                TransportEvent::CredentialSaved { .. } => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            states,
            vec![
                ConnectionState::Connecting,
                ConnectionState::VerifyingHostKey,
                ConnectionState::Failed,
            ]
        );
    }
}
