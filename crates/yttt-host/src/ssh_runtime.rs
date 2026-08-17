use std::{collections::HashMap, path::PathBuf, sync::Arc};

use crate::project::{HostProjectError, HostProjectRuntime, RegisteredSshProject};
use parking_lot::Mutex;
use tokio::sync::broadcast;
use yttt_core::model::{
    ids::{ClientInstanceId, ConnectionId, CredentialId},
    project::{RemotePathBuf, RemoteRelativePathBuf},
};
use yttt_protocol::{
    FailureCode, ProtocolFailure, Response, ServerEvent,
    ssh::{
        CredentialAnswer, CredentialChallenge, CredentialChallengeKind, HostKeyDecision,
        RemoteCommandRequest, RemoteCommandResponse, RemoteDirectory, RemoteEntryMutation,
        RemoteFileContent, RemoteFileEntry, RemoteFileFingerprint, RemoteFileKind,
        RemoteFileRequest, RemoteFileResponse, RemoteFileState, RemoteHostCommand,
        RemoteSaveResult, SshAuthentication, SshConnectSpec, SshConnectionState,
        SshConnectionStatus, StoredSshCredential,
    },
};
use yttt_ssh::{
    Authentication, ConnectRequest, CredentialStore, HostKeyChallenge, SftpProject,
    StoredCredential, TransportEvent, TransportService,
    sftp::{
        RemoteEntryKind, RemoteFileState as SftpFileState, RemoteFingerprint, RemoteSaveOutcome,
        SftpError,
    },
    transport::{ConnectionState, HostKeyDecision as TransportHostKeyDecision},
};
use zeroize::Zeroizing;

const SERVER_EVENT_CAPACITY: usize = 256;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SshRuntimeEvent {
    Broadcast(ServerEvent),
    Unicast {
        client_id: ClientInstanceId,
        event: ServerEvent,
    },
}

impl SshRuntimeEvent {
    pub fn for_client(&self, client_id: &ClientInstanceId) -> Option<&ServerEvent> {
        match self {
            Self::Broadcast(event) => Some(event),
            Self::Unicast {
                client_id: target,
                event,
            } if target == client_id => Some(event),
            Self::Unicast { .. } => None,
        }
    }
}

struct PendingCredentialChallenge {
    initiator: ClientInstanceId,
    #[allow(dead_code)]
    connection_id: String,
    challenge: Option<HostKeyChallenge>,
}

struct SshEventRouter {
    events: broadcast::Sender<SshRuntimeEvent>,
    challenges: Mutex<HashMap<u64, PendingCredentialChallenge>>,
    initiators: Mutex<HashMap<String, ClientInstanceId>>,
    next_challenge_id: std::sync::atomic::AtomicU64,
}

impl SshEventRouter {
    fn new() -> Self {
        let (events, _) = broadcast::channel(SERVER_EVENT_CAPACITY);
        Self {
            events,
            challenges: Mutex::new(HashMap::new()),
            initiators: Mutex::new(HashMap::new()),
            next_challenge_id: std::sync::atomic::AtomicU64::new(1),
        }
    }

    fn remember_initiator(&self, connection_id: String, initiator: ClientInstanceId) {
        self.initiators.lock().insert(connection_id, initiator);
    }

    fn forget_connection(&self, connection_id: &str) {
        self.initiators.lock().remove(connection_id);
    }

    fn publish(&self, event: SshRuntimeEvent) {
        let _ = self.events.send(event);
    }

    fn publish_host_key_challenge(&self, challenge: HostKeyChallenge) {
        let connection_id = challenge.connection_id.as_str().to_string();
        let Some(initiator) = self.initiators.lock().get(&connection_id).cloned() else {
            let _ = challenge.respond(TransportHostKeyDecision {
                accept: false,
                remember: false,
            });
            return;
        };
        let challenge_id = self
            .next_challenge_id
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let wire = CredentialChallenge {
            challenge_id,
            connection_id: connection_id.clone(),
            kind: CredentialChallengeKind::HostKey {
                host: challenge.host.clone(),
                port: challenge.port,
                algorithm: challenge.algorithm.clone(),
                fingerprint: challenge.fingerprint.clone(),
                previous_fingerprint: challenge.previous_fingerprint.clone(),
            },
            attempt: 1,
        };
        self.challenges.lock().insert(
            challenge_id,
            PendingCredentialChallenge {
                initiator: initiator.clone(),
                connection_id,
                challenge: Some(challenge),
            },
        );
        self.publish(SshRuntimeEvent::Unicast {
            client_id: initiator,
            event: ServerEvent::CredentialChallenge(wire),
        });
    }

    fn answer_credential(
        &self,
        challenge_id: u64,
        answer: CredentialAnswer,
        client_id: &ClientInstanceId,
    ) -> Result<Response, ProtocolFailure> {
        let mut challenges = self.challenges.lock();
        let Some(pending) = challenges.get(&challenge_id) else {
            return Err(ProtocolFailure::new(
                FailureCode::NotFound,
                format!("SSH credential challenge {challenge_id} was not found"),
                false,
            ));
        };
        if pending.initiator != *client_id {
            return Err(ProtocolFailure::new(
                FailureCode::PermissionDenied,
                "only the SSH connect initiator may answer this credential challenge",
                false,
            ));
        }
        let pending = challenges
            .remove(&challenge_id)
            .expect("challenge was present");
        drop(challenges);
        let CredentialAnswer::HostKey(decision) = answer else {
            return Err(ProtocolFailure::new(
                FailureCode::InvalidRequest,
                "SSH Host currently accepts only host-key challenge answers",
                false,
            ));
        };
        let decision = match decision {
            HostKeyDecision::AcceptOnce => TransportHostKeyDecision {
                accept: true,
                remember: false,
            },
            HostKeyDecision::AcceptAndStore => TransportHostKeyDecision {
                accept: true,
                remember: true,
            },
            HostKeyDecision::Reject => TransportHostKeyDecision {
                accept: false,
                remember: false,
            },
        };
        if let Some(challenge) = pending.challenge {
            challenge.respond(decision).map_err(|_| {
                ProtocolFailure::new(
                    FailureCode::NotFound,
                    "SSH host-key challenge receiver was closed",
                    false,
                )
            })?;
        }
        Ok(Response::CredentialAccepted)
    }

    fn abandon_challenges(&self, client_id: &ClientInstanceId) {
        let mut challenges = self.challenges.lock();
        let ids = challenges
            .iter()
            .filter(|(_, pending)| pending.initiator == *client_id)
            .map(|(id, _)| *id)
            .collect::<Vec<_>>();
        for id in ids {
            if let Some(pending) = challenges.remove(&id)
                && let Some(challenge) = pending.challenge
            {
                let _ = challenge.respond(TransportHostKeyDecision {
                    accept: false,
                    remember: false,
                });
            }
        }
        self.initiators
            .lock()
            .retain(|_, initiator| initiator != client_id);
    }

    #[cfg(test)]
    fn inject_host_key_challenge_for_tests(&self, connection_id: &str) -> u64 {
        let initiator = self
            .initiators
            .lock()
            .get(connection_id)
            .cloned()
            .expect("initiator must be registered before injecting a challenge");
        let challenge_id = self
            .next_challenge_id
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.challenges.lock().insert(
            challenge_id,
            PendingCredentialChallenge {
                initiator: initiator.clone(),
                connection_id: connection_id.to_string(),
                challenge: None,
            },
        );
        self.publish(SshRuntimeEvent::Unicast {
            client_id: initiator,
            event: ServerEvent::CredentialChallenge(CredentialChallenge {
                challenge_id,
                connection_id: connection_id.to_string(),
                kind: CredentialChallengeKind::HostKey {
                    host: "example.test".to_string(),
                    port: 22,
                    algorithm: "ssh-ed25519".to_string(),
                    fingerprint: "SHA256:test".to_string(),
                    previous_fingerprint: None,
                },
                attempt: 1,
            }),
        });
        challenge_id
    }
}

pub struct HostSshRuntime {
    transport: TransportService,
    credential_store: CredentialStore,
    router: SshEventRouter,
    statuses: Mutex<HashMap<String, SshConnectionStatus>>,
}

impl HostSshRuntime {
    pub fn start(
        host_keys_path: PathBuf,
        credential_namespace: String,
    ) -> Result<Arc<Self>, String> {
        let credential_store = CredentialStore::new(credential_namespace);
        let transport =
            TransportService::start_with_credential_store(host_keys_path, credential_store.clone())
                .map_err(|error| error.to_string())?;
        let transport_events = transport.events();
        let runtime = Arc::new(Self {
            transport,
            credential_store,
            router: SshEventRouter::new(),
            statuses: Mutex::new(HashMap::new()),
        });
        let bridge = runtime.clone();
        tokio::spawn(async move {
            while let Ok(event) = transport_events.recv().await {
                bridge.publish_transport_event(event);
            }
        });
        Ok(runtime)
    }

    pub fn subscribe(&self) -> broadcast::Receiver<SshRuntimeEvent> {
        self.router.events.subscribe()
    }

    pub fn abandon_challenges(&self, client_id: &ClientInstanceId) {
        self.router.abandon_challenges(client_id);
    }

    pub fn connections(&self) -> Vec<String> {
        let mut connections = self.statuses.lock().keys().cloned().collect::<Vec<_>>();
        connections.sort();
        connections
    }

    pub async fn connect(
        &self,
        spec: SshConnectSpec,
        initiator: ClientInstanceId,
    ) -> Result<Response, String> {
        let connection_id = spec.connection_id.clone();
        self.router
            .remember_initiator(connection_id.clone(), initiator);
        let attempt = self
            .transport
            .start_connect(ConnectRequest {
                connection_id: ConnectionId::new(spec.connection_id),
                endpoint: yttt_ssh::SshEndpoint {
                    host: spec.endpoint.host,
                    port: spec.endpoint.port,
                    user: spec.endpoint.username,
                },
                authentication: authentication(spec.authentication)?,
                reconnect: spec.reconnect,
            })
            .await
            .map_err(|error| error.to_string())?;
        Ok(Response::SshConnected {
            connection_id,
            epoch: attempt.epoch().get(),
        })
    }

    pub async fn disconnect(&self, connection_id: String) -> Result<Response, String> {
        self.transport
            .disconnect(ConnectionId::new(connection_id))
            .await
            .map_err(|error| error.to_string())?;
        Ok(Response::SshDisconnected)
    }

    pub fn answer_credential(
        &self,
        challenge_id: u64,
        answer: CredentialAnswer,
        client_id: &ClientInstanceId,
    ) -> Result<Response, ProtocolFailure> {
        self.router
            .answer_credential(challenge_id, answer, client_id)
    }

    pub fn delete_credential(&self, credential_id: String) -> Result<Response, String> {
        self.credential_store
            .delete(&CredentialId::new(credential_id))
            .map_err(|error| error.to_string())?;
        Ok(Response::CredentialDeleted)
    }

    pub fn remote_file(
        &self,
        projects: &HostProjectRuntime,
        request: RemoteFileRequest,
    ) -> Result<Response, HostProjectError> {
        let response = match request {
            RemoteFileRequest::ResolveHome { connection_id } => {
                let root = RemotePathBuf::new("/").map_err(|error| error.to_string())?;
                let project = self
                    .transport
                    .sftp_project(ConnectionId::new(connection_id), root);
                RemoteFileResponse::Home(
                    project
                        .resolve_home()
                        .map_err(|error| error.to_string())?
                        .to_string(),
                )
            }
            RemoteFileRequest::BrowseDirectory {
                connection_id,
                root,
                relative_directory,
                show_hidden,
            } => {
                let root = RemotePathBuf::new(root).map_err(|error| error.to_string())?;
                let project = self
                    .transport
                    .sftp_project(ConnectionId::new(connection_id), root);
                let snapshot = project
                    .scan_directory(remote_relative(relative_directory)?, show_hidden)
                    .map_err(|error| error.to_string())?;
                RemoteFileResponse::Directory(RemoteDirectory {
                    relative_directory: snapshot.relative_directory.to_string(),
                    entries: snapshot
                        .entries
                        .into_iter()
                        .map(|entry| RemoteFileEntry {
                            name: entry.name,
                            relative_path: entry.relative_path.to_string(),
                            kind: file_kind(entry.kind),
                        })
                        .collect(),
                })
            }
            RemoteFileRequest::ScanDirectory {
                project_id,
                relative_directory,
                show_hidden,
            } => {
                let project = self.project(projects.ssh_project(&project_id)?);
                let relative = remote_relative(relative_directory)?;
                let snapshot = project
                    .scan_directory(relative, show_hidden)
                    .map_err(|error| error.to_string())?;
                RemoteFileResponse::Directory(RemoteDirectory {
                    relative_directory: snapshot.relative_directory.to_string(),
                    entries: snapshot
                        .entries
                        .into_iter()
                        .map(|entry| RemoteFileEntry {
                            name: entry.name,
                            relative_path: entry.relative_path.to_string(),
                            kind: file_kind(entry.kind),
                        })
                        .collect(),
                })
            }
            RemoteFileRequest::Read {
                project_id,
                relative_path,
                maximum_bytes,
            } => {
                let path_key = relative_path.clone();
                let project = self.project(projects.ssh_project(&project_id)?);
                let file = project
                    .read_file(remote_relative(relative_path)?, maximum_bytes)
                    .map_err(sftp_project_error)?;
                RemoteFileResponse::File(RemoteFileContent {
                    relative_path: file.relative_path.to_string(),
                    bytes: file.bytes,
                    fingerprint: bind_remote_fingerprint(
                        projects,
                        &project_id,
                        &path_key,
                        file.fingerprint,
                    ),
                })
            }
            RemoteFileRequest::Save {
                project_id,
                relative_path,
                expected,
                force,
                maximum_bytes,
                bytes,
            } => {
                let path_key = relative_path.clone();
                let project = self.project(projects.ssh_project(&project_id)?);
                if !force
                    && let Some(base) = &expected
                    && !projects.revision_matches(&project_id, &path_key, &base.revision)
                {
                    RemoteFileResponse::Save(
                        match project.read_file(remote_relative(path_key.clone())?, maximum_bytes) {
                            Ok(file) => RemoteSaveResult::Conflict(RemoteFileState::Present(
                                bind_remote_fingerprint(
                                    projects,
                                    &project_id,
                                    &path_key,
                                    file.fingerprint,
                                ),
                            )),
                            Err(_) => RemoteSaveResult::Conflict(RemoteFileState::Missing),
                        },
                    )
                } else {
                    let outcome = project
                        .save_file(
                            remote_relative(relative_path)?,
                            bytes,
                            expected.map(sftp_fingerprint),
                            force,
                            maximum_bytes,
                        )
                        .map_err(sftp_project_error)?;
                    RemoteFileResponse::Save(match outcome {
                        RemoteSaveOutcome::Saved(value) => {
                            let mut fingerprint = fingerprint(value);
                            fingerprint.revision = projects.bump_revision(
                                &project_id,
                                &path_key,
                                fingerprint.revision.content_sha256,
                            );
                            RemoteSaveResult::Saved(fingerprint)
                        }
                        RemoteSaveOutcome::Conflict(SftpFileState::Missing) => {
                            RemoteSaveResult::Conflict(RemoteFileState::Missing)
                        }
                        RemoteSaveOutcome::Conflict(SftpFileState::Present(value)) => {
                            RemoteSaveResult::Conflict(RemoteFileState::Present(
                                bind_remote_fingerprint(projects, &project_id, &path_key, value),
                            ))
                        }
                    })
                }
            }
            RemoteFileRequest::Create {
                project_id,
                relative_path,
                directory,
            } => {
                let project = self.project(projects.ssh_project(&project_id)?);
                let mutation = project
                    .create_entry(remote_relative(relative_path)?, directory)
                    .map_err(|error| error.to_string())?;
                RemoteFileResponse::Mutation(entry_mutation(mutation))
            }
            RemoteFileRequest::Rename {
                project_id,
                relative_path,
                new_name,
            } => {
                let project = self.project(projects.ssh_project(&project_id)?);
                let mutation = project
                    .rename_entry(remote_relative(relative_path)?, new_name)
                    .map_err(|error| error.to_string())?;
                RemoteFileResponse::Mutation(entry_mutation(mutation))
            }
            RemoteFileRequest::Delete {
                project_id,
                relative_path,
            } => {
                let project = self.project(projects.ssh_project(&project_id)?);
                project
                    .delete_entry(remote_relative(relative_path)?)
                    .map_err(|error| error.to_string())?;
                RemoteFileResponse::Deleted
            }
        };
        Ok(Response::RemoteFile(response))
    }

    pub fn remote_command(
        &self,
        projects: &HostProjectRuntime,
        request: RemoteCommandRequest,
    ) -> Result<Response, HostProjectError> {
        let operation = authorize_remote_host_command(request.command)?;
        let registered = projects.ssh_project(&request.project_id)?;
        let root = match operation.work_tree() {
            Some(work_tree) => {
                let relative = work_tree.to_utf8().map_err(|error| error.to_string())?;
                registered.root.join_relative(&remote_relative(relative)?)
            }
            None => registered.root.clone(),
        };
        let args = operation.argv("/dev/null").map_err(|error| {
            HostProjectError::with_code(FailureCode::InvalidRequest, error.to_string())
        })?;
        if !yttt_protocol::git_argv_is_safe(&args) {
            return Err(HostProjectError::with_code(
                FailureCode::PermissionDenied,
                "git arguments are not allowed",
            ));
        }
        let project = self
            .transport
            .sftp_project(ConnectionId::new(registered.connection_id), root);
        let output = project
            .run_command("git", args)
            .map_err(|error| error.to_string())?;
        Ok(Response::RemoteCommand(RemoteCommandResponse {
            exit_status: output.exit_status.unwrap_or(u32::MAX as i32) as u32,
            stdout: output.stdout,
            stderr: output.stderr,
        }))
    }

    pub fn transport(&self) -> TransportService {
        self.transport.clone()
    }

    fn project(&self, project: RegisteredSshProject) -> SftpProject {
        self.transport
            .sftp_project(ConnectionId::new(project.connection_id), project.root)
    }

    fn publish_transport_event(&self, event: TransportEvent) {
        let event = match event {
            TransportEvent::StateChanged(status) => {
                let status = SshConnectionStatus {
                    connection_id: status.connection_id.to_string(),
                    epoch: status.epoch.get(),
                    state: connection_state(status.state),
                    error: status.error,
                };
                if status.state == SshConnectionState::Disconnected {
                    self.router.forget_connection(&status.connection_id);
                    self.statuses.lock().remove(&status.connection_id);
                } else {
                    self.statuses
                        .lock()
                        .insert(status.connection_id.clone(), status.clone());
                }
                self.router
                    .publish(SshRuntimeEvent::Broadcast(ServerEvent::SshStateChanged(
                        status,
                    )));
                return;
            }
            TransportEvent::HostKeyChallenge(challenge) => {
                self.router.publish_host_key_challenge(challenge);
                return;
            }
            TransportEvent::CredentialSaved {
                connection_id,
                epoch,
                credential,
            } => SshRuntimeEvent::Broadcast(ServerEvent::SshCredentialSaved {
                connection_id: connection_id.to_string(),
                epoch: epoch.get(),
                credential: stored_credential(credential),
            }),
        };
        self.router.publish(event);
    }
}

fn authentication(authentication: SshAuthentication) -> Result<Authentication, String> {
    Ok(match authentication {
        SshAuthentication::Auto {
            identity_file,
            passphrase,
            credential,
        } => Authentication::Auto {
            identity_file: identity_file.map(PathBuf::from),
            passphrase: passphrase.map(secret_string).transpose()?,
            credential: credential.map(transport_credential),
        },
        SshAuthentication::Agent => Authentication::Agent,
        SshAuthentication::Password { secret, save_as } => Authentication::Password {
            secret: secret_string(secret)?,
            save_as: save_as.map(CredentialId::new),
        },
        SshAuthentication::StoredPassword(credential) => {
            Authentication::StoredPassword(transport_credential(credential))
        }
        SshAuthentication::PrivateKey { path, passphrase } => Authentication::PrivateKey {
            path: PathBuf::from(path),
            passphrase: passphrase.map(secret_string).transpose()?,
        },
    })
}

fn secret_string(secret: yttt_protocol::ssh::SensitiveBytes) -> Result<Zeroizing<String>, String> {
    String::from_utf8(secret.into_inner())
        .map(Zeroizing::new)
        .map_err(|_| "SSH secret must be valid UTF-8".to_string())
}

fn transport_credential(credential: StoredSshCredential) -> StoredCredential {
    StoredCredential {
        id: CredentialId::new(credential.id),
        effective_user: credential.effective_user,
        resolved_host: String::new(),
        port: 0,
        host_key_sha256: String::new(),
        private_key_identity: credential.private_key_identity,
    }
}

fn stored_credential(credential: StoredCredential) -> StoredSshCredential {
    StoredSshCredential {
        id: credential.id.to_string(),
        effective_user: credential.effective_user,
        private_key_identity: credential.private_key_identity,
    }
}

fn remote_relative(path: String) -> Result<RemoteRelativePathBuf, String> {
    RemoteRelativePathBuf::new(path).map_err(|error| error.to_string())
}

fn sftp_project_error(error: SftpError) -> HostProjectError {
    let code = match error {
        SftpError::FileTooLarge { .. } => yttt_protocol::FailureCode::ResourceLimit,
        SftpError::PathOutsideRoot(_) => yttt_protocol::FailureCode::PermissionDenied,
        SftpError::AlreadyExists(_) => yttt_protocol::FailureCode::AlreadyExists,
        _ => yttt_protocol::FailureCode::Internal,
    };
    HostProjectError::with_code(code, error.to_string())
}

fn file_kind(kind: RemoteEntryKind) -> RemoteFileKind {
    match kind {
        RemoteEntryKind::File => RemoteFileKind::File,
        RemoteEntryKind::Directory => RemoteFileKind::Directory,
        RemoteEntryKind::SymlinkFile => RemoteFileKind::SymlinkFile,
        RemoteEntryKind::SymlinkDirectory => RemoteFileKind::SymlinkDirectory,
    }
}

fn entry_mutation(mutation: yttt_ssh::sftp::RemoteEntryMutation) -> RemoteEntryMutation {
    RemoteEntryMutation {
        relative_path: mutation.relative_path.to_string(),
        kind: file_kind(mutation.kind),
    }
}

fn bind_remote_fingerprint(
    projects: &HostProjectRuntime,
    project_id: &yttt_core::model::ids::ProjectId,
    relative_path: &str,
    value: RemoteFingerprint,
) -> RemoteFileFingerprint {
    let mut fingerprint = fingerprint(value);
    fingerprint.revision = projects.bind_revision(
        project_id,
        relative_path,
        fingerprint.revision.content_sha256,
    );
    fingerprint
}

fn fingerprint(value: RemoteFingerprint) -> RemoteFileFingerprint {
    RemoteFileFingerprint {
        byte_len: value.byte_len,
        modified_seconds: value.modified_seconds,
        content_hash: value.content_hash,
        revision: yttt_protocol::ContentRevision {
            content_sha256: value.content_sha256,
            ..Default::default()
        },
    }
}

fn sftp_fingerprint(value: RemoteFileFingerprint) -> RemoteFingerprint {
    RemoteFingerprint {
        byte_len: value.byte_len,
        modified_seconds: value.modified_seconds,
        content_hash: value.content_hash,
        content_sha256: value.revision.content_sha256,
    }
}

fn authorize_remote_host_command(
    command: RemoteHostCommand,
) -> Result<yttt_protocol::ProjectGitOperation, HostProjectError> {
    match command {
        RemoteHostCommand::Git { operation } => Ok(operation),
        RemoteHostCommand::Privileged { .. } => Err(HostProjectError::with_code(
            FailureCode::PermissionDenied,
            "privileged remote commands are disabled",
        )),
    }
}

fn connection_state(state: ConnectionState) -> SshConnectionState {
    match state {
        ConnectionState::Disconnected => SshConnectionState::Disconnected,
        ConnectionState::Connecting => SshConnectionState::Connecting,
        ConnectionState::VerifyingHostKey => SshConnectionState::VerifyingHostKey,
        ConnectionState::Authenticating => SshConnectionState::Authenticating,
        ConnectionState::Connected => SshConnectionState::Connected,
        ConnectionState::Reconnecting => SshConnectionState::Reconnecting,
        ConnectionState::Failed => SshConnectionState::Failed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yttt_protocol::ssh::HostKeyDecision;

    #[test]
    fn credential_challenge_is_visible_only_to_the_initiator() {
        let router = SshEventRouter::new();
        let initiator = ClientInstanceId::new("client-a");
        let observer = ClientInstanceId::new("client-b");
        router.remember_initiator("ssh-1".to_string(), initiator.clone());
        let mut events = router.events.subscribe();
        let challenge_id = router.inject_host_key_challenge_for_tests("ssh-1");
        let event = events.try_recv().unwrap();
        assert!(matches!(
            event.for_client(&initiator),
            Some(ServerEvent::CredentialChallenge(challenge))
                if challenge.challenge_id == challenge_id
        ));
        assert_eq!(event.for_client(&observer), None);

        router.publish(SshRuntimeEvent::Broadcast(ServerEvent::SshStateChanged(
            SshConnectionStatus {
                connection_id: "ssh-1".to_string(),
                epoch: 1,
                state: SshConnectionState::VerifyingHostKey,
                error: None,
            },
        )));
        let state = events.try_recv().unwrap();
        assert!(matches!(
            state.for_client(&initiator),
            Some(ServerEvent::SshStateChanged(_))
        ));
        assert!(matches!(
            state.for_client(&observer),
            Some(ServerEvent::SshStateChanged(_))
        ));

        let denied = router
            .answer_credential(
                challenge_id,
                CredentialAnswer::HostKey(HostKeyDecision::AcceptOnce),
                &observer,
            )
            .unwrap_err();
        assert_eq!(denied.code, FailureCode::PermissionDenied);

        router.abandon_challenges(&initiator);
        let missing = router
            .answer_credential(
                challenge_id,
                CredentialAnswer::HostKey(HostKeyDecision::AcceptOnce),
                &initiator,
            )
            .unwrap_err();
        assert_eq!(missing.code, FailureCode::NotFound);
    }

    #[test]
    fn privileged_remote_commands_are_disabled() {
        let error = authorize_remote_host_command(RemoteHostCommand::Privileged {
            program: "id".to_string(),
            args: Vec::new(),
        })
        .unwrap_err();
        assert_eq!(error.code, FailureCode::PermissionDenied);
        assert!(
            authorize_remote_host_command(RemoteHostCommand::Git {
                operation: yttt_protocol::ProjectGitOperation::Status { work_tree: None },
            })
            .is_ok()
        );
    }
}
