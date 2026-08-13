use serde::{Deserialize, Serialize};
use yttt_core::model::ids::{
    ClientInstanceId, HostId, PaneId, ProfileId, ProjectId, TabId, TerminalSessionId,
};

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
        SemanticViewport, SetTerminalQueryPalette, TerminalCheckpoint, TerminalGeometry,
        TerminalInput, TerminalLeaseMode, TerminalSearchResults, TerminalSpawnSpec,
        TerminalStreamUpdate, TerminalViewportRead, TerminateTerminalRequest, TerminatedTerminal,
        TerminationMode,
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
    StopIfIdle,
    ForceStop,
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
    HostIdle,
    HostBusy {
        blockers: Vec<HostBlocker>,
    },
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
    pub spawn_fingerprint: u64,
    pub owner: Option<ClientInstanceId>,
    pub viewport: Option<SemanticViewport>,
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
