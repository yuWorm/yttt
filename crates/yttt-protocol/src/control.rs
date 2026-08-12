use serde::{Deserialize, Serialize};
use yttt_core::model::ids::{
    ClientInstanceId, HostId, PaneId, ProfileId, ProjectId, TabId, TerminalSessionId,
};

use crate::{
    agent::{AgentHookEnvironment, AgentHookEvent, AgentHookScope},
    project::{ProjectChange, ProjectRequest, ProjectResponse},
    ssh::{
        CredentialAnswer, CredentialChallenge, RemoteCommandRequest, RemoteCommandResponse,
        RemoteFileRequest, RemoteFileResponse, SshConnectSpec, SshConnectionStatus,
        StoredSshCredential,
    },
    terminal::{
        AttachTerminal, SemanticViewport, TerminalCheckpoint, TerminalGeometry, TerminalInput,
        TerminalLeaseMode, TerminalSpawnSpec, TerminalStreamUpdate, TerminationMode,
    },
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientRequest {
    pub request_id: u64,
    pub body: Request,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostResponse {
    pub request_id: u64,
    pub result: Result<Response, ProtocolFailure>,
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
    ResizeTerminal {
        session_id: TerminalSessionId,
        geometry: TerminalGeometry,
        geometry_epoch: u64,
    },
    ScrollTerminal {
        session_id: TerminalSessionId,
        display_offset: u64,
    },
    SetTerminalQueryPalette {
        session_id: TerminalSessionId,
        colors: Vec<u32>,
        revision: u64,
    },
    RequestCheckpoint {
        session_id: TerminalSessionId,
        after_sequence: Option<u64>,
    },
    AcknowledgeTerminalExit {
        session_id: TerminalSessionId,
        session_epoch: u64,
    },
    TerminateTerminal {
        session_id: TerminalSessionId,
        mode: TerminationMode,
    },
    TerminateMany {
        session_ids: Vec<TerminalSessionId>,
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
    AgentHookEnvironment(AgentHookScope),
    DrainAndStop,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Response {
    Pong {
        sent_millis: u64,
        host_millis: u64,
    },
    Resources(ResourceCatalog),
    TerminalSpawned {
        session_id: TerminalSessionId,
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
    TerminalScrolled {
        display_offset: u64,
    },
    TerminalPaletteAccepted {
        revision: u64,
    },
    TerminalCheckpoint(TerminalCheckpoint),
    TerminalExitAcknowledged,
    TerminalTerminated,
    TerminalsTerminated {
        terminated: usize,
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
    AgentHookEnvironment(AgentHookEnvironment),
    Draining,
    Applied,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ServerEvent {
    Terminal(TerminalStreamUpdate),
    TerminalLeaseRevoked {
        session_id: TerminalSessionId,
        previous_owner: ClientInstanceId,
    },
    TerminalExit {
        session_id: TerminalSessionId,
        session_epoch: u64,
        code: Option<i32>,
    },
    SshStateChanged(SshConnectionStatus),
    CredentialChallenge(CredentialChallenge),
    SshCredentialSaved {
        connection_id: String,
        epoch: u64,
        credential: StoredSshCredential,
    },
    ProjectChanged(ProjectChange),
    AgentHook(AgentHookEvent),
    HostDraining,
    HostStopping,
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
    pub tab_id: TabId,
    pub pane_id: PaneId,
    pub geometry: TerminalGeometry,
    pub last_sequence: u64,
    pub owner: Option<ClientInstanceId>,
    pub viewport: Option<SemanticViewport>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FailureCode {
    InvalidRequest,
    NotFound,
    AlreadyExists,
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
