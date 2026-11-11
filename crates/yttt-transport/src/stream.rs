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
    /// How often the Client sends a heartbeat, which also measures latency.
    pub heartbeat_interval: Duration,
    /// How long a heartbeat may wait for its reply before the link is shown
    /// as unstable; the session stays up until `liveness_timeout`.
    pub unstable_after: Duration,
}

impl LinkProfile {
    /// An endpoint on this machine.
    pub const LOCAL: Self = Self {
        connect_timeout: Duration::from_secs(30),
        liveness_timeout: Duration::from_secs(15),
        heartbeat_interval: Duration::from_secs(5),
        unstable_after: Duration::from_secs(6),
    };
    /// A network link (SSH, TLS, P2P), whose latency and buffering vary and
    /// whose reconnect may rebuild a session. Any inbound message resets the
    /// silence, so a slow link that keeps delivering stays up; 20 s of nothing
    /// at all means the link is gone.
    pub const REMOTE: Self = Self {
        connect_timeout: Duration::from_secs(90),
        liveness_timeout: Duration::from_secs(20),
        heartbeat_interval: Duration::from_secs(2),
        unstable_after: Duration::from_secs(6),
    };
}

pub trait TransportConnector: Send + Sync + 'static {
    fn connect(&self) -> BoxFuture<'_, Result<TransportStream, TransportError>>;

    /// Reports how the handshake on the stream `connect` last returned ended:
    /// `answered` is false when the Host never replied. A connector that
    /// reuses a connection across streams drops one that has never answered,
    /// so the next attempt dials anew instead of waiting on a dead link.
    fn handshake_settled(&self, _answered: bool) {}

    fn link_profile(&self) -> LinkProfile {
        LinkProfile::LOCAL
    }
}

impl<T: TransportConnector> TransportConnector for Arc<T> {
    fn connect(&self) -> BoxFuture<'_, Result<TransportStream, TransportError>> {
        (**self).connect()
    }

    fn handshake_settled(&self, answered: bool) {
        (**self).handshake_settled(answered);
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

    fn handshake_settled(&self, answered: bool) {
        self.inner.handshake_settled(answered);
    }

    fn link_profile(&self) -> LinkProfile {
        self.inner.link_profile()
    }
}
