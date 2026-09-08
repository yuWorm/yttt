use serde::{Deserialize, Serialize};
use yttt_core::model::ids::{ClientInstanceId, HostId, ProfileId, ProjectId, TerminalSessionId};

use crate::{
    agent::{AgentSnapshotCursor, AgentSnapshotUpdate},
    project::{ProjectChange, ProjectRequest, ProjectResponse},
    ssh::{
        CredentialAnswer, CredentialChallenge, RemoteCommandRequest, RemoteCommandResponse,
        RemoteFileRequest, RemoteFileResponse, SshConnectSpec, SshConnectionStatus,
        StoredSshCredential,
    },
    terminal::{
        AttachTerminal, ReadTerminalViewport, ResizeTerminal, ScrollTerminal, SearchTerminal,
        SetTerminalQueryPalette, TerminalCheckpoint, TerminalGeometry, TerminalInput,
        TerminalLeaseMode, TerminalProcessState, TerminalSearchResults, TerminalSpawnSpec,
        TerminalStreamUpdate, TerminalViewportRead, TerminateTerminalRequest, TerminatedTerminal,
        TerminationMode,
    },
    workspace::{WorkspaceRequest, WorkspaceResponse},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Capability {
    TerminalInteractive,
    ProjectRead,
    ProjectMutate,
    GitRead,
    GitMutate,
    SshConnect,
    RemoteCommandPrivileged,
    CredentialAnswer,
    WorkspaceRead,
    WorkspaceMutate,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientRequest {
    pub request_id: u64,
    #[serde(default)]
    pub actor_device_id: Option<String>,
    #[serde(default)]
    pub lease_epoch: Option<u64>,
    pub control: Option<crate::session::ControlContext>,
    pub body: Request,
}

impl ClientRequest {
    pub fn new(request_id: u64, body: Request) -> Self {
        Self {
            request_id,
            actor_device_id: None,
            lease_epoch: None,
            control: None,
            body,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostResponse {
    pub request_id: u64,
    pub result: Result<Response, ProtocolFailure>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalTerminationResult {
    pub request_id: u64,
    pub session_id: TerminalSessionId,
    pub result: Result<TerminatedTerminal, ProtocolFailure>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum HostBlocker {
    RunningTerminal(TerminalSessionId),
    ExitedTerminalAwaitingAck {
        session_id: TerminalSessionId,
        session_epoch: u64,
        final_sequence: u64,
    },
    SshConnection(String),
    Project(ProjectId),
    ConnectedClients {
        count: u32,
    },
    PendingResourceOperations {
        count: u32,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostEvent {
    pub host_sequence: u64,
    pub body: ServerEvent,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ControlMessage {
    Request(ClientRequest),
    Response(HostResponse),
    Event(HostEvent),
    TerminalInput(TerminalInput),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TerminalInteractiveMessage {
    Input {
        input: TerminalInput,
        control: Option<crate::session::ControlContext>,
    },
    Request(ClientRequest),
    Response(HostResponse),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    Ping {
        sent_millis: u64,
    },
    ListResources,
    SpawnTerminal(TerminalSpawnSpec),
    AttachTerminal(AttachTerminal),
    DetachTerminal {
        session_id: TerminalSessionId,
    },
    AcquireTerminalLease {
        session_id: TerminalSessionId,
        mode: TerminalLeaseMode,
    },
    ReleaseTerminalLease {
        session_id: TerminalSessionId,
    },
    TerminalInput(TerminalInput),
    ResizeTerminal(ResizeTerminal),
    ScrollTerminal(ScrollTerminal),
    ReadTerminalViewport(ReadTerminalViewport),
    SearchTerminal(SearchTerminal),
    SetTerminalQueryPalette(SetTerminalQueryPalette),
    RequestCheckpoint {
        session_id: TerminalSessionId,
        after_sequence: Option<u64>,
    },
    AcknowledgeTerminalExit {
        session_id: TerminalSessionId,
        session_epoch: u64,
        final_sequence: u64,
    },
    TerminateTerminal {
        session_id: TerminalSessionId,
        mode: TerminationMode,
    },
    TerminateMany {
        requests: Vec<TerminateTerminalRequest>,
    },
    SshConnect(SshConnectSpec),
    SshDisconnect {
        connection_id: String,
    },
    CredentialAnswer {
        challenge_id: u64,
        answer: CredentialAnswer,
    },
    DeleteSshCredential {
        credential_id: String,
    },
    RemoteFile(RemoteFileRequest),
    RemoteCommand(RemoteCommandRequest),
    Project(ProjectRequest),
    ReadAgentSnapshots {
        acknowledged: Vec<AgentSnapshotCursor>,
    },
    RequestTerminalControl {
        session_id: TerminalSessionId,
    },
    ReleaseTerminalControl {
        session_id: TerminalSessionId,
    },
    Workspace(WorkspaceRequest),
    ProfileControl(crate::session::ProfileControlRequest),
    ReadDeviceSettings,
    SetLoginStartupConsent {
        granted: bool,
    },
    RemoteAccess(crate::remote_access::RemoteAccessRequest),
}

impl Request {
    pub fn required_capability(&self) -> Option<Capability> {
        match self {
            Self::Ping { .. }
            | Self::ProfileControl(_)
            | Self::ReadDeviceSettings
            | Self::SetLoginStartupConsent { .. }
            | Self::RemoteAccess(_)
            | Self::ListResources
            | Self::DetachTerminal { .. }
            | Self::ScrollTerminal(_)
            | Self::ReadTerminalViewport(_)
            | Self::SearchTerminal(_)
            | Self::RequestCheckpoint { .. }
            | Self::AcknowledgeTerminalExit { .. }
            | Self::ReadAgentSnapshots { .. } => None,
            Self::SpawnTerminal(_)
            | Self::TerminalInput(_)
            | Self::ResizeTerminal(_)
            | Self::SetTerminalQueryPalette(_)
            | Self::RequestTerminalControl { .. }
            | Self::ReleaseTerminalControl { .. }
            | Self::ReleaseTerminalLease { .. }
            | Self::TerminateMany { .. } => Some(Capability::TerminalInteractive),
            Self::AttachTerminal(attach) => (attach.mode == TerminalLeaseMode::Interactive)
                .then_some(Capability::TerminalInteractive),
            Self::AcquireTerminalLease { mode, .. } => {
                (*mode == TerminalLeaseMode::Interactive).then_some(Capability::TerminalInteractive)
            }
            Self::TerminateTerminal { mode, .. } => match mode {
                TerminationMode::Detach => None,
                TerminationMode::Terminate | TerminationMode::TerminateMany => {
                    Some(Capability::TerminalInteractive)
                }
            },
            Self::SshConnect(_) | Self::SshDisconnect { .. } | Self::DeleteSshCredential { .. } => {
                Some(Capability::SshConnect)
            }
            Self::CredentialAnswer { .. } => Some(Capability::CredentialAnswer),
            Self::RemoteFile(request) => Some(request.required_capability()),
            Self::RemoteCommand(request) => Some(request.required_capability()),
            Self::Project(request) => Some(request.required_capability()),
            Self::Workspace(request) => Some(match request {
                WorkspaceRequest::Environment
                | WorkspaceRequest::List
                | WorkspaceRequest::Browse { .. }
                | WorkspaceRequest::ReadConfig { .. }
                | WorkspaceRequest::ListConfig { .. }
                | WorkspaceRequest::AgentSessions { .. }
                | WorkspaceRequest::Open { .. }
                | WorkspaceRequest::GetDraft { .. } => Capability::WorkspaceRead,
                _ => Capability::WorkspaceMutate,
            }),
        }
    }

    pub fn audit_action(&self) -> Option<&'static str> {
        Some(match self.required_capability()? {
            Capability::TerminalInteractive => "terminal.mutate",
            Capability::ProjectRead => "project.read",
            Capability::ProjectMutate => "project.mutate",
            Capability::GitRead => "git.read",
            Capability::GitMutate => "git.mutate",
            Capability::SshConnect => "ssh.connect",
            Capability::RemoteCommandPrivileged => "remote.privileged",
            Capability::CredentialAnswer => "credential.answer",
            Capability::WorkspaceRead => "workspace.read",
            Capability::WorkspaceMutate => "workspace.mutate",
        })
    }

    pub fn audit_resource(&self) -> String {
        match self {
            Self::SpawnTerminal(spec) => spec.session_id.to_string(),
            Self::AttachTerminal(attach) => attach.session_id.to_string(),
            Self::DetachTerminal { session_id }
            | Self::AcquireTerminalLease { session_id, .. }
            | Self::ReleaseTerminalLease { session_id }
            | Self::RequestTerminalControl { session_id }
            | Self::ReleaseTerminalControl { session_id }
            | Self::RequestCheckpoint { session_id, .. }
            | Self::AcknowledgeTerminalExit { session_id, .. }
            | Self::TerminateTerminal { session_id, .. } => session_id.to_string(),
            Self::TerminalInput(input) => input.session_id.to_string(),
            Self::ResizeTerminal(request) => request.session_id.to_string(),
            Self::ScrollTerminal(request) => request.session_id.to_string(),
            Self::ReadTerminalViewport(request) => request.session_id.to_string(),
            Self::SearchTerminal(request) => request.session_id.to_string(),
            Self::SetTerminalQueryPalette(request) => request.session_id.to_string(),
            Self::SshConnect(spec) => spec.connection_id.clone(),
            Self::SshDisconnect { connection_id }
            | Self::DeleteSshCredential {
                credential_id: connection_id,
            } => connection_id.clone(),
            Self::CredentialAnswer { challenge_id, .. } => challenge_id.to_string(),
            Self::RemoteFile(request) => remote_file_resource(request),
            Self::RemoteCommand(request) => request.project_id.to_string(),
            Self::Workspace(_)
            | Self::ProfileControl(_)
            | Self::ReadDeviceSettings
            | Self::SetLoginStartupConsent { .. }
            | Self::RemoteAccess(_) => String::new(),
            Self::Project(request) => project_resource(request),
            Self::TerminateMany { requests } => requests
                .iter()
                .map(|request| request.session_id.as_str())
                .collect::<Vec<_>>()
                .join(","),
            Self::Ping { .. } | Self::ListResources | Self::ReadAgentSnapshots { .. } => {
                String::new()
            }
        }
    }
}

fn remote_file_resource(request: &RemoteFileRequest) -> String {
    match request {
        RemoteFileRequest::ResolveHome { connection_id }
        | RemoteFileRequest::BrowseDirectory { connection_id, .. } => connection_id.clone(),
        RemoteFileRequest::ScanDirectory { project_id, .. }
        | RemoteFileRequest::Read { project_id, .. }
        | RemoteFileRequest::Save { project_id, .. }
        | RemoteFileRequest::Create { project_id, .. }
        | RemoteFileRequest::Rename { project_id, .. }
        | RemoteFileRequest::Delete { project_id, .. } => project_id.to_string(),
    }
}

fn project_resource(request: &ProjectRequest) -> String {
    match request {
        ProjectRequest::Register { project_id, .. }
        | ProjectRequest::Observe { project_id, .. }
        | ProjectRequest::RegisterSsh { project_id, .. }
        | ProjectRequest::Close { project_id, .. }
        | ProjectRequest::ScanDirectory { project_id, .. }
        | ProjectRequest::ReadFile { project_id, .. }
        | ProjectRequest::SaveFile { project_id, .. }
        | ProjectRequest::CreateEntry { project_id, .. }
        | ProjectRequest::RenameEntry { project_id, .. }
        | ProjectRequest::DeleteEntry { project_id, .. }
        | ProjectRequest::Git { project_id, .. } => project_id.to_string(),
        ProjectRequest::PasteEntry {
            source_project_id, ..
        } => source_project_id.to_string(),
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Response {
    Pong {
        sent_millis: u64,
        host_millis: u64,
    },
    Resources(ResourceCatalog),
    TerminalSpawned {
        lease: TerminalLease,
        session_epoch: u64,
    },
    TerminalAttached {
        lease: TerminalLease,
        checkpoint: TerminalCheckpoint,
    },
    TerminalDetached,
    TerminalLease(TerminalLease),
    TerminalInputAccepted {
        client_sequence: u64,
    },
    TerminalResized {
        geometry_epoch: u64,
    },
    TerminalScrolled(TerminalViewportRead),
    TerminalViewport(TerminalViewportRead),
    TerminalSearch(TerminalSearchResults),
    TerminalPaletteAccepted {
        revision: u64,
    },
    TerminalCheckpoint(TerminalCheckpoint),
    TerminalExitAcknowledged,
    TerminalTerminated(TerminatedTerminal),
    TerminalsTerminated {
        results: Vec<TerminalTerminationResult>,
    },
    SshConnected {
        connection_id: String,
        epoch: u64,
    },
    SshDisconnected,
    CredentialAccepted,
    CredentialDeleted,
    RemoteFile(RemoteFileResponse),
    RemoteCommand(RemoteCommandResponse),
    Project(ProjectResponse),
    AgentSnapshots(Vec<AgentSnapshotUpdate>),
    Applied,
    TerminalControlPending {
        session_id: TerminalSessionId,
        holder: ClientInstanceId,
    },
    Workspace(WorkspaceResponse),
    ProfileControl(crate::session::ControlStatus),
    RemoteAccess(crate::remote_access::RemoteAccessResponse),
    DeviceSettings(crate::remote_access::RemoteAccessSettings),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ServerEvent {
    Terminal(TerminalStreamUpdate),
    ProfileControl(crate::session::ControlStatus),
    TerminalLeaseRevoked {
        session_id: TerminalSessionId,
        previous_owner: ClientInstanceId,
    },
    TerminalExit {
        session_id: TerminalSessionId,
        session_epoch: u64,
        code: Option<i32>,
        final_sequence: u64,
    },
    SshStateChanged(SshConnectionStatus),
    CredentialChallenge(CredentialChallenge),
    SshCredentialSaved {
        connection_id: String,
        epoch: u64,
        credential: StoredSshCredential,
    },
    ProjectChanged(ProjectChange),
    AgentSnapshot(Box<AgentSnapshotUpdate>),
    ResourceCatalogChanged,
    HostDraining,
    HostStopping,
    TerminalControlRequested {
        session_id: TerminalSessionId,
        requester: ClientInstanceId,
    },
    TerminalControlGranted {
        lease: TerminalLease,
    },
    TerminalLeaseReleased {
        session_id: TerminalSessionId,
        previous_owner: ClientInstanceId,
    },
    TerminalLeaseExpired {
        session_id: TerminalSessionId,
        previous_owner: ClientInstanceId,
    },
    TerminalControlDenied {
        session_id: TerminalSessionId,
        requester: ClientInstanceId,
        reason: TerminalControlDeniedReason,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TerminalControlDeniedReason {
    TimedOut,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalLease {
    pub session_id: TerminalSessionId,
    pub owner: ClientInstanceId,
    pub mode: TerminalLeaseMode,
    pub lease_epoch: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceCatalog {
    pub profile_id: ProfileId,
    pub host_id: HostId,
    pub host_epoch: u64,
    pub revision: u64,
    pub terminals: Vec<TerminalPlacement>,
    pub ssh_connections: Vec<String>,
    pub projects: Vec<ProjectId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalPlacement {
    pub session_id: TerminalSessionId,
    pub session_epoch: u64,
    pub project_id: ProjectId,
    pub geometry: TerminalGeometry,
    pub geometry_epoch: u64,
    pub last_sequence: u64,
    pub spawn_fingerprint: u64,
    pub owner: Option<ClientInstanceId>,
    pub process_state: TerminalProcessState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FailureCode {
    InvalidRequest,
    NotFound,
    AlreadyExists,
    AddressConflict,
    Conflict,
    PermissionDenied,
    AuthenticationFailed,
    VersionMismatch,
    StaleEpoch,
    StaleSequence,
    Backpressure,
    ResourceLimit,
    TransportClosed,
    HostStopping,
    Internal,
    ResyncRequired,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProtocolFailure {
    pub code: FailureCode,
    pub message: String,
    pub retryable: bool,
}

impl ProtocolFailure {
    pub fn new(code: FailureCode, message: impl Into<String>, retryable: bool) -> Self {
        Self {
            code,
            message: message.into(),
            retryable,
        }
    }
}

#[cfg(test)]
mod capability_tests {
    use yttt_core::model::ids::{ProjectId, TerminalSessionId};

    use super::*;
    use crate::{
        project::{ProjectGitOperation, ProjectRequest},
        ssh::{RemoteCommandRequest, RemoteFileRequest, RemoteHostCommand},
        terminal::{AttachTerminal, TerminalGeometry, TerminalLeaseMode, TerminationMode},
    };

    fn session() -> TerminalSessionId {
        TerminalSessionId::new("session")
    }

    fn project() -> ProjectId {
        ProjectId::new("project")
    }

    fn attach(mode: TerminalLeaseMode) -> AttachTerminal {
        AttachTerminal {
            session_id: session(),
            known_session_epoch: None,
            after_sequence: None,
            mode,
            geometry: TerminalGeometry::default(),
            geometry_epoch: 1,
            query_palette: Vec::new(),
            palette_revision: 1,
        }
    }

    #[test]
    fn mutating_requests_declare_a_capability() {
        let mutating = [
            Request::SpawnTerminal(crate::terminal::TerminalSpawnSpec {
                session_id: session(),
                project_id: project(),
                cwd: crate::ProjectRelativePath::root(),
                execution: crate::terminal::TerminalExecutionSpec::Shell {
                    program: "/bin/sh".to_string(),
                    args: Vec::new(),
                    initial_command: None,
                },
                geometry: TerminalGeometry::default(),
                geometry_epoch: 1,
                query_palette: Vec::new(),
                palette_revision: 1,
                environment: Vec::new(),
                removed_environment: Vec::new(),
                scrollback_limit: 1,
            }),
            Request::AttachTerminal(attach(TerminalLeaseMode::Interactive)),
            Request::AcquireTerminalLease {
                session_id: session(),
                mode: TerminalLeaseMode::Interactive,
            },
            Request::ReleaseTerminalLease {
                session_id: session(),
            },
            Request::RequestTerminalControl {
                session_id: session(),
            },
            Request::ReleaseTerminalControl {
                session_id: session(),
            },
            Request::TerminateTerminal {
                session_id: session(),
                mode: TerminationMode::Terminate,
            },
            Request::SshConnect(crate::ssh::SshConnectSpec {
                connection_id: "ssh".to_string(),
                endpoint: crate::ssh::SshEndpoint {
                    host: "localhost".to_string(),
                    port: 22,
                    username: "user".to_string(),
                },
                authentication: crate::ssh::SshAuthentication::Agent,
                reconnect: false,
            }),
            Request::CredentialAnswer {
                challenge_id: 1,
                answer: crate::ssh::CredentialAnswer::Cancelled,
            },
            Request::RemoteFile(RemoteFileRequest::Save {
                project_id: project(),
                relative_path: "file.txt".to_string(),
                expected: None,
                force: true,
                maximum_bytes: 16,
                bytes: Vec::new(),
            }),
            Request::RemoteCommand(RemoteCommandRequest {
                project_id: project(),
                command: RemoteHostCommand::Privileged {
                    program: "id".to_string(),
                    args: Vec::new(),
                },
            }),
            Request::Project(ProjectRequest::Git {
                project_id: project(),
                operation: ProjectGitOperation::Switch {
                    name: "main".to_string(),
                    track_remote: false,
                },
            }),
            Request::Project(ProjectRequest::SaveFile {
                project_id: project(),
                relative_path: crate::ProjectRelativePath::from_utf8("notes.txt").unwrap(),
                text: "x".to_string(),
                mode: crate::project::ProjectSaveMode::Force,
            }),
        ];
        for request in mutating {
            assert!(
                request.required_capability().is_some(),
                "mutating request must declare a capability: {request:?}"
            );
        }
    }

    #[test]
    fn observer_and_read_paths_do_not_require_a_capability() {
        assert_eq!(Request::Ping { sent_millis: 1 }.required_capability(), None);
        assert_eq!(
            Request::AttachTerminal(attach(TerminalLeaseMode::Observer)).required_capability(),
            None
        );
        assert_eq!(
            Request::Project(ProjectRequest::ReadFile {
                project_id: project(),
                relative_path: crate::ProjectRelativePath::root(),
            })
            .required_capability(),
            Some(Capability::ProjectRead)
        );
        assert_eq!(
            Request::Project(ProjectRequest::Git {
                project_id: project(),
                operation: ProjectGitOperation::Status { work_tree: None },
            })
            .required_capability(),
            Some(Capability::GitRead)
        );
    }
}
