use std::{future::Future, pin::Pin, sync::Arc, time::Duration};

use tokio::io::{AsyncRead, AsyncWrite};

use crate::TransportError;

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

pub trait TransportIo: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T> TransportIo for T where T: AsyncRead + AsyncWrite + Unpin + Send {}

pub type TransportStream = Box<dyn TransportIo>;

pub trait TransportListener: Send + Sync + 'static {
    fn accept(&self) -> BoxFuture<'_, Result<TransportStream, TransportError>>;
}

/// How long a link may take to connect, and to show it is alive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LinkProfile {
    pub connect_timeout: Duration,
    /// How long a connection may stay silent while a heartbeat is unanswered.
    pub liveness_timeout: Duration,
}

impl LinkProfile {
    /// An endpoint on this machine.
    pub const LOCAL: Self = Self {
        connect_timeout: Duration::from_secs(30),
        liveness_timeout: Duration::from_secs(15),
    };
    /// A network link (SSH, TLS), whose latency and buffering vary and whose
    /// reconnect may rebuild a session.
    pub const REMOTE: Self = Self {
        connect_timeout: Duration::from_secs(90),
        liveness_timeout: Duration::from_secs(45),
    };
}

pub trait TransportConnector: Send + Sync + 'static {
    fn connect(&self) -> BoxFuture<'_, Result<TransportStream, TransportError>>;

    fn link_profile(&self) -> LinkProfile {
        LinkProfile::LOCAL
    }
}

impl<T: TransportConnector> TransportConnector for Arc<T> {
    fn connect(&self) -> BoxFuture<'_, Result<TransportStream, TransportError>> {
        (**self).connect()
    }

    fn link_profile(&self) -> LinkProfile {
        (**self).link_profile()
    }
}

#[derive(Clone)]
pub struct SharedConnector {
    inner: Arc<dyn TransportConnector>,
}

impl SharedConnector {
    pub fn new(connector: impl TransportConnector) -> Self {
        Self {
            inner: Arc::new(connector),
        }
    }
}

impl TransportConnector for SharedConnector {
    fn connect(&self) -> BoxFuture<'_, Result<TransportStream, TransportError>> {
        self.inner.connect()
    }

    fn link_profile(&self) -> LinkProfile {
        self.inner.link_profile()
    }
}
