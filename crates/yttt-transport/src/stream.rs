use std::{future::Future, pin::Pin, sync::Arc};

use tokio::io::{AsyncRead, AsyncWrite};

use crate::TransportError;

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

pub trait TransportIo: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T> TransportIo for T where T: AsyncRead + AsyncWrite + Unpin + Send {}

pub type TransportStream = Box<dyn TransportIo>;

pub trait TransportListener: Send + Sync + 'static {
    fn accept(&self) -> BoxFuture<'_, Result<TransportStream, TransportError>>;
}

pub trait TransportConnector: Send + Sync + 'static {
    fn connect(&self) -> BoxFuture<'_, Result<TransportStream, TransportError>>;
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
}
