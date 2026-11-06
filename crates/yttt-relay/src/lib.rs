//! A paid iroh relay node.
//!
//! The node forwards end-to-end encrypted QUIC datagrams between EndpointIds.
//! It admits a connection only with a ticket the control plane signed, paces
//! each account with one bucket shared by all its connections, reports what
//! each account sent, and follows the control plane's answer: the account's
//! current rate, devices removed from it, or suspension. It holds
//! no key that could open, forge or redirect traffic between devices.

mod config;
pub mod enroll;
mod framing;
mod http;
mod node;
mod report;

use std::{net::SocketAddr, sync::Arc, time::Duration};

use tokio::{net::TcpListener, task::JoinSet};
use tokio_rustls::TlsAcceptor;
use tokio_util::sync::CancellationToken;
use yttt_relay_ticket::{TicketVerifier, api::AccountStanding};

pub use config::{AcmeSettings, ControlConfig, NodeConfig, OpenConfig, TlsFiles};
pub use node::AccountSnapshot;

const PRUNE_TICK: Duration = Duration::from_secs(10);

#[derive(Debug, thiserror::Error)]
pub enum StartError {
    #[error("ticket key {0:?} is not 32 bytes of hex")]
    TicketKey(String),
    #[error("a node needs at least one ticket key, or [open] for self-hosting")]
    NoTicketKeys,
    #[error("[open] and [control] exclude each other: an open node bills no one")]
    OpenWithControl,
    #[error("QUIC address discovery needs TLS")]
    QuicWithoutTls,
    #[error("set either [tls] or [acme], not both")]
    TlsAndAcme,
    #[error("could not load TLS files: {0}")]
    Tls(String),
    #[error("could not bind {addr}: {source}")]
    Bind {
        addr: SocketAddr,
        source: std::io::Error,
    },
    #[error("could not start QUIC address discovery: {0}")]
    Quic(String),
}

/// A running relay node. Dropping it does not stop it; call [`Self::shutdown`].
pub struct RelayNode {
    node: Arc<node::Node>,
    addr: SocketAddr,
    quic_addr: Option<SocketAddr>,
    cancel: CancellationToken,
    tasks: JoinSet<()>,
    quic: Option<iroh_relay::server::Server>,
}

impl RelayNode {
    pub async fn spawn(config: NodeConfig) -> Result<Self, StartError> {
        let keys = config
            .ticket_keys
            .iter()
            .map(|key| {
                yttt_relay_ticket::parse_key(key).ok_or_else(|| StartError::TicketKey(key.clone()))
            })
            .collect::<Result<Vec<_>, _>>()?;
        if keys.is_empty() && config.open.is_none() {
            return Err(StartError::NoTicketKeys);
        }
        if config.open.is_some() && config.control.is_some() {
            return Err(StartError::OpenWithControl);
        }
        let verifier = if keys.is_empty() {
            None
        } else {
            Some(TicketVerifier::new(&keys).map_err(|_| StartError::TicketKey(String::new()))?)
        };
        let open = config
            .open
            .map(|open| node::open_rate(open.rate_bps, open.burst_bytes));
        let node = Arc::new(node::Node::new(config.node_id.clone(), verifier, open));
        let cancel = CancellationToken::new();
        let mut tasks = JoinSet::new();
        let (tls, server_config) = match (&config.tls, &config.acme) {
            (Some(_), Some(_)) => return Err(StartError::TlsAndAcme),
            (Some(files), None) => {
                let server_config = load_tls(files)?;
                (
                    Some(http::Tls::Manual(TlsAcceptor::from(server_config.clone()))),
                    Some(server_config),
                )
            }
            (None, Some(acme)) => {
                let (acceptor, server_config) = start_acme(acme, &mut tasks, cancel.clone());
                (
                    Some(http::Tls::Acme(acceptor, server_config.clone())),
                    Some(server_config),
                )
            }
            (None, None) => (None, None),
        };

        let listener = bind(config.listen).await?;
        let addr = listener.local_addr().map_err(|source| StartError::Bind {
            addr: config.listen,
            source,
        })?;
        tasks.spawn(http::serve(
            listener,
            tls.clone(),
            node.clone(),
            cancel.clone(),
        ));
        if let (Some(_), Some(captive)) = (&server_config, config.captive_portal_listen) {
            tasks.spawn(http::serve_captive_portal(
                bind(captive).await?,
                cancel.clone(),
            ));
        }

        let quic = match config.quic_listen {
            None => None,
            Some(quic_addr) => {
                let tls = server_config.as_ref().ok_or(StartError::QuicWithoutTls)?;
                let mut quic = iroh_relay::server::QuicConfig::new(quic_addr);
                // With ACME this shares the certificate resolver, so QUIC
                // picks up each new certificate too.
                quic.server_config = Some((**tls).clone());
                let mut server = iroh_relay::server::ServerConfig::default();
                server.quic = Some(quic);
                Some(
                    iroh_relay::server::Server::spawn(server)
                        .await
                        .map_err(|error| StartError::Quic(error.to_string()))?,
                )
            }
        };
        let quic_addr = quic.as_ref().and_then(|server| server.quic_addr());

        tasks.spawn({
            let node = node.clone();
            let cancel = cancel.clone();
            async move {
                let mut tick = tokio::time::interval(PRUNE_TICK);
                loop {
                    tokio::select! {
                        () = cancel.cancelled() => return,
                        _ = tick.tick() => node.prune(node::now_unix()),
                    }
                }
            }
        });
        if let Some(control) = config.control.clone() {
            tasks.spawn(report::run(
                node.clone(),
                config.node_id.clone(),
                control,
                cancel.clone(),
            ));
        }

        Ok(Self {
            node,
            addr,
            quic_addr,
            cancel,
            tasks,
            quic,
        })
    }

    /// The address serving `/relay`.
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    pub fn quic_addr(&self) -> Option<SocketAddr> {
        self.quic_addr
    }

    /// The account's connections and bytes on this node, if it has any.
    pub fn account(&self, account_id: u64) -> Option<AccountSnapshot> {
        self.node.snapshot(account_id)
    }

    /// Applies standings directly, as the control plane's replies do.
    pub fn apply_standings(&self, standings: &[AccountStanding]) {
        self.node.apply(standings);
    }

    pub async fn shutdown(mut self) {
        self.cancel.cancel();
        self.node.clients.shutdown().await;
        if let Some(quic) = self.quic.take() {
            let _ = quic.shutdown().await;
        }
        while self.tasks.join_next().await.is_some() {}
    }
}

async fn bind(addr: SocketAddr) -> Result<TcpListener, StartError> {
    TcpListener::bind(addr)
        .await
        .map_err(|source| StartError::Bind { addr, source })
}

/// Starts requesting and renewing certificates; connections are served with
/// whatever certificate the resolver holds, and fail TLS until the first one
/// arrives.
fn start_acme(
    settings: &AcmeSettings,
    tasks: &mut JoinSet<()>,
    cancel: CancellationToken,
) -> (tokio_rustls_acme::AcmeAcceptor, Arc<rustls::ServerConfig>) {
    use futures_util::StreamExt as _;
    // tokio-rustls-acme builds its own configs from the process default.
    let _ = rustls::crypto::ring::default_provider().install_default();
    let mut state = tokio_rustls_acme::AcmeConfig::new(&settings.domains)
        .contact(settings.contact.iter().map(|contact| {
            if contact.starts_with("mailto:") {
                contact.clone()
            } else {
                format!("mailto:{contact}")
            }
        }))
        .directory_lets_encrypt(!settings.staging)
        .cache_option(Some(tokio_rustls_acme::caches::DirCache::new(
            settings.cache_dir.clone(),
        )))
        .state();
    let acceptor = state.acceptor();
    let server_config = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .expect("ring supports the default TLS versions")
    .with_no_client_auth()
    .with_cert_resolver(state.resolver());
    tasks.spawn(async move {
        loop {
            tokio::select! {
                () = cancel.cancelled() => return,
                event = state.next() => match event {
                    Some(Ok(event)) => tracing::info!(?event, "certificate"),
                    Some(Err(error)) => tracing::warn!(?error, "certificate request failed; retrying"),
                    None => return,
                },
            }
        }
    });
    (acceptor, Arc::new(server_config))
}

fn load_tls(files: &TlsFiles) -> Result<Arc<rustls::ServerConfig>, StartError> {
    use rustls::pki_types::{CertificateDer, PrivateKeyDer, pem::PemObject as _};
    let chain = CertificateDer::pem_file_iter(&files.cert_chain)
        .and_then(|certs| certs.collect::<Result<Vec<_>, _>>())
        .map_err(|error| StartError::Tls(format!("{}: {error}", files.cert_chain.display())))?;
    let key = PrivateKeyDer::from_pem_file(&files.private_key)
        .map_err(|error| StartError::Tls(format!("{}: {error}", files.private_key.display())))?;
    let config = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_protocol_versions(&[&rustls::version::TLS13, &rustls::version::TLS12])
    .map_err(|error| StartError::Tls(error.to_string()))?
    .with_no_client_auth()
    .with_single_cert(chain, key)
    .map_err(|error| StartError::Tls(error.to_string()))?;
    Ok(Arc::new(config))
}
