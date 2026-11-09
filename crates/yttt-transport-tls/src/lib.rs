#![forbid(unsafe_code)]
use rustls::{
    ClientConfig, RootCertStore, ServerConfig,
    pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName},
};
use socket2::{SockRef, TcpKeepalive};
use std::{io, net::SocketAddr, sync::Arc, time::Duration};
use tokio::{
    net::{TcpListener, TcpStream},
    task::JoinSet,
    time::Instant,
};
use tokio_rustls::{TlsAcceptor, TlsConnector as RustlsConnector};
use yttt_protocol::remote_access::TLS_SERVER_NAME;
use yttt_transport::{
    BoxFuture, LinkProfile, TransportConnector, TransportError, TransportListener, TransportStream,
};

/// Resolving and opening TCP; a lossy link may retransmit a SYN several times.
const TCP_CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// The TLS handshake once TCP is open.
const TLS_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
/// How long an attempt on one resolved address runs alone before the next
/// address is tried alongside it (RFC 8305).
const CONNECT_ATTEMPT_DELAY: Duration = Duration::from_millis(250);
/// Idle time before the kernel probes a silent peer, and between probes.
/// The Session heartbeat normally keeps the link busy; this is a backstop.
const KEEPALIVE_TIME: Duration = Duration::from_secs(30);
const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(10);
/// Unsent bytes the kernel holds before it stops taking more. The Session
/// orders heartbeats and keystrokes ahead of bulk messages only among bytes
/// it still holds, so a deep socket buffer would put them seconds behind on a
/// slow link. Linux only: macOS and Windows offer no safe way to set it.
#[cfg(any(target_os = "linux", target_os = "android"))]
const NOTSENT_LOWAT: u32 = 64 * 1024;

#[derive(Clone)]
pub struct TlsConnector {
    address: String,
    connector: RustlsConnector,
}
impl TlsConnector {
    /// Trust is imported out of band. TCP address is never used as certificate identity.
    pub fn new(address: String, certificate_der: Vec<u8>) -> Result<Self, TransportError> {
        let mut roots = RootCertStore::empty();
        roots
            .add(CertificateDer::from(certificate_der))
            .map_err(tls_error)?;
        let mut config = ClientConfig::builder_with_provider(Arc::new(
            rustls::crypto::aws_lc_rs::default_provider(),
        ))
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(tls_error)?
        .with_root_certificates(roots)
        .with_no_client_auth();
        config.enable_early_data = false;
        Ok(Self {
            address,
            connector: RustlsConnector::from(Arc::new(config)),
        })
    }
}
impl TransportConnector for TlsConnector {
    fn connect(&self) -> BoxFuture<'_, Result<TransportStream, TransportError>> {
        Box::pin(async move {
            let stream = tokio::time::timeout(TCP_CONNECT_TIMEOUT, async {
                let addresses = tokio::net::lookup_host(&self.address).await?.collect();
                connect_any(addresses, TcpStream::connect).await
            })
            .await
            .map_err(|_| timed_out("TCP connection timed out"))??;
            configure(&stream)?;
            let name = ServerName::try_from(TLS_SERVER_NAME).map_err(tls_error)?;
            let stream =
                tokio::time::timeout(TLS_HANDSHAKE_TIMEOUT, self.connector.connect(name, stream))
                    .await
                    .map_err(|_| timed_out("TLS handshake timed out"))??;
            Ok(Box::new(stream) as TransportStream)
        })
    }

    fn link_profile(&self) -> LinkProfile {
        LinkProfile::REMOTE
    }
}

pub struct TlsListener {
    listener: TcpListener,
    acceptor: TlsAcceptor,
}
impl TlsListener {
    pub async fn bind(
        address: SocketAddr,
        certificate_der: Vec<u8>,
        private_key_der: Vec<u8>,
    ) -> Result<Self, TransportError> {
        let mut config = ServerConfig::builder_with_provider(Arc::new(
            rustls::crypto::aws_lc_rs::default_provider(),
        ))
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(tls_error)?
        .with_no_client_auth()
        .with_single_cert(
            vec![CertificateDer::from(certificate_der)],
            PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(private_key_der)),
        )
        .map_err(tls_error)?;
        config.max_early_data_size = 0;
        config.send_tls13_tickets = 0;
        Ok(Self {
            listener: TcpListener::bind(address).await?,
            acceptor: TlsAcceptor::from(Arc::new(config)),
        })
    }
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.listener.local_addr()
    }
    pub fn acceptor(&self) -> TlsAcceptor {
        self.acceptor.clone()
    }
    /// The Host admits TCP streams before starting TLS so unauthenticated handshakes are bounded.
    pub async fn accept_tcp(&self) -> io::Result<(TcpStream, SocketAddr)> {
        let (stream, peer) = self.listener.accept().await?;
        configure(&stream)?;
        Ok((stream, peer))
    }
}
impl TransportListener for TlsListener {
    fn accept(&self) -> BoxFuture<'_, Result<TransportStream, TransportError>> {
        Box::pin(async move {
            let (stream, _) = self.accept_tcp().await?;
            let stream = tokio::time::timeout(TLS_HANDSHAKE_TIMEOUT, self.acceptor.accept(stream))
                .await
                .map_err(|_| {
                    io::Error::new(io::ErrorKind::TimedOut, "TLS handshake timed out")
                })??;
            Ok(Box::new(stream) as TransportStream)
        })
    }
}
/// Connects to the first address that answers. Each attempt runs alone for
/// `CONNECT_ATTEMPT_DELAY`, or until it fails, before the next one starts, so
/// one unreachable address does not hold up the others.
async fn connect_any<T, Attempt>(
    addresses: Vec<SocketAddr>,
    connect: impl Fn(SocketAddr) -> Attempt,
) -> io::Result<T>
where
    T: Send + 'static,
    Attempt: std::future::Future<Output = io::Result<T>> + Send + 'static,
{
    let mut pending = addresses.into_iter();
    let mut attempts = JoinSet::new();
    let mut next_attempt = Instant::now();
    let mut last_error = None;
    loop {
        tokio::select! {
            Some(attempt) = attempts.join_next(), if !attempts.is_empty() => {
                match attempt {
                    Ok(Ok(stream)) => return Ok(stream),
                    Ok(Err(error)) => last_error = Some(error),
                    Err(error) => last_error = Some(io::Error::other(error)),
                }
                next_attempt = Instant::now();
            }
            _ = tokio::time::sleep_until(next_attempt), if pending.len() != 0 => {
                let address = pending.next().expect("a pending address");
                attempts.spawn(connect(address));
                next_attempt = Instant::now() + CONNECT_ATTEMPT_DELAY;
            }
            else => {
                return Err(last_error.unwrap_or_else(|| {
                    io::Error::new(io::ErrorKind::NotFound, "address resolved to nothing")
                }));
            }
        }
    }
}

fn configure(stream: &TcpStream) -> io::Result<()> {
    stream.set_nodelay(true)?;
    let keepalive = TcpKeepalive::new()
        .with_time(KEEPALIVE_TIME)
        .with_interval(KEEPALIVE_INTERVAL);
    #[cfg(not(windows))]
    let keepalive = keepalive.with_retries(3);
    SockRef::from(stream).set_tcp_keepalive(&keepalive)?;
    #[cfg(any(target_os = "linux", target_os = "android"))]
    SockRef::from(stream).set_tcp_notsent_lowat(NOTSENT_LOWAT)?;
    Ok(())
}

fn timed_out(message: &'static str) -> TransportError {
    TransportError::Io(io::Error::new(io::ErrorKind::TimedOut, message))
}

fn tls_error(error: impl std::fmt::Display) -> TransportError {
    TransportError::Other(format!("TLS configuration rejected: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn address(port: u16) -> SocketAddr {
        SocketAddr::from(([127, 0, 0, 1], port))
    }

    /// Port 1 never answers, port 2 refuses, any other port connects.
    async fn fake_connect(address: SocketAddr) -> io::Result<SocketAddr> {
        match address.port() {
            1 => std::future::pending().await,
            2 => Err(io::ErrorKind::ConnectionRefused.into()),
            _ => Ok(address),
        }
    }

    #[tokio::test(start_paused = true)]
    async fn an_unreachable_address_does_not_hold_up_the_next_one() {
        let started = Instant::now();
        let connected = connect_any(vec![address(1), address(2), address(3)], fake_connect)
            .await
            .unwrap();
        assert_eq!(connected, address(3));
        // The silent address runs alone for one delay; the refusal starts the next at once.
        assert_eq!(started.elapsed(), CONNECT_ATTEMPT_DELAY);
    }

    #[tokio::test(start_paused = true)]
    async fn the_last_failure_is_reported_when_no_address_answers() {
        let error = connect_any(vec![address(2)], fake_connect)
            .await
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::ConnectionRefused);
        let error = connect_any(Vec::new(), fake_connect).await.unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::NotFound);
    }

    #[tokio::test]
    async fn both_ends_enable_keepalive() {
        let key = rcgen::generate_simple_self_signed(vec![TLS_SERVER_NAME.to_string()]).unwrap();
        let listener = TlsListener::bind(
            "127.0.0.1:0".parse().unwrap(),
            key.cert.der().to_vec(),
            key.key_pair.serialize_der(),
        )
        .await
        .unwrap();
        let address = listener.local_addr().unwrap();
        let client = tokio::spawn(TcpStream::connect(address));
        let (accepted, peer) = listener.accept_tcp().await.unwrap();
        let client = client.await.unwrap().unwrap();
        assert_eq!(peer, client.local_addr().unwrap());
        assert!(SockRef::from(&accepted).keepalive().unwrap());
        configure(&client).unwrap();
        assert!(SockRef::from(&client).keepalive().unwrap());
        #[cfg(any(target_os = "linux", target_os = "android"))]
        for stream in [&accepted, &client] {
            assert_eq!(
                SockRef::from(stream).tcp_notsent_lowat().unwrap(),
                NOTSENT_LOWAT
            );
        }
    }
}
