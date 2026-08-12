use std::{collections::HashMap, path::PathBuf, sync::Arc};

use parking_lot::Mutex;
use tokio::sync::broadcast;
use yttt_core::model::{
    ids::{ConnectionId, CredentialId},
    project::{RemotePathBuf, RemoteRelativePathBuf},
};
use yttt_protocol::{
    Response, ServerEvent,
    ssh::{
        CredentialAnswer, CredentialChallenge, CredentialChallengeKind, HostKeyDecision,
        RemoteCommandRequest, RemoteCommandResponse, RemoteDirectory, RemoteEntryMutation,
        RemoteFileContent, RemoteFileEntry, RemoteFileFingerprint, RemoteFileKind,
        RemoteFileRequest, RemoteFileResponse, RemoteFileState, RemoteSaveResult,
        SshAuthentication, SshConnectSpec, SshConnectionState, SshConnectionStatus,
        StoredSshCredential,
    },
};
use yttt_ssh::{
    Authentication, ConnectRequest, CredentialStore, HostKeyChallenge, SftpProject,
    StoredCredential, TransportEvent, TransportService,
    sftp::{
        RemoteEntryKind, RemoteFileState as SftpFileState, RemoteFingerprint, RemoteSaveOutcome,
    },
    transport::{ConnectionState, HostKeyDecision as TransportHostKeyDecision},
};
use zeroize::Zeroizing;

const SERVER_EVENT_CAPACITY: usize = 256;

pub struct HostSshRuntime {
    transport: TransportService,
    credential_store: CredentialStore,
    events: broadcast::Sender<ServerEvent>,
    challenges: Mutex<HashMap<u64, HostKeyChallenge>>,
    statuses: Mutex<HashMap<String, SshConnectionStatus>>,
    next_challenge_id: std::sync::atomic::AtomicU64,
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
        let (events, _) = broadcast::channel(SERVER_EVENT_CAPACITY);
        let runtime = Arc::new(Self {
            transport,
            credential_store,
            events,
            challenges: Mutex::new(HashMap::new()),
            statuses: Mutex::new(HashMap::new()),
            next_challenge_id: std::sync::atomic::AtomicU64::new(1),
        });
        let bridge = runtime.clone();
        tokio::spawn(async move {
            while let Ok(event) = transport_events.recv().await {
                bridge.publish_transport_event(event);
            }
        });
        Ok(runtime)
    }

    pub fn subscribe(&self) -> broadcast::Receiver<ServerEvent> {
        self.events.subscribe()
    }

    pub fn connections(&self) -> Vec<String> {
        let mut connections = self.statuses.lock().keys().cloned().collect::<Vec<_>>();
        connections.sort();
        connections
    }

    pub async fn connect(&self, spec: SshConnectSpec) -> Result<Response, String> {
        let connection_id = spec.connection_id.clone();
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
    ) -> Result<Response, String> {
        let challenge = self
            .challenges
            .lock()
            .remove(&challenge_id)
            .ok_or_else(|| format!("SSH credential challenge {challenge_id} was not found"))?;
        let CredentialAnswer::HostKey(decision) = answer else {
            return Err("SSH Host currently accepts only host-key challenge answers".to_string());
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
        challenge
            .respond(decision)
            .map_err(|_| "SSH host-key challenge receiver was closed".to_string())?;
        Ok(Response::CredentialAccepted)
    }

    pub fn delete_credential(&self, credential_id: String) -> Result<Response, String> {
        self.credential_store
            .delete(&CredentialId::new(credential_id))
            .map_err(|error| error.to_string())?;
        Ok(Response::CredentialDeleted)
    }

    pub fn remote_file(&self, request: RemoteFileRequest) -> Result<Response, String> {
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
            RemoteFileRequest::ScanDirectory {
                connection_id,
                root,
                relative_directory,
                show_hidden,
            } => {
                let project = self.project(connection_id, root)?;
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
                connection_id,
                root,
                relative_path,
                maximum_bytes,
            } => {
                let project = self.project(connection_id, root)?;
                let file = project
                    .read_file(remote_relative(relative_path)?, maximum_bytes)
                    .map_err(|error| error.to_string())?;
                RemoteFileResponse::File(RemoteFileContent {
                    canonical_path: file.canonical_path.to_string(),
                    relative_path: file.relative_path.to_string(),
                    bytes: file.bytes,
                    fingerprint: fingerprint(file.fingerprint),
                })
            }
            RemoteFileRequest::Save {
                connection_id,
                root,
                relative_path,
                expected,
                force,
                maximum_bytes,
                bytes,
            } => {
                let project = self.project(connection_id, root)?;
                let outcome = project
                    .save_file(
                        remote_relative(relative_path)?,
                        bytes,
                        expected.map(sftp_fingerprint),
                        force,
                        maximum_bytes,
                    )
                    .map_err(|error| error.to_string())?;
                RemoteFileResponse::Save(match outcome {
                    RemoteSaveOutcome::Saved(value) => RemoteSaveResult::Saved(fingerprint(value)),
                    RemoteSaveOutcome::Conflict(SftpFileState::Missing) => {
                        RemoteSaveResult::Conflict(RemoteFileState::Missing)
                    }
                    RemoteSaveOutcome::Conflict(SftpFileState::Present(value)) => {
                        RemoteSaveResult::Conflict(RemoteFileState::Present(fingerprint(value)))
                    }
                })
            }
            RemoteFileRequest::Create {
                connection_id,
                root,
                relative_path,
                directory,
            } => {
                let project = self.project(connection_id, root)?;
                let mutation = project
                    .create_entry(remote_relative(relative_path)?, directory)
                    .map_err(|error| error.to_string())?;
                RemoteFileResponse::Mutation(entry_mutation(mutation))
            }
            RemoteFileRequest::Rename {
                connection_id,
                root,
                relative_path,
                new_name,
            } => {
                let project = self.project(connection_id, root)?;
                let mutation = project
                    .rename_entry(remote_relative(relative_path)?, new_name)
                    .map_err(|error| error.to_string())?;
                RemoteFileResponse::Mutation(entry_mutation(mutation))
            }
            RemoteFileRequest::Delete {
                connection_id,
                root,
                relative_path,
            } => {
                let project = self.project(connection_id, root)?;
                project
                    .delete_entry(remote_relative(relative_path)?)
                    .map_err(|error| error.to_string())?;
                RemoteFileResponse::Deleted
            }
        };
        Ok(Response::RemoteFile(response))
    }

    pub fn remote_command(&self, request: RemoteCommandRequest) -> Result<Response, String> {
        let project = self.project(request.connection_id, request.root)?;
        let output = project
            .run_command(request.program, request.args)
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

    fn project(&self, connection_id: String, root: String) -> Result<SftpProject, String> {
        let root = RemotePathBuf::new(root).map_err(|error| error.to_string())?;
        Ok(self
            .transport
            .sftp_project(ConnectionId::new(connection_id), root))
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
                    self.statuses.lock().remove(&status.connection_id);
                } else {
                    self.statuses
                        .lock()
                        .insert(status.connection_id.clone(), status.clone());
                }
                ServerEvent::SshStateChanged(status)
            }
            TransportEvent::HostKeyChallenge(challenge) => {
                let challenge_id = self
                    .next_challenge_id
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let wire = CredentialChallenge {
                    challenge_id,
                    connection_id: challenge.connection_id.as_str().to_string(),
                    kind: CredentialChallengeKind::HostKey {
                        host: challenge.host.clone(),
                        port: challenge.port,
                        algorithm: challenge.algorithm.clone(),
                        fingerprint: challenge.fingerprint.clone(),
                        previous_fingerprint: challenge.previous_fingerprint.clone(),
                    },
                    attempt: 1,
                };
                self.challenges.lock().insert(challenge_id, challenge);
                ServerEvent::CredentialChallenge(wire)
            }
            TransportEvent::CredentialSaved {
                connection_id,
                epoch,
                credential,
            } => ServerEvent::SshCredentialSaved {
                connection_id: connection_id.to_string(),
                epoch: epoch.get(),
                credential: stored_credential(credential),
            },
        };
        let _ = self.events.send(event);
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
        resolved_host: credential.resolved_host,
        port: credential.port,
        host_key_sha256: credential.host_key_sha256,
        private_key_identity: credential.private_key_identity,
    }
}

fn stored_credential(credential: StoredCredential) -> StoredSshCredential {
    StoredSshCredential {
        id: credential.id.to_string(),
        effective_user: credential.effective_user,
        resolved_host: credential.resolved_host,
        port: credential.port,
        host_key_sha256: credential.host_key_sha256,
        private_key_identity: credential.private_key_identity,
    }
}

fn remote_relative(path: String) -> Result<RemoteRelativePathBuf, String> {
    RemoteRelativePathBuf::new(path).map_err(|error| error.to_string())
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

fn fingerprint(value: RemoteFingerprint) -> RemoteFileFingerprint {
    RemoteFileFingerprint {
        byte_len: value.byte_len,
        modified_seconds: value.modified_seconds,
        content_hash: value.content_hash,
    }
}

fn sftp_fingerprint(value: RemoteFileFingerprint) -> RemoteFingerprint {
    RemoteFingerprint {
        byte_len: value.byte_len,
        modified_seconds: value.modified_seconds,
        content_hash: value.content_hash,
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
