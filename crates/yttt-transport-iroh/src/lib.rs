#![forbid(unsafe_code)]
//! P2P transport over iroh: QUIC with hole punching and an optional relay.
//!
//! A peer is named by its Ed25519 [`EndpointKey`], which iroh's TLS handshake
//! authenticates. A [`PeerGate`] decides which peers and ALPNs an endpoint serves;
//! it runs once after the handshake and again before every accepted stream, so a
//! revoked peer cannot open new streams on a connection that is still alive.

mod path_report;

use std::{collections::HashMap, fmt, net::SocketAddr, sync::Arc, time::Duration};

use iroh::{
    Endpoint, EndpointAddr, PublicKey, RelayConfig, RelayMap, RelayMode, RelayUrl, SecretKey,
    TransportAddr,
    endpoint::{
        AfterHandshakeOutcome, Connection, EndpointHooks, QuicTransportConfig, VarInt,
        WeakConnectionHandle, presets,
    },
};
use parking_lot::Mutex;
use yttt_protocol::EndpointKey;
use yttt_transport::{BoxFuture, LinkProfile, TransportConnector, TransportError, TransportStream};

pub use path_report::{AddressKind, PathReport, PathSummary, address_kind};

/// QUIC close code for a peer the gate refused.
const CLOSE_REFUSED: u32 = 1;
/// QUIC close code for a peer the Host revoked or disconnected.
const CLOSE_REVOKED: u32 = 2;
/// Keeps NAT bindings and relay paths warm between Session heartbeats.
const KEEP_ALIVE: Duration = Duration::from_secs(5);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
/// Bytes a sender may have outstanding on one stream beyond what the peer read.
/// The Session puts heartbeats and keystrokes ahead of bulk messages only among
/// bytes it still holds; QUIC's 1.25 MB default left a heartbeat 5 s behind on
/// a 250 KB/s relay and over a minute behind on a throttled one. 256 KB still
/// carries about 5 MB/s at a 50 ms round trip.
const STREAM_WINDOW: u32 = 256 * 1024;
/// Bytes a sender may have outstanding across all of a connection's streams.
const SEND_WINDOW: u64 = 2 * STREAM_WINDOW as u64;
/// What a receiver lets its peer have outstanding over a relay. A relay
/// queues what exceeds its rate instead of dropping it, so QUIC fills
/// whatever window it is given: 256 KB through a relay carrying 40 KB/s held
/// every heartbeat and reply about 6 s behind. 64 KB keeps that near 1.5 s
/// and still carries about 200 KB/s at a 300 ms round trip.
const RELAY_RECEIVE_WINDOW: u32 = 64 * 1024;

/// The connection receive window for a connection whose selected path is
/// (or is not) a relay.
fn receive_window(relayed: bool) -> VarInt {
    if relayed {
        VarInt::from_u32(RELAY_RECEIVE_WINDOW)
    } else {
        VarInt::from_u64(SEND_WINDOW).expect("the send window fits a VarInt")
    }
}

/// How often a direct path is probed, and how long it may stay silent before
/// it is given up for another, such as the relay. iroh's 5 s and 15 s left a
/// session whose direct path broke frozen for 16 s before it moved to the
/// relay; three probes missed in a row is enough to say the path is gone.
const PATH_KEEP_ALIVE: Duration = Duration::from_secs(2);
const PATH_IDLE_TIMEOUT: Duration = Duration::from_secs(7);

/// How long iroh's own resolver may take for one name.
const DNS_TIMEOUT: Duration = Duration::from_secs(5);

/// Resolves names with iroh's resolver and the system's at once, taking the
/// first answer. iroh asks the configured nameservers itself over UDP, which
/// a network or firewall that lets only the system resolver through blocks:
/// the relay's name then never resolved and the relay was out of reach,
/// though every other program could still look it up. TXT records, which
/// the system resolver cannot look up, go to iroh's alone.
#[derive(Debug)]
struct SystemFallbackResolver {
    iroh: iroh::dns::DnsResolver,
}

impl SystemFallbackResolver {
    fn resolver() -> iroh::dns::DnsResolver {
        iroh::dns::DnsResolver::custom(Self {
            iroh: iroh::dns::DnsResolver::new(),
        })
    }

    /// The first of iroh's and the system's answers for `host`; iroh's error
    /// when neither has one.
    async fn first_answer<A, I>(
        iroh: impl Future<Output = Result<I, iroh::dns::DnsError>>,
        host: String,
        pick: fn(std::net::IpAddr) -> Option<A>,
    ) -> Result<Vec<A>, iroh::dns::DnsError>
    where
        I: Iterator<Item = std::net::IpAddr>,
    {
        let system = async move {
            let addresses = tokio::net::lookup_host((host.as_str(), 0)).await.ok()?;
            let picked = addresses
                .filter_map(|address| pick(address.ip()))
                .collect::<Vec<_>>();
            (!picked.is_empty()).then_some(picked)
        };
        tokio::pin!(iroh, system);
        let (mut iroh_done, mut system_done, mut iroh_error) = (false, false, None);
        while !(iroh_done && system_done) {
            tokio::select! {
                answer = &mut iroh, if !iroh_done => {
                    iroh_done = true;
                    match answer {
                        Ok(addresses) => {
                            let picked = addresses.filter_map(pick).collect::<Vec<_>>();
                            if !picked.is_empty() {
                                return Ok(picked);
                            }
                        }
                        Err(error) => iroh_error = Some(error),
                    }
                }
                picked = &mut system, if !system_done => {
                    system_done = true;
                    if let Some(picked) = picked {
                        return Ok(picked);
                    }
                }
            }
        }
        iroh_error.map_or_else(|| Ok(Vec::new()), Err)
    }
}

impl iroh::dns::Resolver for SystemFallbackResolver {
    fn lookup_ipv4(
        &self,
        host: String,
    ) -> n0_future::boxed::BoxFuture<
        Result<iroh::dns::BoxIter<std::net::Ipv4Addr>, iroh::dns::DnsError>,
    > {
        let iroh = self.iroh.clone();
        Box::pin(async move {
            let answer = Self::first_answer(
                async { iroh.lookup_ipv4(host.clone(), DNS_TIMEOUT).await },
                host.clone(),
                |address| match address {
                    std::net::IpAddr::V4(address) => Some(address),
                    std::net::IpAddr::V6(_) => None,
                },
            )
            .await?;
            Ok(Box::new(answer.into_iter()) as iroh::dns::BoxIter<_>)
        })
    }

    fn lookup_ipv6(
        &self,
        host: String,
    ) -> n0_future::boxed::BoxFuture<
        Result<iroh::dns::BoxIter<std::net::Ipv6Addr>, iroh::dns::DnsError>,
    > {
        let iroh = self.iroh.clone();
        Box::pin(async move {
            let answer = Self::first_answer(
                async { iroh.lookup_ipv6(host.clone(), DNS_TIMEOUT).await },
                host.clone(),
                |address| match address {
                    std::net::IpAddr::V6(address) => Some(address),
                    std::net::IpAddr::V4(_) => None,
                },
            )
            .await?;
            Ok(Box::new(answer.into_iter()) as iroh::dns::BoxIter<_>)
        })
    }

    fn lookup_txt(
        &self,
        host: String,
    ) -> n0_future::boxed::BoxFuture<
        Result<iroh::dns::BoxIter<iroh::dns::TxtRecordData>, iroh::dns::DnsError>,
    > {
        let iroh = self.iroh.clone();
        Box::pin(async move {
            let records = iroh.lookup_txt(host, DNS_TIMEOUT).await?;
            Ok(Box::new(records.collect::<Vec<_>>().into_iter()) as iroh::dns::BoxIter<_>)
        })
    }

    fn clear_cache(&self) {
        self.iroh.clear_cache();
    }

    fn reset(&self) -> Box<dyn iroh::dns::Resolver> {
        self.iroh.reset();
        Box::new(Self {
            iroh: self.iroh.clone(),
        })
    }
}

/// Keeps `connection`'s receive window to its selected path until it closes.
fn follow_selected_path(connection: Connection) {
    use n0_future::StreamExt as _;
    tokio::spawn(async move {
        let mut paths = connection.paths_stream();
        let mut relayed = None;
        loop {
            let selected_relay = tokio::select! {
                paths = paths.next() => match paths {
                    Some(paths) => paths
                        .iter()
                        .find(|path| path.is_selected())
                        .map(|path| path.is_relay()),
                    None => break,
                },
                _ = connection.closed() => break,
            };
            if let Some(relay) = selected_relay
                && relayed != Some(relay)
            {
                relayed = Some(relay);
                connection.set_receive_window(receive_window(relay));
                tracing::debug!(
                    relay,
                    window = receive_window(relay).into_inner(),
                    "receive window follows the path"
                );
            }
        }
    });
}

/// Decides which peers an endpoint serves. Must answer quickly; it runs inside
/// iroh's handshake processing.
pub trait PeerGate: Send + Sync + 'static {
    fn admit(&self, peer: EndpointKey, alpn: &[u8]) -> bool;
}

/// A gate that serves every peer, for endpoints that only dial out.
pub struct OpenGate;

impl PeerGate for OpenGate {
    fn admit(&self, _peer: EndpointKey, _alpn: &[u8]) -> bool {
        true
    }
}

/// The relay an endpoint keeps its home connection to.
#[derive(Clone, PartialEq, Eq)]
pub struct RelayTarget {
    pub url: String,
    /// Bearer token the relay requires, e.g. a ticket from the control plane.
    pub token: Option<String>,
}

impl fmt::Debug for RelayTarget {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RelayTarget")
            .field("url", &self.url)
            .field("token", &self.token.as_ref().map(|_| "[redacted]"))
            .finish()
    }
}

pub struct EndpointConfig {
    pub secret: [u8; 32],
    pub relay: RelaySetup,
    /// ALPNs this endpoint accepts; an endpoint that only dials passes none.
    pub alpns: Vec<Vec<u8>>,
    pub gate: Arc<dyn PeerGate>,
    pub direct: DirectPaths,
}

/// Whether an endpoint keeps a home relay.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum RelaySetup {
    /// Direct paths only; a relay can never be added.
    #[default]
    Off,
    /// No relay yet, but one may come with [`IrohEndpoint::set_relay`].
    Pending,
    Use(RelayTarget),
}

impl From<Option<RelayTarget>> for RelaySetup {
    fn from(relay: Option<RelayTarget>) -> Self {
        relay.map_or(Self::Off, Self::Use)
    }
}

/// Whether, and where, an endpoint takes direct UDP paths.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DirectPaths {
    /// An ephemeral port on all interfaces.
    #[default]
    Any,
    Bind(SocketAddr),
    /// None at all: every packet goes through the relay, and the endpoint
    /// never reveals its addresses to peers.
    Off,
}

/// How to reach a peer: its key plus hints, all of which may be stale.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PeerAddr {
    pub key: EndpointKey,
    pub relay_url: Option<String>,
    pub direct_addrs: Vec<SocketAddr>,
}

/// Which path a connection currently sends application data on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathKind {
    Direct,
    Relay,
    Unknown,
}

/// Connections by peer, so that revoking a peer closes all of them.
#[derive(Default)]
struct PeerRegistry {
    peers: Mutex<HashMap<EndpointKey, Vec<WeakConnectionHandle>>>,
}

impl PeerRegistry {
    fn record(&self, peer: EndpointKey, connection: &Connection) {
        let mut peers = self.peers.lock();
        let handles = peers.entry(peer).or_default();
        handles.retain(|handle| handle.upgrade().is_some());
        handles.push(connection.weak_handle());
    }

    fn close(&self, peer: EndpointKey) -> usize {
        let handles = self.peers.lock().remove(&peer).unwrap_or_default();
        let mut closed = 0;
        for connection in handles.iter().filter_map(WeakConnectionHandle::upgrade) {
            connection.close(VarInt::from_u32(CLOSE_REVOKED), b"revoked");
            closed += 1;
        }
        closed
    }
}

struct GateHook {
    gate: Arc<dyn PeerGate>,
    registry: Arc<PeerRegistry>,
}

impl fmt::Debug for GateHook {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("GateHook")
    }
}

impl EndpointHooks for GateHook {
    async fn after_handshake<'a>(&'a self, connection: &'a Connection) -> AfterHandshakeOutcome {
        let peer = key_of(connection.remote_id());
        // Outgoing connections reach peers this endpoint chose to dial.
        if connection.side().is_server() && !self.gate.admit(peer, connection.alpn()) {
            tracing::debug!(%peer, alpn = %String::from_utf8_lossy(connection.alpn()), "the gate refused a connection");
            return AfterHandshakeOutcome::Reject {
                error_code: VarInt::from_u32(CLOSE_REFUSED),
                reason: b"refused".to_vec(),
            };
        }
        self.registry.record(peer, connection);
        AfterHandshakeOutcome::accept()
    }
}

#[derive(Clone)]
pub struct IrohEndpoint {
    endpoint: Endpoint,
    gate: Arc<dyn PeerGate>,
    registry: Arc<PeerRegistry>,
    relay: Arc<tokio::sync::Mutex<Option<RelayUrl>>>,
}

impl IrohEndpoint {
    pub async fn bind(config: EndpointConfig) -> Result<Self, TransportError> {
        let registry = Arc::new(PeerRegistry::default());
        let (relay_mode, relay_url) = match &config.relay {
            RelaySetup::Off => (RelayMode::Disabled, None),
            RelaySetup::Pending => (RelayMode::Custom(RelayMap::empty()), None),
            RelaySetup::Use(target) => {
                let relay = relay_config(target)?;
                let url = relay.url.clone();
                (RelayMode::Custom(RelayMap::from(relay)), Some(url))
            }
        };
        let transport = QuicTransportConfig::builder()
            .keep_alive_interval(KEEP_ALIVE)
            .default_path_keep_alive_interval(PATH_KEEP_ALIVE)
            .default_path_max_idle_timeout(PATH_IDLE_TIMEOUT)
            .stream_receive_window(VarInt::from_u32(STREAM_WINDOW))
            .receive_window(receive_window(false))
            .send_window(SEND_WINDOW)
            .build();
        let mut builder = Endpoint::builder(presets::Minimal)
            .secret_key(SecretKey::from_bytes(&config.secret))
            .alpns(config.alpns)
            .relay_mode(relay_mode)
            .transport_config(transport)
            .dns_resolver(SystemFallbackResolver::resolver())
            .hooks(GateHook {
                gate: config.gate.clone(),
                registry: registry.clone(),
            });
        match config.direct {
            DirectPaths::Any => {}
            DirectPaths::Bind(address) => {
                builder = builder
                    .clear_ip_transports()
                    .bind_addr(address)
                    .map_err(other)?;
            }
            DirectPaths::Off => builder = builder.clear_ip_transports(),
        }
        let endpoint = builder.bind().await.map_err(other)?;
        Ok(Self {
            endpoint,
            gate: config.gate,
            registry,
            relay: Arc::new(tokio::sync::Mutex::new(relay_url)),
        })
    }

    pub fn id(&self) -> EndpointKey {
        key_of(self.endpoint.id())
    }

    /// Local addresses a peer on the same network may dial directly.
    pub fn direct_addrs(&self) -> Vec<SocketAddr> {
        let mut addresses: Vec<_> = self.endpoint.addr().ip_addrs().copied().collect();
        if addresses.is_empty() {
            addresses = self.endpoint.bound_sockets();
        }
        addresses
    }

    /// What this endpoint advertises and which paths `connection` holds.
    pub fn path_report(&self, connection: &PeerConnection) -> PathReport {
        PathReport {
            advertised: self.direct_addrs(),
            paths: connection
                .connection
                .paths()
                .iter()
                .map(|path| PathSummary {
                    remote: match path.remote_addr() {
                        TransportAddr::Relay(url) => url.to_string(),
                        TransportAddr::Ip(address) => address.to_string(),
                        _ => "custom transport".to_string(),
                    },
                    relay: path.is_relay(),
                    selected: path.is_selected(),
                    rtt: path.rtt(),
                })
                .collect(),
        }
    }

    /// Waits until the endpoint can be reached through its relay, if it has one.
    pub async fn online(&self) {
        self.endpoint.online().await;
    }

    /// The next connection the gate admitted; `None` once the endpoint closed.
    pub async fn accept(&self) -> Option<PeerConnection> {
        loop {
            let incoming = self.endpoint.accept().await?;
            // A refused or failed handshake only ends that connection.
            match incoming.await {
                Ok(connection) => return Some(self.wrap(connection)),
                Err(error) => tracing::debug!(%error, "an incoming connection failed"),
            }
        }
    }

    pub async fn connect(
        &self,
        peer: &PeerAddr,
        alpn: &[u8],
    ) -> Result<PeerConnection, TransportError> {
        let mut address = EndpointAddr::new(public_key(peer.key)?);
        if let Some(url) = &peer.relay_url {
            address = address.with_relay_url(parse_relay_url(url)?);
        }
        address = address.with_addrs(peer.direct_addrs.iter().copied().map(TransportAddr::Ip));
        let connection =
            tokio::time::timeout(CONNECT_TIMEOUT, self.endpoint.connect(address, alpn))
                .await
                .map_err(|_| TransportError::Other("P2P connection timed out".into()))?
                .map_err(other)?;
        Ok(self.wrap(connection))
    }

    /// Closes every connection to `peer`; the gate must already refuse it so that
    /// it cannot reconnect. Returns how many connections were open.
    pub fn close_peer(&self, peer: EndpointKey) -> usize {
        self.registry.close(peer)
    }

    /// Points the endpoint at `relay`, replacing the previous relay and token.
    pub async fn set_relay(&self, relay: &RelayTarget) -> Result<(), TransportError> {
        let config = relay_config(relay)?;
        let mut current = self.relay.lock().await;
        if let Some(previous) = current.as_ref().filter(|url| **url != config.url) {
            self.endpoint.remove_relay(previous).await;
        }
        *current = Some(config.url.clone());
        self.endpoint
            .insert_relay(config.url.clone(), Arc::new(config))
            .await;
        Ok(())
    }

    pub async fn close(&self) {
        self.endpoint.close().await;
    }

    fn wrap(&self, connection: Connection) -> PeerConnection {
        follow_selected_path(connection.clone());
        PeerConnection {
            peer: key_of(connection.remote_id()),
            connection,
            gate: self.gate.clone(),
        }
    }
}

pub struct PeerConnection {
    connection: Connection,
    peer: EndpointKey,
    gate: Arc<dyn PeerGate>,
}

impl PeerConnection {
    pub fn peer(&self) -> EndpointKey {
        self.peer
    }

    pub fn alpn(&self) -> &[u8] {
        self.connection.alpn()
    }

    /// The next stream the peer opened, if the gate still admits the peer. A refused
    /// peer's connection is closed and `None` returned.
    pub async fn accept_stream(&self) -> Option<TransportStream> {
        let (send, receive) = match self.connection.accept_bi().await {
            Ok(stream) => stream,
            Err(error) => {
                tracing::debug!(peer = %self.peer, %error, "the connection ended");
                return None;
            }
        };
        if !self.gate.admit(self.peer, self.connection.alpn()) {
            tracing::debug!(peer = %self.peer, "the gate refused a stream");
            self.connection
                .close(VarInt::from_u32(CLOSE_REVOKED), b"revoked");
            return None;
        }
        Some(Box::new(tokio::io::join(receive, send)))
    }

    pub async fn open_stream(&self) -> Result<TransportStream, TransportError> {
        let (send, receive) = self.connection.open_bi().await.map_err(other)?;
        Ok(Box::new(tokio::io::join(receive, send)))
    }

    pub fn path_kind(&self) -> PathKind {
        self.connection
            .paths()
            .iter()
            .find(|path| path.is_selected())
            .map_or(PathKind::Unknown, |path| {
                if path.is_relay() {
                    PathKind::Relay
                } else {
                    PathKind::Direct
                }
            })
    }

    pub fn is_closed(&self) -> bool {
        self.connection.close_reason().is_some()
    }

    pub async fn closed(&self) {
        self.connection.closed().await;
    }

    pub fn close(&self) {
        self.connection.close(VarInt::from_u32(0), b"closed");
    }
}

/// Opens one stream per `connect`, all on one QUIC connection that is redialed
/// when it closes.
pub struct IrohConnector {
    endpoint: IrohEndpoint,
    peer: PeerAddr,
    alpn: Vec<u8>,
    connection: tokio::sync::Mutex<Option<Arc<PeerConnection>>>,
}

impl IrohConnector {
    pub fn new(endpoint: IrohEndpoint, peer: PeerAddr, alpn: &[u8]) -> Self {
        Self {
            endpoint,
            peer,
            alpn: alpn.to_vec(),
            connection: tokio::sync::Mutex::new(None),
        }
    }

    /// The path of the current connection, `Unknown` before the first connect.
    pub async fn path_kind(&self) -> PathKind {
        self.connection
            .lock()
            .await
            .as_ref()
            .map_or(PathKind::Unknown, |connection| connection.path_kind())
    }

    /// Why the current connection runs where it does; `None` before the first connect.
    pub async fn path_report(&self) -> Option<PathReport> {
        let connection = self.connection.lock().await.clone()?;
        Some(self.endpoint.path_report(&connection))
    }

    async fn connection(&self) -> Result<Arc<PeerConnection>, TransportError> {
        let mut current = self.connection.lock().await;
        if let Some(connection) = current
            .as_ref()
            .filter(|connection| !connection.is_closed())
        {
            return Ok(connection.clone());
        }
        let connection = Arc::new(self.endpoint.connect(&self.peer, &self.alpn).await?);
        *current = Some(connection.clone());
        Ok(connection)
    }
}

impl TransportConnector for IrohConnector {
    fn connect(&self) -> BoxFuture<'_, Result<TransportStream, TransportError>> {
        Box::pin(async move {
            let connection = self.connection().await?;
            match connection.open_stream().await {
                Ok(stream) => Ok(stream),
                Err(_) => {
                    // The cached connection died between the check and the open.
                    self.connection.lock().await.take();
                    self.connection().await?.open_stream().await
                }
            }
        })
    }

    fn link_profile(&self) -> LinkProfile {
        LinkProfile::REMOTE
    }
}

/// The public key a secret key belongs to, as the peer will see it.
pub fn endpoint_key(secret: &[u8; 32]) -> EndpointKey {
    key_of(SecretKey::from_bytes(secret).public())
}

fn relay_config(relay: &RelayTarget) -> Result<RelayConfig, TransportError> {
    let config = RelayConfig::from(parse_relay_url(&relay.url)?);
    Ok(match &relay.token {
        Some(token) => config.with_auth_token(token.clone()),
        None => config,
    })
}

fn parse_relay_url(url: &str) -> Result<RelayUrl, TransportError> {
    url.parse()
        .map_err(|error| TransportError::Permanent(format!("invalid relay URL: {error}")))
}

fn public_key(key: EndpointKey) -> Result<PublicKey, TransportError> {
    PublicKey::from_bytes(&key.0)
        .map_err(|_| TransportError::Permanent("invalid endpoint key".into()))
}

fn key_of(key: PublicKey) -> EndpointKey {
    EndpointKey(*key.as_bytes())
}

fn other(error: impl fmt::Display) -> TransportError {
    TransportError::Other(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_relayed_connection_lets_its_peer_hold_far_less_unread() {
        let relayed = receive_window(true).into_inner();
        let direct = receive_window(false).into_inner();
        assert!(relayed < u64::from(STREAM_WINDOW));
        assert_eq!(direct, SEND_WINDOW);
        // Still a few round trips of a 250 KB/s relay at 300 ms.
        assert!(relayed >= 250_000 * 3 / 10 / 2);
    }

    /// iroh's own lookup never answering, as when its UDP queries are
    /// blocked, leaves the system's answer to resolve the name.
    #[tokio::test]
    async fn a_name_iroh_cannot_resolve_is_resolved_by_the_system() {
        let answer = tokio::time::timeout(
            Duration::from_secs(5),
            SystemFallbackResolver::first_answer(
                std::future::pending::<
                    Result<std::iter::Empty<std::net::IpAddr>, iroh::dns::DnsError>,
                >(),
                "localhost".to_string(),
                |address| match address {
                    std::net::IpAddr::V4(address) => Some(address),
                    std::net::IpAddr::V6(_) => None,
                },
            ),
        )
        .await
        .expect("the system's answer was not taken")
        .unwrap();
        assert!(
            answer.contains(&std::net::Ipv4Addr::LOCALHOST),
            "{answer:?}"
        );
    }
}
