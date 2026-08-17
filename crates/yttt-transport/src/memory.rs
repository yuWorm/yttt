use tokio::{
    io::duplex,
    sync::{Mutex, mpsc, oneshot},
};

use crate::{BoxFuture, TransportConnector, TransportError, TransportListener, TransportStream};

const MEMORY_BUFFER_BYTES: usize = 256 * 1024;

pub fn memory_pair() -> (MemoryListener, MemoryConnector) {
    let (requests, incoming) = mpsc::channel(16);
    (
        MemoryListener {
            incoming: Mutex::new(incoming),
        },
        MemoryConnector { requests },
    )
}

pub struct MemoryListener {
    incoming: Mutex<mpsc::Receiver<oneshot::Sender<TransportStream>>>,
}

#[derive(Clone)]
pub struct MemoryConnector {
    requests: mpsc::Sender<oneshot::Sender<TransportStream>>,
}

impl TransportListener for MemoryListener {
    fn accept(&self) -> BoxFuture<'_, Result<TransportStream, TransportError>> {
        Box::pin(async {
            let reply = self
                .incoming
                .lock()
                .await
                .recv()
                .await
                .ok_or(TransportError::Closed)?;
            let (server, client) = duplex(MEMORY_BUFFER_BYTES);
            reply
                .send(Box::new(client))
                .map_err(|_| TransportError::Closed)?;
            Ok(Box::new(server) as TransportStream)
        })
    }
}

impl TransportConnector for MemoryConnector {
    fn connect(&self) -> BoxFuture<'_, Result<TransportStream, TransportError>> {
        let requests = self.requests.clone();
        Box::pin(async move {
            let (reply, connected) = oneshot::channel();
            requests
                .send(reply)
                .await
                .map_err(|_| TransportError::Closed)?;
            connected.await.map_err(|_| TransportError::Closed)
        })
    }
}

#[cfg(test)]
mod tests {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    use super::*;

    #[tokio::test]
    async fn memory_pair_exchanges_bytes_in_both_directions() {
        let (listener, connector) = memory_pair();
        let server = tokio::spawn(async move { listener.accept().await.unwrap() });
        let mut client = connector.connect().await.unwrap();
        let mut server = server.await.unwrap();

        client.write_all(b"ping").await.unwrap();
        client.flush().await.unwrap();
        let mut buffer = [0_u8; 4];
        server.read_exact(&mut buffer).await.unwrap();
        assert_eq!(&buffer, b"ping");

        server.write_all(b"pong").await.unwrap();
        server.flush().await.unwrap();
        client.read_exact(&mut buffer).await.unwrap();
        assert_eq!(&buffer, b"pong");
    }

    #[tokio::test]
    async fn memory_connector_fails_after_listener_drops() {
        let (listener, connector) = memory_pair();
        drop(listener);
        assert!(matches!(
            connector.connect().await,
            Err(TransportError::Closed)
        ));
    }
}
