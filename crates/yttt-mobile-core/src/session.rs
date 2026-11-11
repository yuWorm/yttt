//! A mobile Client's link to one paired Host: pairing, the `ClientCore`
//! session, observing terminals, and taking control to type.

use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use yttt_client_core::{ClientCore, ClientEvent, ConnectionState, p2p::PairedHost};
use yttt_core::model::ids::{ClientInstanceId, TerminalSessionId};
use yttt_protocol::{
    BuildIdentity, ConnectionChannel, ProtocolRange, RESOURCE_PROTOCOL_VERSION, Request, Response,
    ServerEvent,
    p2p::PairingCode,
    terminal::{
        AttachTerminal, ResizeTerminal, TerminalGeometry, TerminalInput, TerminalLeaseMode,
        TerminalMutationContext,
    },
    workspace::{WorkspaceRequest, WorkspaceResponse, WorkspaceSummary},
};
use yttt_transport::{ClientIdentity, TransportConnector};
use yttt_transport_iroh::{
    DirectPaths, EndpointConfig, IrohEndpoint, OpenGate, RelaySetup, RelayTarget,
};

use crate::vt::VtEncoder;

/// The visibility scope this Client reports; it has one screen.
const VISIBILITY_SCOPE: u64 = 1;
const CONTROL_GRANT_WAIT: Duration = Duration::from_secs(5);

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
}

/// Which relay the endpoint uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RelayChoice {
    /// The relay the Host named when pairing.
    FromHost,
    /// None: direct paths only (tests, LAN).
    Off,
}

impl RelayChoice {
    fn setup(self, hint: Option<&str>) -> RelaySetup {
        match self {
            Self::Off => RelaySetup::Off,
            Self::FromHost => hint
                .map(|url| RelayTarget {
                    url: url.to_string(),
                    token: None,
                })
                .into(),
        }
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
}

async fn bind(secret: [u8; 32], relay: RelaySetup) -> Result<IrohEndpoint, MobileError> {
    IrohEndpoint::bind(EndpointConfig {
        secret,
        relay,
        alpns: Vec::new(),
        gate: Arc::new(OpenGate),
        direct: DirectPaths::Any,
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
    let endpoint = bind(device_secret, relay.setup(code.relay_url.as_deref())).await?;
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

/// `drain` timing and volume, for the spike's on-screen counters.
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
    grants: tokio::sync::Notify,
}

/// A connected session with one Host.
pub struct MobileSession {
    client: Arc<ClientCore>,
    client_id: ClientInstanceId,
    shared: Arc<Shared>,
    events: tokio::task::JoinHandle<()>,
    endpoint: Option<IrohEndpoint>,
}

impl MobileSession {
    /// Connects to a paired Host over P2P.
    pub async fn connect(
        record: &PairedHostRecord,
        device_secret: [u8; 32],
        relay: RelayChoice,
    ) -> Result<Self, MobileError> {
        let endpoint = bind(device_secret, relay.setup(record.host.relay_url.as_deref())).await?;
        let connector = record.host.connector(endpoint.clone());
        let session =
            Self::connect_with(connector, record, record.host.credential(device_secret)).await;
        match session {
            Ok(mut session) => {
                session.endpoint = Some(endpoint);
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
            grants: tokio::sync::Notify::new(),
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
        self.request(Request::Workspace(WorkspaceRequest::Open {
            workspace_id: active.workspace_id.clone(),
        }))
        .await?;
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
        terminal.session_epoch = viewport.session_epoch;
        terminal.geometry_epoch = viewport.geometry_epoch;
        let bytes = terminal.encoder.encode(&viewport);
        drop(attached);
        let mut times = self.shared.times.lock();
        times.bytes += bytes.len() as u64;
        times.micros.push(started.elapsed().as_micros() as u64);
        (!bytes.is_empty()).then_some(bytes)
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

    /// Takes terminal control for this device. The Host moves every terminal's
    /// lease here (see the design's open question 1); returns once the lease
    /// of each attached terminal arrived.
    pub async fn take_control(&self) -> Result<(), MobileError> {
        self.request(Request::TakeTerminalController).await?;
        tokio::time::timeout(CONTROL_GRANT_WAIT, async {
            loop {
                let granted = self.shared.grants.notified();
                if self
                    .shared
                    .attached
                    .lock()
                    .values()
                    .all(|terminal| terminal.lease_epoch != 0)
                {
                    return;
                }
                granted.await;
            }
        })
        .await
        .map_err(|_| MobileError::Host("the Host did not grant the terminal leases".into()))
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

    pub async fn close(&self) {
        self.events.abort();
        self.client.shutdown().await;
        if let Some(endpoint) = &self.endpoint {
            endpoint.close().await;
        }
    }

    pub fn client_instance_id(&self) -> &str {
        self.client_id.as_str()
    }
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
                ServerEvent::TerminalControlGranted { lease } => {
                    if let Some(terminal) = shared.attached.lock().get_mut(&lease.session_id) {
                        let interactive = lease.mode == TerminalLeaseMode::Interactive;
                        terminal.lease_epoch = if interactive { lease.lease_epoch } else { 0 };
                        terminal.client_sequence = 0;
                        terminal.encoder.set_interactive(interactive);
                        terminal.dirty = true;
                    }
                    shared.grants.notify_waiters();
                }
                ServerEvent::TerminalControllerChanged(controller)
                    if controller.controller.as_ref() != Some(&me) =>
                {
                    for terminal in shared.attached.lock().values_mut() {
                        terminal.lease_epoch = 0;
                        terminal.encoder.set_interactive(false);
                        terminal.dirty = true;
                    }
                }
                _ => {}
            },
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
