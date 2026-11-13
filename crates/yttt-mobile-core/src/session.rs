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
    account::{Account, AccountError, TICKET_WAIT, host_key},
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

/// How long a link that may go direct has to reach the Host before the phone
/// tries the relay alone. A proxy or VPN on the phone (a TUN in fake-IP mode)
/// can pass the first packets to a LAN address and drop the rest: the phone
/// thinks the handshake is done, the Host never hears the end of it, and the
/// connection stalls for its whole timeout while the relay would have worked.
const DIRECT_GRACE: Duration = Duration::from_secs(8);

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
    /// The relay account's session is no longer valid: sign in again.
    #[error("the relay account's sign-in expired")]
    SignInExpired,
}

/// Where the endpoint's relay came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum RelaySource {
    /// A ticket from the signed-in relay account, for the Host's node.
    Account,
    /// The relay the Host named when pairing, with no ticket: only a relay
    /// that admits anyone carries it.
    PairingHint,
    /// No relay: direct paths only.
    None,
}

/// A ticket for `host`'s relay node, if signed in. Failures other than an
/// expired sign-in are returned as text, so a phone that cannot reach the
/// control plane still tries the pairing relay and direct paths.
async fn account_ticket(
    account: Option<&Account>,
    device_secret: &[u8; 32],
    host: &yttt_protocol::EndpointKey,
    label: &str,
    wait: Option<Duration>,
) -> Result<Result<Option<yttt_client_core::control::RelayTicket>, String>, MobileError> {
    let Some(account) = account else {
        return Ok(Ok(None));
    };
    let lookup = account.ticket(device_secret, host, label);
    let result = match wait {
        Some(wait) => tokio::time::timeout(wait, lookup)
            .await
            .unwrap_or_else(|_| {
                Err(AccountError::Control(format!(
                    "the control plane did not answer within {} s",
                    wait.as_secs()
                )))
            }),
        None => lookup.await,
    };
    match result {
        Ok(ticket) => Ok(Ok(Some(ticket))),
        Err(AccountError::SignInExpired) => Err(MobileError::SignInExpired),
        Err(error) => {
            tracing::warn!(%error, "no relay ticket; using the pairing relay");
            Ok(Err(error.to_string()))
        }
    }
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
    /// The endpoint's relay and direct paths, given the relay to use: a
    /// ticket's, or the Host's hint.
    fn setup(self, target: Option<RelayTarget>) -> Result<(RelaySetup, DirectPaths), MobileError> {
        let relay = || target.clone();
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
mod grace_tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn a_link_that_answers_in_time_is_kept() {
        let quick = async {
            tokio::time::sleep(Duration::from_secs(1)).await;
            7
        };
        assert_eq!(
            within_grace(quick, Some(DIRECT_GRACE), || false).await,
            Some(7)
        );
        assert_eq!(within_grace(async { 8 }, None, || false).await, Some(8));
    }

    #[tokio::test(start_paused = true)]
    async fn a_link_that_never_answers_gives_way_at_the_grace() {
        let started = tokio::time::Instant::now();
        let stalled =
            within_grace(std::future::pending::<()>(), Some(DIRECT_GRACE), || false).await;
        assert_eq!(stalled, None);
        assert_eq!(started.elapsed(), DIRECT_GRACE);
    }

    #[tokio::test(start_paused = true)]
    async fn a_link_the_host_answered_runs_past_the_grace() {
        // Pairing waits on the desktop user after the short code shows.
        let slow = async {
            tokio::time::sleep(DIRECT_GRACE * 3).await;
            "accepted"
        };
        assert_eq!(
            within_grace(slow, Some(DIRECT_GRACE), || true).await,
            Some("accepted")
        );
    }
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

/// The relay a hint names, with no ticket.
fn hint(url: Option<&String>) -> Option<RelayTarget> {
    url.map(|url| RelayTarget {
        url: url.clone(),
        token: None,
    })
}

/// Pairs this device with the Host named in `code_text`. `on_short_code`
/// receives the six digits the user compares with the Host's screen.
///
/// Signed in to a relay account, the phone first registers its key for this
/// Host and pairs through the Host's relay node with a ticket, as a Host
/// reachable only through that relay needs.
pub async fn pair(
    code_text: &str,
    device_label: &str,
    device_secret: [u8; 32],
    relay: RelayChoice,
    account: Option<&Account>,
    on_short_code: impl FnOnce(u32),
) -> Result<PairedHostRecord, MobileError> {
    let mut code = PairingCode::decode(code_text.trim())
        .map_err(|error| MobileError::InvalidCode(format!("{error:?}")))?;
    let key = host_key(&device_secret, &code.host);
    // The Host's name comes with pairing; connecting renames the key.
    let short_host = code.host.to_string().chars().take(8).collect::<String>();
    let label = crate::account::device_label(device_label, &short_host);
    let ticket = account_ticket(account, &device_secret, &code.host, &label, None).await?;
    let target = match &ticket {
        Ok(Some(ticket)) => {
            code.relay_url = Some(ticket.relay_url.clone());
            Some(ticket.target())
        }
        _ => hint(code.relay_url.as_ref()),
    };
    // The short code shows once the Host has answered twice: until then the
    // link may be one a proxy stalls, and the relay alone gets a second try.
    let on_short_code = Mutex::new(Some(on_short_code));
    let shown = std::sync::atomic::AtomicBool::new(false);
    let show = |short_code| {
        shown.store(true, std::sync::atomic::Ordering::SeqCst);
        if let Some(show) = on_short_code.lock().take() {
            show(short_code);
        }
    };
    let grace = (relay == RelayChoice::FromHost && target.is_some()).then_some(DIRECT_GRACE);
    let endpoint = bind(key, relay.setup(target.clone())?).await?;
    let attempt = yttt_client_core::p2p::pair(&endpoint, &code, device_label, &show);
    let result = match within_grace(attempt, grace, || {
        shown.load(std::sync::atomic::Ordering::SeqCst)
    })
    .await
    {
        Some(result) => {
            endpoint.close().await;
            result
        }
        None => {
            endpoint.close().await;
            tracing::warn!("pairing heard nothing from the Host directly; trying the relay alone");
            let endpoint = bind(key, RelayChoice::OnlyFromHost.setup(target)?).await?;
            let result = yttt_client_core::p2p::pair(&endpoint, &code, device_label, &show).await;
            endpoint.close().await;
            result
        }
    };
    match result {
        Ok(host) => Ok(PairedHostRecord { host }),
        Err(error) => {
            // A key that never paired should not hold one of the plan's slots.
            if let (Some(account), Ok(Some(_))) = (account, &ticket) {
                let _ = account.release(&device_secret, &code.host).await;
            }
            Err(MobileError::Pairing(match ticket {
                Err(ticket) => format!("{error} (relay account: {ticket})"),
                Ok(_) => error.to_string(),
            }))
        }
    }
}

/// `work`, or `None` when `grace` passed first and `started` still says no:
/// the link stalled before the Host answered. Once started it runs to the end.
async fn within_grace<T>(
    work: impl std::future::Future<Output = T>,
    grace: Option<Duration>,
    started: impl Fn() -> bool,
) -> Option<T> {
    let Some(grace) = grace else {
        return Some(work.await);
    };
    tokio::pin!(work);
    tokio::select! {
        result = &mut work => Some(result),
        () = tokio::time::sleep(grace) => {
            if started() { Some(work.await) } else { None }
        }
    }
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
    /// The terminals the screens want, attached or not yet: one asked for
    /// while the link was down is attached once it is back.
    wanted: Mutex<std::collections::HashSet<TerminalSessionId>>,
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
    /// The task calling back with link states, replaced by each `watch_link`.
    link_watch: Mutex<Option<tokio::task::JoinHandle<()>>>,
    /// Renews the relay ticket while the session lasts.
    ticket_renewal: Option<tokio::task::JoinHandle<()>>,
    relay_source: RelaySource,
}

/// Whether the session reaches the Host, for the banner the apps show.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum LinkState {
    Connecting,
    Ready,
    /// Down, and trying again; `attempt` counts from 1.
    Reconnecting {
        attempt: u32,
        reason: String,
    },
    /// Given up: the Host refused this device or is not the one paired.
    /// Only a new session can help.
    Lost {
        reason: String,
    },
}

impl From<&ConnectionState> for LinkState {
    fn from(state: &ConnectionState) -> Self {
        match state {
            ConnectionState::Ready { .. } => Self::Ready,
            ConnectionState::Connecting => Self::Connecting,
            ConnectionState::Reconnecting { attempt, message } => Self::Reconnecting {
                attempt: *attempt,
                reason: message.clone(),
            },
            ConnectionState::HostLost { message } => Self::Lost {
                reason: message.clone(),
            },
            ConnectionState::Disconnected => Self::Lost {
                reason: "disconnected".into(),
            },
        }
    }
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
    /// Connects to a paired Host over P2P. Signed in to a relay account, it
    /// goes through the Host's relay node with a ticket it keeps renewed;
    /// `device_label` names this phone on the account's console.
    pub async fn connect(
        record: &PairedHostRecord,
        device_secret: [u8; 32],
        relay: RelayChoice,
        account: Option<&Account>,
        device_label: &str,
    ) -> Result<Self, MobileError> {
        let mut host = record.host.clone();
        let key = host_key(&device_secret, &host.host);
        let label = crate::account::device_label(device_label, &host.host_label);
        let ticket = account_ticket(
            account,
            &device_secret,
            &host.host,
            &label,
            Some(TICKET_WAIT),
        )
        .await?
        .ok()
        .flatten();
        let target = match &ticket {
            Some(ticket) => {
                host.relay_url = Some(ticket.relay_url.clone());
                Some(ticket.target())
            }
            None => hint(host.relay_url.as_ref()),
        };
        let relay_source = match (&ticket, &target, relay) {
            (_, _, RelayChoice::Off) | (_, None, _) => RelaySource::None,
            (Some(_), _, _) => RelaySource::Account,
            (None, Some(_), _) => RelaySource::PairingHint,
        };
        let grace = (relay == RelayChoice::FromHost && target.is_some()).then_some(DIRECT_GRACE);
        let mut endpoint = bind(key, relay.setup(target.clone())?).await?;
        let mut connector = Arc::new(host.connector(endpoint.clone()));
        let first = Self::connect_with(connector.clone(), record, host.credential(key));
        let session = match within_grace(first, grace, || false).await {
            Some(session) => session,
            None => {
                // As in pairing: a proxy can stall the direct path; the
                // relay alone carries this session instead.
                endpoint.close().await;
                tracing::warn!(
                    "the Host did not answer directly; connecting through the relay alone"
                );
                endpoint = bind(key, RelayChoice::OnlyFromHost.setup(target)?).await?;
                connector = Arc::new(host.connector(endpoint.clone()));
                Self::connect_with(connector.clone(), record, host.credential(key)).await
            }
        };
        match session {
            Ok(mut session) => {
                if let (Some(account), Some(ticket), true) =
                    (account, ticket, relay_source == RelaySource::Account)
                {
                    session.ticket_renewal =
                        Some(tokio::spawn(yttt_client_core::control::keep_ticket(
                            account.session.clone(),
                            key,
                            yttt_client_core::control::DeviceKind::Client { host: host.host.0 },
                            endpoint.clone(),
                            ticket,
                            |_| {},
                            |error| tracing::warn!(%error, "relay ticket renewal failed"),
                        )));
                }
                session.relay_source = relay_source;
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
            wanted: Mutex::new(std::collections::HashSet::new()),
            times: Mutex::new(DrainTimes::default()),
            workspace: Mutex::new(None),
            opened_files: Mutex::new(HashMap::new()),
        });
        let events = tokio::spawn(follow_events(
            client.subscribe_events(),
            Arc::downgrade(&client),
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
            link_watch: Mutex::new(None),
            ticket_renewal: None,
            relay_source: RelaySource::None,
        })
    }

    pub fn client(&self) -> &ClientCore {
        &self.client
    }

    pub fn is_ready(&self) -> bool {
        matches!(self.client.state(), ConnectionState::Ready { .. })
    }

    /// Where the relay this session uses came from.
    pub fn relay_source(&self) -> RelaySource {
        self.relay_source
    }

    /// Opens the workspace the Host's desktop shows, without changing any
    /// shared view.
    pub async fn open_active_workspace(&self) -> Result<Option<WorkspaceSummary>, MobileError> {
        open_active_workspace(&self.client, &self.shared).await
    }

    /// Where the link stands now.
    pub fn link_state(&self) -> LinkState {
        LinkState::from(&self.client.state())
    }

    /// Calls `callback` with the link state now and after each change;
    /// replaces an earlier callback and ends with `close`.
    pub fn watch_link(&self, callback: impl Fn(LinkState) + Send + Sync + 'static) {
        let mut states = self.client.subscribe_state();
        let task = tokio::spawn(async move {
            let mut last = None;
            loop {
                let state = LinkState::from(&*states.borrow_and_update());
                if last.as_ref() != Some(&state) {
                    callback(state.clone());
                    last = Some(state);
                }
                if states.changed().await.is_err() {
                    return;
                }
            }
        });
        if let Some(previous) = self.link_watch.lock().replace(task) {
            previous.abort();
        }
    }

    /// Tries to reconnect at once rather than after the backoff.
    pub fn retry_now(&self) {
        self.client.retry_now();
    }
}

/// Opens the workspace the Host's desktop shows, without changing any shared
/// view, and keeps it for the overview.
async fn open_active_workspace(
    client: &ClientCore,
    shared: &Shared,
) -> Result<Option<WorkspaceSummary>, MobileError> {
    let request = |request| async move {
        client
            .request(request)
            .await
            .map_err(|error| MobileError::Host(error.to_string()))
    };
    {
        let Response::Workspace(WorkspaceResponse::Workspaces(workspaces)) =
            request(Request::Workspace(WorkspaceRequest::List)).await?
        else {
            return Err(MobileError::Host(
                "unexpected workspace list response".into(),
            ));
        };
        let Some(active) = workspaces.into_iter().find(|workspace| workspace.active) else {
            return Ok(None);
        };
        let Response::Workspace(WorkspaceResponse::Opened { layout, .. }) =
            request(Request::Workspace(WorkspaceRequest::Open {
                workspace_id: active.workspace_id.clone(),
            }))
            .await?
        else {
            return Err(MobileError::Host(
                "unexpected workspace open response".into(),
            ));
        };
        *shared.workspace.lock() = Some((active.workspace_id.clone(), layout));
        Ok(Some(active))
    }
}

impl MobileSession {
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
    /// Observes a terminal. Asked while the link is down, it is attached
    /// once the link is back rather than failing.
    pub async fn attach(&self, session_id: &str) -> Result<(), MobileError> {
        let session_id = TerminalSessionId::new(session_id);
        self.shared.wanted.lock().insert(session_id.clone());
        if !self.is_ready() {
            return Ok(());
        }
        match attach_now(&self.client, &self.shared, &session_id).await {
            // The link went down during the request: the next Ready attaches.
            Err(_) if !self.is_ready() => Ok(()),
            Err(error) => {
                self.shared.wanted.lock().remove(&session_id);
                Err(error)
            }
            Ok(()) => Ok(()),
        }
    }

    pub async fn detach(&self, session_id: &str) -> Result<(), MobileError> {
        let session_id = TerminalSessionId::new(session_id);
        self.shared.wanted.lock().remove(&session_id);
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
        if let Some(renewal) = &self.ticket_renewal {
            renewal.abort();
        }
        if let Some(watch) = self.overview_watch.lock().take() {
            watch.abort();
        }
        if let Some(watch) = self.link_watch.lock().take() {
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
        client.resource_catalog().map_or_else(Vec::new, |catalog| {
            catalog
                .terminals
                .iter()
                .map(|terminal| terminal.session_id.to_string())
                .collect()
        }),
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

/// Attaches one terminal as an Observer, keeping any lease the Host has
/// for this device.
async fn attach_now(
    client: &ClientCore,
    shared: &Shared,
    session_id: &TerminalSessionId,
) -> Result<(), MobileError> {
    let Response::TerminalAttached { lease, checkpoint } = client
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
        .await
        .map_err(|error| MobileError::Host(error.to_string()))?
    else {
        return Err(MobileError::Host("unexpected attach response".into()));
    };
    let interactive = lease.mode == TerminalLeaseMode::Interactive;
    let mut encoder = VtEncoder::new();
    encoder.set_interactive(interactive);
    shared.attached.lock().insert(
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
    let visible: Vec<_> = shared.attached.lock().keys().cloned().collect();
    client.set_visible_terminals(VISIBILITY_SCOPE, visible);
    Ok(())
}

/// After a reconnect: the terminals the screens want and the Host still
/// runs are attached, and the workspace is opened if this session has none,
/// as after a Host restart or a first open that failed.
async fn recover(client: Arc<ClientCore>, shared: Arc<Shared>) {
    let missing: Vec<_> = {
        let attached = shared.attached.lock();
        shared
            .wanted
            .lock()
            .iter()
            .filter(|id| !attached.contains_key(*id))
            .cloned()
            .collect()
    };
    for session_id in missing {
        if let Err(error) = attach_now(&client, &shared, &session_id).await {
            if !matches!(client.state(), ConnectionState::Ready { .. }) {
                // Down again: the next Ready tries again.
                return;
            }
            tracing::debug!(%error, session = %session_id, "a wanted terminal is gone");
            shared.wanted.lock().remove(&session_id);
        }
    }
    if shared.workspace.lock().is_none()
        && let Err(error) = open_active_workspace(&client, &shared).await
    {
        tracing::debug!(%error, "reopening the workspace failed");
    }
}

/// Tracks lease grants and losses so input carries the right epoch, and the
/// encoder syncs mouse modes only while this device is in control; recovers
/// after each reconnect.
async fn follow_events(
    mut events: tokio::sync::broadcast::Receiver<ClientEvent>,
    client: std::sync::Weak<ClientCore>,
    shared: Arc<Shared>,
    me: ClientInstanceId,
) {
    let mut host_epoch = client.upgrade().and_then(|client| client.host_epoch());
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
            ClientEvent::Connection(ConnectionState::Ready {
                host_epoch: epoch, ..
            }) => {
                // The same Host keeps this device's leases through a short
                // drop; a restarted one has forgotten them, its terminals and
                // the workspace this session opened.
                let same_host = host_epoch == Some(epoch);
                host_epoch = Some(epoch);
                for terminal in shared.attached.lock().values_mut() {
                    if !same_host {
                        terminal.lease_epoch = 0;
                    }
                    terminal.encoder.reset();
                    terminal.encoder.set_interactive(terminal.lease_epoch != 0);
                    terminal.encoded = None;
                    terminal.dirty = true;
                }
                if !same_host {
                    *shared.workspace.lock() = None;
                }
                if let Some(client) = client.upgrade() {
                    tokio::spawn(recover(client, shared.clone()));
                }
            }
            ClientEvent::TerminalUnavailable(session_id) => {
                shared.attached.lock().remove(&session_id);
                shared.wanted.lock().remove(&session_id);
            }
            _ => {}
        }
    }
}
