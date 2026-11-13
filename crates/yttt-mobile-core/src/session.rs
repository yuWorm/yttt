//! A mobile Client's link to one paired Host: pairing, the `ClientCore`
//! session, observing terminals, and taking control to type.

use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use yttt_client_core::{
    ClientCore, ClientCoreError, ClientEvent, ConnectionState, p2p::PairedHost,
};
use yttt_core::model::ids::{ClientInstanceId, TerminalSessionId};
use yttt_protocol::{
    BuildIdentity, ConnectionChannel, FailureCode, ProtocolRange, RESOURCE_PROTOCOL_VERSION,
    Request, Response, ServerEvent, TerminalLease,
    p2p::PairingCode,
    terminal::{
        AttachTerminal, ResizeTerminal, TerminalGeometry, TerminalInput, TerminalLeaseMode,
        TerminalMutationContext,
    },
    workspace::{WorkspaceId, WorkspaceRequest, WorkspaceResponse, WorkspaceSummary},
};
use yttt_transport::{ClientIdentity, TransportConnector};
use yttt_transport_iroh::{
    DirectPaths, EndpointConfig, IrohConnector, IrohEndpoint, OpenGate, PathKind, RelaySetup,
    RelayTarget,
};

use crate::{
    files::{FileEntry, OpenedFile, SESSION_PROVIDERS, SaveOutcome, SessionEntry},
    keys::TerminalKey,
    overview::Overview,
    vt::VtEncoder,
};
use yttt_core::model::host_workspace::HostWorkspaceState;
use yttt_core::model::ids::ProjectId;
use yttt_protocol::{
    HostPath, ProjectRelativePath,
    project::{
        ProjectFileFingerprint, ProjectRequest, ProjectResponse, ProjectSaveMode, ProjectSaveResult,
    },
};

/// The visibility scope this Client reports; it has one screen.
const VISIBILITY_SCOPE: u64 = 1;

#[derive(Debug, thiserror::Error)]
pub enum MobileError {
    #[error("the pairing code is invalid: {0}")]
    InvalidCode(String),
    #[error("pairing failed: {0}")]
    Pairing(String),
    #[error("cannot reach the Host: {0}")]
    Transport(String),
    #[error("the Host refused: {0}")]
    Host(String),
    #[error("the paired Host record is unreadable: {0}")]
    Record(String),
    #[error("the terminal is not attached")]
    NotAttached,
    #[error("this device does not control the terminal")]
    NotInControl,
    #[error("the Host is too old to hand over a single terminal; update yttt on the computer")]
    HostTooOld,
}

/// Which relay the endpoint uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RelayChoice {
    /// The relay the Host named when pairing.
    FromHost,
    /// None: direct paths only (tests, LAN).
    Off,
    /// The Host's relay and nothing else: no direct paths, and the device
    /// reveals no address. For diagnosing a link, and for tests that must
    /// go through the relay.
    OnlyFromHost,
}

impl RelayChoice {
    /// The endpoint's relay and direct paths, given the Host's relay `hint`.
    fn setup(self, hint: Option<&str>) -> Result<(RelaySetup, DirectPaths), MobileError> {
        let relay = || {
            hint.map(|url| RelayTarget {
                url: url.to_string(),
                token: None,
            })
        };
        Ok(match self {
            Self::Off => (RelaySetup::Off, DirectPaths::Any),
            Self::FromHost => (relay().into(), DirectPaths::Any),
            // Without a relay such an endpoint reaches nothing; say so now
            // rather than after the connect timeout.
            Self::OnlyFromHost => (
                RelaySetup::Use(
                    relay()
                        .ok_or_else(|| MobileError::Transport("the Host has no relay".into()))?,
                ),
                DirectPaths::Off,
            ),
        })
    }
}

/// What the app stores for a paired Host, next to the device secret it keeps
/// in the Keychain or Keystore.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairedHostRecord {
    pub host: PairedHost,
}

impl PairedHostRecord {
    pub fn to_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(self).expect("a paired Host record serializes")
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, MobileError> {
        serde_json::from_slice(bytes).map_err(|error| MobileError::Record(error.to_string()))
    }

    pub fn info(&self) -> HostInfo {
        HostInfo {
            id: self.host.host.to_string(),
            label: self.host.host_label.clone(),
        }
    }
}

/// Who a stored record names: `id` stays the same when the phone pairs with
/// the same Host again, so the phone keeps one entry per Host.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct HostInfo {
    pub id: String,
    pub label: String,
}

#[cfg(test)]
mod record_tests {
    use super::*;
    use yttt_core::model::ids::ProfileId;
    use yttt_protocol::EndpointKey;

    fn record(key: u8, label: &str) -> PairedHostRecord {
        PairedHostRecord {
            host: PairedHost {
                host: EndpointKey([key; 32]),
                environment_id: format!("environment-{key}"),
                profile_id: ProfileId::new("default"),
                host_label: label.into(),
                relay_url: None,
                direct_addrs: Vec::new(),
            },
        }
    }

    #[test]
    fn a_host_keeps_its_id_through_storage_and_hosts_differ() {
        let stored = PairedHostRecord::from_bytes(&record(1, "mac").to_bytes()).unwrap();
        assert_eq!(stored.info(), record(1, "mac").info());
        assert_eq!(stored.info().label, "mac");
        assert!(!stored.info().id.is_empty());
        assert_ne!(record(1, "mac").info().id, record(2, "mac").info().id);
    }
}

async fn bind(
    secret: [u8; 32],
    (relay, direct): (RelaySetup, DirectPaths),
) -> Result<IrohEndpoint, MobileError> {
    IrohEndpoint::bind(EndpointConfig {
        secret,
        relay,
        alpns: Vec::new(),
        gate: Arc::new(OpenGate),
        direct,
    })
    .await
    .map_err(|error| MobileError::Transport(error.to_string()))
}

/// Pairs this device with the Host named in `code_text`. `on_short_code`
/// receives the six digits the user compares with the Host's screen.
pub async fn pair(
    code_text: &str,
    device_label: &str,
    device_secret: [u8; 32],
    relay: RelayChoice,
    on_short_code: impl FnOnce(u32),
) -> Result<PairedHostRecord, MobileError> {
    let code = PairingCode::decode(code_text.trim())
        .map_err(|error| MobileError::InvalidCode(format!("{error:?}")))?;
    let endpoint = bind(device_secret, relay.setup(code.relay_url.as_deref())?).await?;
    let result = yttt_client_core::p2p::pair(&endpoint, &code, device_label, on_short_code).await;
    endpoint.close().await;
    let host = result.map_err(|error| MobileError::Pairing(error.to_string()))?;
    Ok(PairedHostRecord { host })
}

/// Terminals the Host runs.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct TerminalInfo {
    pub session_id: String,
    pub project_id: String,
    pub cols: u16,
    pub rows: u16,
}

/// `drain` timing and volume, for the on-screen counters.
#[derive(Clone, Copy, Debug, Default, PartialEq, uniffi::Record)]
pub struct DrainStats {
    pub bytes: u64,
    pub frames: u64,
    pub p50_micros: u64,
    pub p95_micros: u64,
    pub max_micros: u64,
}

struct Attached {
    encoder: VtEncoder,
    /// `(session_epoch, sequence)` of the viewport last encoded.
    encoded: Option<(u64, u64)>,
    /// Interactive state changed since the last encode.
    dirty: bool,
    session_epoch: u64,
    /// Geometry epoch of the latest viewport seen.
    geometry_epoch: u64,
    /// Non-zero while this device holds the Interactive lease.
    lease_epoch: u64,
    client_sequence: u64,
}

#[derive(Default)]
struct DrainTimes {
    bytes: u64,
    micros: Vec<u64>,
}

struct Shared {
    attached: Mutex<HashMap<TerminalSessionId, Attached>>,
    times: Mutex<DrainTimes>,
    /// The workspace `open_active_workspace` opened, kept current with the
    /// Host's layout updates.
    workspace: Mutex<Option<(WorkspaceId, HostWorkspaceState)>>,
    /// Each file opened to edit, by project and path: what it looked like
    /// when last read or saved, to save against.
    opened_files: Mutex<HashMap<(String, String), ProjectFileFingerprint>>,
}

/// A connected session with one Host.
pub struct MobileSession {
    client: Arc<ClientCore>,
    client_id: ClientInstanceId,
    shared: Arc<Shared>,
    events: tokio::task::JoinHandle<()>,
    endpoint: Option<IrohEndpoint>,
    /// The P2P connector, which knows the path in use.
    connector: Option<Arc<IrohConnector>>,
    /// The task calling back with overviews, replaced by each `watch_overview`.
    overview_watch: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

/// The path a P2P session's data takes now.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum LinkPath {
    /// Straight to the Host.
    Direct,
    /// Through the relay.
    Relay,
    /// Not known yet, or not a P2P session.
    Unknown,
}

impl MobileSession {
    /// Connects to a paired Host over P2P.
    pub async fn connect(
        record: &PairedHostRecord,
        device_secret: [u8; 32],
        relay: RelayChoice,
    ) -> Result<Self, MobileError> {
        let endpoint = bind(
            device_secret,
            relay.setup(record.host.relay_url.as_deref())?,
        )
        .await?;
        let connector = Arc::new(record.host.connector(endpoint.clone()));
        let session = Self::connect_with(
            connector.clone(),
            record,
            record.host.credential(device_secret),
        )
        .await;
        match session {
            Ok(mut session) => {
                session.endpoint = Some(endpoint);
                session.connector = Some(connector);
                Ok(session)
            }
            Err(error) => {
                endpoint.close().await;
                Err(error)
            }
        }
    }

    /// Connects through `connector`; tests pass their own.
    pub async fn connect_with(
        connector: impl TransportConnector,
        record: &PairedHostRecord,
        credential: impl Into<yttt_transport::Credential>,
    ) -> Result<Self, MobileError> {
        // A fresh Client ID for every session, like the desktop. The Host binds
        // an ID to the authenticated session that first used it and keeps that
        // binding for 5 minutes after it disconnects, refusing any other
        // session for the ID; and its request journal replays by request ID,
        // which a new process starts again from 1.
        let suffix: u64 = rand::random();
        let client_id = ClientInstanceId::new(format!("mobile-client-{suffix:016x}"));
        let identity = ClientIdentity {
            expected_environment: Some(record.host.environment_id.clone()),
            credential_generation: 0,
            session_nonce: yttt_transport::new_session_nonce(),
            supported: ProtocolRange::exact(RESOURCE_PROTOCOL_VERSION),
            build: BuildIdentity {
                product_version: env!("CARGO_PKG_VERSION").into(),
                build_fingerprint: option_env!("YTTT_BUILD_FINGERPRINT")
                    .unwrap_or(env!("CARGO_PKG_VERSION"))
                    .into(),
                resource_compatibility: "yttt-resource-v1".into(),
            },
            profile_id: record.host.profile_id.clone(),
            client_instance_id: client_id.clone(),
            host_epoch_hint: None,
            can_force_stop: false,
            channel: ConnectionChannel::Session,
        };
        let client = Arc::new(
            ClientCore::connect(connector, identity, credential)
                .await
                .map_err(|error| MobileError::Transport(error.to_string()))?,
        );
        let shared = Arc::new(Shared {
            attached: Mutex::new(HashMap::new()),
            times: Mutex::new(DrainTimes::default()),
            workspace: Mutex::new(None),
            opened_files: Mutex::new(HashMap::new()),
        });
        let events = tokio::spawn(follow_events(
            client.subscribe_events(),
            shared.clone(),
            client_id.clone(),
        ));
        Ok(Self {
            client,
            client_id,
            shared,
            events,
            endpoint: None,
            connector: None,
            overview_watch: Mutex::new(None),
        })
    }

    pub fn client(&self) -> &ClientCore {
        &self.client
    }

    pub fn is_ready(&self) -> bool {
        matches!(self.client.state(), ConnectionState::Ready { .. })
    }

    /// Opens the workspace the Host's desktop shows, without changing any
    /// shared view.
    pub async fn open_active_workspace(&self) -> Result<Option<WorkspaceSummary>, MobileError> {
        let Response::Workspace(WorkspaceResponse::Workspaces(workspaces)) = self
            .request(Request::Workspace(WorkspaceRequest::List))
            .await?
        else {
            return Err(MobileError::Host(
                "unexpected workspace list response".into(),
            ));
        };
        let Some(active) = workspaces.into_iter().find(|workspace| workspace.active) else {
            return Ok(None);
        };
        let Response::Workspace(WorkspaceResponse::Opened { layout, .. }) = self
            .request(Request::Workspace(WorkspaceRequest::Open {
                workspace_id: active.workspace_id.clone(),
            }))
            .await?
        else {
            return Err(MobileError::Host(
                "unexpected workspace open response".into(),
            ));
        };
        *self.shared.workspace.lock() = Some((active.workspace_id.clone(), layout));
        Ok(Some(active))
    }

    /// Terminals the Host runs, from its resource catalog.
    pub async fn terminals(&self) -> Result<Vec<TerminalInfo>, MobileError> {
        let Response::Resources(catalog) = self.request(Request::ListResources).await? else {
            return Err(MobileError::Host("unexpected resource response".into()));
        };
        Ok(catalog
            .terminals
            .into_iter()
            .map(|terminal| TerminalInfo {
                session_id: terminal.session_id.to_string(),
                project_id: terminal.project_id.to_string(),
                cols: terminal.geometry.cols,
                rows: terminal.geometry.rows,
            })
            .collect())
    }

    /// Observes a terminal. Never asks for the Interactive lease: that would
    /// resize the PTY and make this device the controller.
    pub async fn attach(&self, session_id: &str) -> Result<(), MobileError> {
        let session_id = TerminalSessionId::new(session_id);
        let Response::TerminalAttached { lease, checkpoint } = self
            .request(Request::AttachTerminal(AttachTerminal {
                session_id: session_id.clone(),
                known_session_epoch: None,
                after_sequence: None,
                mode: TerminalLeaseMode::Observer,
                geometry: checkpoint_geometry(),
                geometry_epoch: 0,
                query_palette: Vec::new(),
                palette_revision: 0,
            }))
            .await?
        else {
            return Err(MobileError::Host("unexpected attach response".into()));
        };
        let interactive = lease.mode == TerminalLeaseMode::Interactive;
        let mut encoder = VtEncoder::new();
        encoder.set_interactive(interactive);
        self.shared.attached.lock().insert(
            session_id.clone(),
            Attached {
                encoder,
                encoded: None,
                dirty: false,
                session_epoch: checkpoint.viewport.session_epoch,
                geometry_epoch: checkpoint.viewport.geometry_epoch,
                lease_epoch: if interactive { lease.lease_epoch } else { 0 },
                client_sequence: 0,
            },
        );
        let visible: Vec<_> = self.shared.attached.lock().keys().cloned().collect();
        self.client.set_visible_terminals(VISIBILITY_SCOPE, visible);
        Ok(())
    }

    pub async fn detach(&self, session_id: &str) -> Result<(), MobileError> {
        let session_id = TerminalSessionId::new(session_id);
        if self.shared.attached.lock().remove(&session_id).is_none() {
            return Ok(());
        }
        let visible: Vec<_> = self.shared.attached.lock().keys().cloned().collect();
        self.client.set_visible_terminals(VISIBILITY_SCOPE, visible);
        self.request(Request::DetachTerminal { session_id }).await?;
        Ok(())
    }

    /// The repaint bytes since the last call, or `None` when nothing changed.
    /// Cheap when idle: the viewport is copied only after its sequence moved.
    pub fn drain(&self, session_id: &str) -> Option<Vec<u8>> {
        let started = Instant::now();
        let session_id = TerminalSessionId::new(session_id);
        let metadata = self.client.terminal_metadata(&session_id)?;
        let mut attached = self.shared.attached.lock();
        let terminal = attached.get_mut(&session_id)?;
        let current = (metadata.session_epoch, metadata.sequence);
        if terminal.encoded == Some(current) && !terminal.dirty {
            return None;
        }
        let viewport = self.client.terminal_snapshot(&session_id)?;
        terminal.encoded = Some((viewport.session_epoch, viewport.sequence));
        terminal.dirty = false;
        // A resize this device sent is ahead of the viewports still arriving
        // from before it: going back to theirs would make every key after it
        // stale, and the Host drops stale keys silently.
        terminal.geometry_epoch = if terminal.session_epoch == viewport.session_epoch {
            terminal.geometry_epoch.max(viewport.geometry_epoch)
        } else {
            viewport.geometry_epoch
        };
        terminal.session_epoch = viewport.session_epoch;
        let bytes = terminal.encoder.encode(&viewport);
        drop(attached);
        let mut times = self.shared.times.lock();
        times.bytes += bytes.len() as u64;
        times.micros.push(started.elapsed().as_micros() as u64);
        (!bytes.is_empty()).then_some(bytes)
    }

    /// Makes the next `drain` repaint the whole terminal, for a view that
    /// cleared itself, as termlib does when its grid changes size.
    pub fn repaint(&self, session_id: &str) {
        if let Some(terminal) = self
            .shared
            .attached
            .lock()
            .get_mut(&TerminalSessionId::new(session_id))
        {
            terminal.encoder.reset();
            terminal.encoded = None;
        }
    }

    /// The device's network changed (the app watches for that): iroh rereads
    /// the interfaces, and the session checks at once that it still reaches
    /// the Host, replacing a connection that went quiet instead of waiting
    /// for the liveness timeout. Opens no new session, so the Client ID stays.
    pub async fn network_changed(&self) {
        if let Some(endpoint) = &self.endpoint {
            endpoint.network_change().await;
        }
        self.client.probe_now();
    }

    /// Whether data goes straight to the Host or through the relay.
    pub async fn path(&self) -> LinkPath {
        let Some(connector) = &self.connector else {
            return LinkPath::Unknown;
        };
        match connector.path_kind().await {
            PathKind::Direct => LinkPath::Direct,
            PathKind::Relay => LinkPath::Relay,
            PathKind::Unknown => LinkPath::Unknown,
        }
    }

    pub fn drain_stats(&self) -> DrainStats {
        let times = self.shared.times.lock();
        let mut micros = times.micros.clone();
        micros.sort_unstable();
        let at = |quantile: f64| {
            micros
                .get(((micros.len() as f64 - 1.0) * quantile).round() as usize)
                .copied()
                .unwrap_or(0)
        };
        DrainStats {
            bytes: times.bytes,
            frames: micros.len() as u64,
            p50_micros: at(0.5),
            p95_micros: at(0.95),
            max_micros: micros.last().copied().unwrap_or(0),
        }
    }

    /// Takes this one attached terminal for this device and sizes its PTY
    /// to `cols`×`rows`. Returns once the Host applied the size, so the first
    /// key typed already sees it. Other terminals, and the computer's
    /// control of them, are left alone. A Host that cannot hand over a single
    /// terminal answers `HostTooOld`; this never falls back to taking every
    /// terminal.
    pub async fn take_control(
        &self,
        session_id: &str,
        cols: u16,
        rows: u16,
    ) -> Result<(), MobileError> {
        let id = TerminalSessionId::new(session_id);
        if !self.shared.attached.lock().contains_key(&id) {
            return Err(MobileError::NotAttached);
        }
        let lease = match self
            .client
            .request(Request::TakeTerminal {
                session_id: id.clone(),
            })
            .await
        {
            Ok(Response::TerminalLease(lease)) => lease,
            Ok(other) => return Err(MobileError::Host(format!("unexpected answer {other:?}"))),
            Err(ClientCoreError::Protocol(failure)) if failure.code == FailureCode::Unsupported => {
                return Err(MobileError::HostTooOld);
            }
            Err(error) => return Err(MobileError::Host(error.to_string())),
        };
        if let Some(terminal) = self.shared.attached.lock().get_mut(&id) {
            grant(terminal, &lease);
        }
        self.resize(session_id, cols, rows).await
    }

    /// Gives terminals back to whoever controlled them before this device
    /// took them: the listed ones, or with an empty list every one it took.
    /// Safe to repeat, as on each move to the background.
    pub async fn give_back(&self, session_ids: Vec<String>) -> Result<(), MobileError> {
        let ids = session_ids
            .into_iter()
            .map(TerminalSessionId::new)
            .collect::<Vec<_>>();
        // Stop sending keys before the Host has answered.
        for (id, terminal) in self.shared.attached.lock().iter_mut() {
            if ids.is_empty() || ids.contains(id) {
                lose(terminal);
            }
        }
        match self
            .client
            .request(Request::ReturnTerminals { session_ids: ids })
            .await
        {
            Ok(_) => Ok(()),
            // A Host this old never handed this device a single terminal.
            Err(ClientCoreError::Protocol(failure)) if failure.code == FailureCode::Unsupported => {
                Ok(())
            }
            Err(error) => Err(MobileError::Host(error.to_string())),
        }
    }

    pub fn in_control(&self, session_id: &str) -> bool {
        self.shared
            .attached
            .lock()
            .get(&TerminalSessionId::new(session_id))
            .is_some_and(|terminal| terminal.lease_epoch != 0)
    }

    pub fn send_input(&self, session_id: &str, bytes: Vec<u8>) -> Result<(), MobileError> {
        let session_id = TerminalSessionId::new(session_id);
        let context = self.next_context(&session_id, false)?;
        self.client
            .send_terminal_input(TerminalInput {
                session_id,
                context,
                bytes,
            })
            .map_err(|error| MobileError::Transport(error.to_string()))
    }

    /// Whether the program in the terminal wants application cursor keys.
    pub fn app_cursor(&self, session_id: &str) -> bool {
        self.client
            .terminal_snapshot(&TerminalSessionId::new(session_id))
            .is_some_and(|viewport| crate::vt::app_cursor(viewport.modes.bits))
    }

    /// Sends a key-bar key as the program expects it; only while in control.
    pub fn send_key(&self, session_id: &str, key: TerminalKey) -> Result<(), MobileError> {
        let bytes = crate::keys::encode(key, self.app_cursor(session_id));
        self.send_input(session_id, bytes.to_vec())
    }

    /// Resizes the PTY to this device's screen; only while in control.
    pub async fn resize(&self, session_id: &str, cols: u16, rows: u16) -> Result<(), MobileError> {
        let session_id = TerminalSessionId::new(session_id);
        let context = self.next_context(&session_id, true)?;
        self.request(Request::ResizeTerminal(ResizeTerminal {
            session_id,
            context,
            geometry: TerminalGeometry {
                cols,
                rows,
                cell_width: 0,
                cell_height: 0,
            },
        }))
        .await?;
        Ok(())
    }

    fn next_context(
        &self,
        session_id: &TerminalSessionId,
        new_geometry: bool,
    ) -> Result<TerminalMutationContext, MobileError> {
        let ConnectionState::Ready { host_epoch, .. } = self.client.state() else {
            return Err(MobileError::Transport("not connected".into()));
        };
        let mut attached = self.shared.attached.lock();
        let terminal = attached
            .get_mut(session_id)
            .ok_or(MobileError::NotAttached)?;
        if terminal.lease_epoch == 0 {
            return Err(MobileError::NotInControl);
        }
        terminal.client_sequence += 1;
        if new_geometry {
            terminal.geometry_epoch += 1;
        }
        Ok(TerminalMutationContext {
            host_epoch,
            session_epoch: terminal.session_epoch,
            lease_epoch: terminal.lease_epoch,
            geometry_epoch: terminal.geometry_epoch,
            client_sequence: terminal.client_sequence,
        })
    }

    async fn request(&self, request: Request) -> Result<Response, MobileError> {
        self.client
            .request(request)
            .await
            .map_err(|error| MobileError::Host(error.to_string()))
    }

    /// One directory of a project, `""` for its root, without hidden entries.
    pub async fn list_directory(
        &self,
        project_id: &str,
        directory: &str,
    ) -> Result<Vec<FileEntry>, MobileError> {
        let Response::Project(ProjectResponse::Directory(listing)) = self
            .request(Request::Project(ProjectRequest::ScanDirectory {
                project_id: ProjectId::new(project_id),
                relative_directory: relative(directory)?,
                show_hidden: false,
            }))
            .await?
        else {
            return Err(MobileError::Host("unexpected directory response".into()));
        };
        listing
            .entries
            .into_iter()
            .map(|entry| {
                let path = entry
                    .relative_path
                    .to_utf8()
                    .map_err(|error| MobileError::Host(error.to_string()))?;
                Ok(FileEntry {
                    name: path.rsplit('/').next().unwrap_or(&path).to_string(),
                    path,
                    kind: entry.kind.into(),
                })
            })
            .collect()
    }

    /// Reads a file to edit; the next `save_file` checks it is unchanged.
    pub async fn open_file(&self, project_id: &str, path: &str) -> Result<OpenedFile, MobileError> {
        let Response::Project(ProjectResponse::File(file)) = self
            .request(Request::Project(ProjectRequest::ReadFile {
                project_id: ProjectId::new(project_id),
                relative_path: relative(path)?,
            }))
            .await?
        else {
            return Err(MobileError::Host("unexpected file response".into()));
        };
        self.shared
            .opened_files
            .lock()
            .insert((project_id.to_string(), path.to_string()), file.fingerprint);
        Ok(OpenedFile {
            path: path.to_string(),
            text: file.text,
        })
    }

    /// Saves an opened file. Unless `force`, only if it is as it was when
    /// opened or last saved; otherwise `Conflict` and nothing is written.
    pub async fn save_file(
        &self,
        project_id: &str,
        path: &str,
        text: String,
        force: bool,
    ) -> Result<SaveOutcome, MobileError> {
        let key = (project_id.to_string(), path.to_string());
        let mode =
            if force {
                ProjectSaveMode::Force
            } else {
                let fingerprint = self.shared.opened_files.lock().get(&key).cloned();
                ProjectSaveMode::Check(fingerprint.ok_or_else(|| {
                    MobileError::Host(format!("{path} was not opened before saving"))
                })?)
            };
        let Response::Project(ProjectResponse::Save(result)) = self
            .request(Request::Project(ProjectRequest::SaveFile {
                project_id: ProjectId::new(project_id),
                relative_path: relative(path)?,
                text,
                mode,
            }))
            .await?
        else {
            return Err(MobileError::Host("unexpected save response".into()));
        };
        Ok(match result {
            ProjectSaveResult::Saved(fingerprint) => {
                self.shared.opened_files.lock().insert(key, fingerprint);
                SaveOutcome::Saved
            }
            ProjectSaveResult::Conflict(_) => SaveOutcome::Conflict,
        })
    }

    /// The Agent sessions the Host has for a project of the open workspace.
    pub async fn agent_sessions(&self, project_id: &str) -> Result<Vec<SessionEntry>, MobileError> {
        let root = {
            let workspace = self.shared.workspace.lock();
            let project = workspace.as_ref().and_then(|(_, layout)| {
                layout
                    .workspace
                    .opened_projects
                    .iter()
                    .find(|project| project.id.as_str() == project_id)
            });
            let path = project
                .and_then(|project| project.location.local_path().cloned())
                .ok_or_else(|| MobileError::Host(format!("{project_id} has no local root")))?;
            HostPath::from_path(&path).map_err(|error| MobileError::Host(error.to_string()))?
        };
        let Response::Workspace(WorkspaceResponse::AgentSessions { sessions, .. }) = self
            .request(Request::Workspace(WorkspaceRequest::AgentSessions {
                providers: SESSION_PROVIDERS
                    .iter()
                    .map(|provider| provider.to_string())
                    .collect(),
                project_root: root,
            }))
            .await?
        else {
            return Err(MobileError::Host("unexpected session response".into()));
        };
        Ok(sessions
            .into_iter()
            .map(|session| SessionEntry {
                provider: session.provider,
                id: session.id,
                title: session.title,
                model: session.model,
                updated_at_ms: session.updated_at_ms,
            })
            .collect())
    }

    /// The home screen's data as it is now.
    pub fn overview(&self) -> Overview {
        current_overview(&self.client, &self.shared)
    }

    /// Calls `callback` with the overview now and after each change: Agent
    /// states, the workspace layout, the terminals, a reconnect. Changes
    /// within [`OVERVIEW_DEBOUNCE`] arrive as one call. Replaces the callback
    /// of an earlier call; ends with `close`.
    pub fn watch_overview(&self, callback: impl Fn(Overview) + Send + Sync + 'static) {
        let client = self.client.clone();
        let shared = self.shared.clone();
        let mut events = client.subscribe_events();
        let task = tokio::spawn(async move {
            use tokio::sync::broadcast::error::RecvError;
            callback(current_overview(&client, &shared));
            loop {
                match events.recv().await {
                    Ok(event) if !changes_overview(&event) => continue,
                    Ok(_) | Err(RecvError::Lagged(_)) => {}
                    Err(RecvError::Closed) => return,
                }
                tokio::time::sleep(OVERVIEW_DEBOUNCE).await;
                // Whatever came meanwhile is in the overview about to be sent.
                while let Ok(_) | Err(tokio::sync::broadcast::error::TryRecvError::Lagged(_)) =
                    events.try_recv()
                {}
                callback(current_overview(&client, &shared));
            }
        });
        if let Some(previous) = self.overview_watch.lock().replace(task) {
            previous.abort();
        }
    }

    pub async fn close(&self) {
        self.events.abort();
        if let Some(watch) = self.overview_watch.lock().take() {
            watch.abort();
        }
        self.client.shutdown().await;
        if let Some(endpoint) = &self.endpoint {
            endpoint.close().await;
        }
    }

    pub fn client_instance_id(&self) -> &str {
        self.client_id.as_str()
    }
}

fn relative(path: &str) -> Result<ProjectRelativePath, MobileError> {
    ProjectRelativePath::from_utf8(path).map_err(|error| MobileError::Host(error.to_string()))
}

/// How long the overview waits for more changes before it is sent.
pub const OVERVIEW_DEBOUNCE: Duration = Duration::from_millis(250);

fn current_overview(client: &ClientCore, shared: &Shared) -> Overview {
    let workspace = shared.workspace.lock();
    crate::overview::build(
        workspace.as_ref().map(|(_, layout)| layout),
        &client.agent_snapshots(),
        client
            .resource_catalog()
            .map_or(0, |catalog| catalog.terminals.len()),
    )
}

fn changes_overview(event: &ClientEvent) -> bool {
    matches!(
        event,
        ClientEvent::AgentSnapshotUpdated(_)
            | ClientEvent::AgentSnapshotsReset { .. }
            | ClientEvent::AgentStateCleared(_)
            | ClientEvent::ResourceCatalogUpdated(_)
            | ClientEvent::WorkspaceLayoutChanged(_)
            | ClientEvent::Connection(_)
    )
}

/// Observers ask for no geometry; the Host keeps the PTY's own.
fn checkpoint_geometry() -> TerminalGeometry {
    TerminalGeometry {
        cols: 80,
        rows: 24,
        cell_width: 0,
        cell_height: 0,
    }
}

/// Records an Interactive lease. The answer to `TakeTerminal` and the grant
/// event that follows carry the same lease: only a new one restarts the input
/// sequence, or keys sent in between would be refused as stale.
fn grant(terminal: &mut Attached, lease: &TerminalLease) {
    if lease.mode != TerminalLeaseMode::Interactive {
        return lose(terminal);
    }
    if terminal.lease_epoch == lease.lease_epoch {
        return;
    }
    terminal.lease_epoch = lease.lease_epoch;
    terminal.client_sequence = 0;
    terminal.encoder.set_interactive(true);
    terminal.dirty = true;
}

fn lose(terminal: &mut Attached) {
    if terminal.lease_epoch == 0 {
        return;
    }
    terminal.lease_epoch = 0;
    terminal.encoder.set_interactive(false);
    terminal.dirty = true;
}

/// Tracks lease grants and losses so input carries the right epoch, and the
/// encoder syncs mouse modes only while this device is in control.
async fn follow_events(
    mut events: tokio::sync::broadcast::Receiver<ClientEvent>,
    shared: Arc<Shared>,
    me: ClientInstanceId,
) {
    use tokio::sync::broadcast::error::RecvError;
    loop {
        let event = match events.recv().await {
            Ok(event) => event,
            Err(RecvError::Lagged(_)) => continue,
            Err(RecvError::Closed) => return,
        };
        match event {
            ClientEvent::Server(event) => match event.body {
                ServerEvent::TerminalControlGranted { lease } if lease.owner == me => {
                    if let Some(terminal) = shared.attached.lock().get_mut(&lease.session_id) {
                        grant(terminal, &lease);
                    }
                }
                ServerEvent::TerminalLeaseRevoked {
                    session_id,
                    previous_owner,
                }
                | ServerEvent::TerminalLeaseReleased {
                    session_id,
                    previous_owner,
                } if previous_owner == me => {
                    if let Some(terminal) = shared.attached.lock().get_mut(&session_id) {
                        lose(terminal);
                    }
                }
                _ => {}
            },
            ClientEvent::WorkspaceLayoutChanged(update) => {
                let mut workspace = shared.workspace.lock();
                if let Some((id, layout)) = workspace.as_mut()
                    && *id == update.workspace_id
                {
                    *layout = update.layout;
                }
            }
            ClientEvent::Connection(ConnectionState::Ready { .. }) => {
                // A new connection: leases are granted again, the view repaints.
                for terminal in shared.attached.lock().values_mut() {
                    terminal.lease_epoch = 0;
                    terminal.encoder.reset();
                    terminal.encoder.set_interactive(false);
                    terminal.encoded = None;
                }
            }
            _ => {}
        }
    }
}
