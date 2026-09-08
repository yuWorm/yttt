#![forbid(unsafe_code)]
use rustls::{
    ClientConfig, RootCertStore, ServerConfig,
    pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName},
};
use std::{io, net::SocketAddr, sync::Arc, time::Duration};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::{TlsAcceptor, TlsConnector as RustlsConnector};
use yttt_protocol::remote_access::TLS_SERVER_NAME;
use yttt_transport::{
    BoxFuture, TransportConnector, TransportError, TransportListener, TransportStream,
};

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);

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
            tokio::time::timeout(HANDSHAKE_TIMEOUT, async {
                let stream = TcpStream::connect(&self.address).await?;
                stream.set_nodelay(true)?;
                let name = ServerName::try_from(TLS_SERVER_NAME).map_err(tls_error)?;
                let stream = self.connector.connect(name, stream).await?;
                Ok(Box::new(stream) as TransportStream)
            })
            .await
            .map_err(|_| {
                TransportError::Io(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "TLS connection timed out",
                ))
            })?
        })
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
    pub async fn accept_tcp(&self) -> io::Result<TcpStream> {
        let (stream, _) = self.listener.accept().await?;
        stream.set_nodelay(true)?;
        Ok(stream)
    }
}
impl TransportListener for TlsListener {
    fn accept(&self) -> BoxFuture<'_, Result<TransportStream, TransportError>> {
        Box::pin(async move {
            let stream = self.accept_tcp().await?;
            let stream = tokio::time::timeout(HANDSHAKE_TIMEOUT, self.acceptor.accept(stream))
                .await
                .map_err(|_| {
                    io::Error::new(io::ErrorKind::TimedOut, "TLS handshake timed out")
                })??;
            Ok(Box::new(stream) as TransportStream)
        })
    }
}
fn tls_error(error: impl std::fmt::Display) -> TransportError {
    TransportError::Other(format!("TLS configuration rejected: {error}"))
}
