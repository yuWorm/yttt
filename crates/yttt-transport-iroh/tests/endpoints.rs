use std::{
    collections::HashSet,
    sync::{Arc, Mutex},
    time::Duration,
};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use yttt_protocol::EndpointKey;
use yttt_transport::TransportConnector as _;
use yttt_transport_iroh::{
    DirectPaths, EndpointConfig, IrohConnector, IrohEndpoint, OpenGate, PathKind, PeerAddr,
    PeerGate, RelaySetup, endpoint_key,
};

const ALPN: &[u8] = b"yttt/test/1";
const HOST_SECRET: [u8; 32] = [11; 32];
const CLIENT_SECRET: [u8; 32] = [12; 32];

/// Admits the peers in its set, which tests change while connections are open.
#[derive(Default)]
struct ListGate(Mutex<HashSet<EndpointKey>>);

impl ListGate {
    fn allow(&self, peer: EndpointKey) {
        self.0.lock().unwrap().insert(peer);
    }
    fn deny(&self, peer: EndpointKey) {
        self.0.lock().unwrap().remove(&peer);
    }
}

impl PeerGate for ListGate {
    fn admit(&self, peer: EndpointKey, alpn: &[u8]) -> bool {
        alpn == ALPN && self.0.lock().unwrap().contains(&peer)
    }
}

async fn host(gate: Arc<ListGate>) -> IrohEndpoint {
    IrohEndpoint::bind(EndpointConfig {
        secret: HOST_SECRET,
        relay: RelaySetup::Off,
        alpns: vec![ALPN.to_vec()],
        gate,
        direct: DirectPaths::Bind("127.0.0.1:0".parse().unwrap()),
    })
    .await
    .unwrap()
}

async fn client() -> IrohEndpoint {
    IrohEndpoint::bind(EndpointConfig {
        secret: CLIENT_SECRET,
        relay: RelaySetup::Off,
        alpns: Vec::new(),
        gate: Arc::new(OpenGate),
        direct: DirectPaths::Bind("127.0.0.1:0".parse().unwrap()),
    })
    .await
    .unwrap()
}

fn address_of(endpoint: &IrohEndpoint) -> PeerAddr {
    PeerAddr {
        key: endpoint.id(),
        relay_url: None,
        direct_addrs: endpoint.direct_addrs(),
    }
}

/// Echoes every stream of every admitted connection until the endpoint closes.
fn serve_echo(endpoint: IrohEndpoint) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        while let Some(connection) = endpoint.accept().await {
            tokio::spawn(async move {
                while let Some(mut stream) = connection.accept_stream().await {
                    tokio::spawn(async move {
                        let mut buffer = [0_u8; 64];
                        while let Ok(read) = stream.read(&mut buffer).await {
                            if read == 0 || stream.write_all(&buffer[..read]).await.is_err() {
                                break;
                            }
                        }
                    });
                }
            });
        }
    })
}

async fn echo(
    stream: &mut yttt_transport::TransportStream,
    text: &[u8],
) -> std::io::Result<Vec<u8>> {
    stream.write_all(text).await?;
    let mut reply = vec![0_u8; text.len()];
    tokio::time::timeout(Duration::from_secs(10), stream.read_exact(&mut reply))
        .await
        .map_err(|_| std::io::Error::other("echo timed out"))??;
    Ok(reply)
}

#[tokio::test(flavor = "multi_thread")]
async fn an_admitted_peer_reaches_the_host_directly_on_several_streams() {
    let gate = Arc::new(ListGate::default());
    gate.allow(endpoint_key(&CLIENT_SECRET));
    let host = host(gate).await;
    assert_eq!(host.id(), endpoint_key(&HOST_SECRET));
    let server = serve_echo(host.clone());
    let connector = IrohConnector::new(client().await, address_of(&host), ALPN);

    let mut first = connector.connect().await.unwrap();
    let mut second = connector.connect().await.unwrap();
    assert_eq!(echo(&mut first, b"one").await.unwrap(), b"one");
    assert_eq!(echo(&mut second, b"two").await.unwrap(), b"two");
    assert_eq!(connector.path_kind().await, PathKind::Direct);
    let report = connector.path_report().await.unwrap();
    assert!(
        report.paths.iter().any(|path| path.selected && !path.relay),
        "{report}"
    );
    assert_eq!(report.hint(), None);

    host.close().await;
    server.await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_peer_the_gate_refuses_gets_no_stream() {
    let host = host(Arc::new(ListGate::default())).await;
    let server = serve_echo(host.clone());
    let connector = IrohConnector::new(client().await, address_of(&host), ALPN);

    let refused = async {
        let mut stream = connector.connect().await?;
        echo(&mut stream, b"hello")
            .await
            .map_err(yttt_transport::TransportError::Io)
    };
    assert!(refused.await.is_err());

    host.close().await;
    server.await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn revoking_a_peer_closes_its_connection_and_refuses_new_streams() {
    let gate = Arc::new(ListGate::default());
    let device = endpoint_key(&CLIENT_SECRET);
    gate.allow(device);
    let host = host(gate.clone()).await;
    let server = serve_echo(host.clone());
    let client = client().await;
    let connection = client.connect(&address_of(&host), ALPN).await.unwrap();
    let mut stream = connection.open_stream().await.unwrap();
    assert_eq!(echo(&mut stream, b"before").await.unwrap(), b"before");

    gate.deny(device);
    assert_eq!(host.close_peer(device), 1);
    tokio::time::timeout(Duration::from_secs(10), connection.closed())
        .await
        .expect("the Host closed the connection");
    assert!(echo(&mut stream, b"after").await.is_err());
    let reconnect = async {
        let connection = client.connect(&address_of(&host), ALPN).await?;
        let mut stream = connection.open_stream().await?;
        echo(&mut stream, b"again")
            .await
            .map_err(yttt_transport::TransportError::Io)
    };
    assert!(reconnect.await.is_err());

    host.close().await;
    server.await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_new_stream_rechecks_the_gate_on_a_live_connection() {
    let gate = Arc::new(ListGate::default());
    let device = endpoint_key(&CLIENT_SECRET);
    gate.allow(device);
    let host = host(gate.clone()).await;
    let server = serve_echo(host.clone());
    let client = client().await;
    let connection = client.connect(&address_of(&host), ALPN).await.unwrap();
    let mut first = connection.open_stream().await.unwrap();
    assert_eq!(echo(&mut first, b"first").await.unwrap(), b"first");

    // Revoked in the gate only; the connection itself was never closed.
    gate.deny(device);
    let mut second = connection.open_stream().await.unwrap();
    assert!(echo(&mut second, b"second").await.is_err());
    tokio::time::timeout(Duration::from_secs(10), connection.closed())
        .await
        .expect("the Host closed the connection");

    host.close().await;
    server.await.unwrap();
}

/// The Session schedules heartbeats and keystrokes ahead of bulk messages, but
/// only among bytes not yet handed to QUIC. A peer that has not read must stop
/// the writer after a small window, or a heartbeat waits behind megabytes on a
/// slow relay.
#[tokio::test]
async fn a_writer_stops_soon_after_the_peer_stops_reading() {
    let gate = Arc::new(ListGate::default());
    gate.allow(endpoint_key(&CLIENT_SECRET));
    let host = host(gate).await;
    let address = address_of(&host);
    let (accepted, mut held) = tokio::sync::mpsc::channel(1);
    let server = tokio::spawn({
        let host = host.clone();
        async move {
            let connection = host.accept().await.unwrap();
            let stream = connection.accept_stream().await.unwrap();
            accepted.send((connection, stream)).await.unwrap();
            std::future::pending::<()>().await;
        }
    });
    let connector = IrohConnector::new(client().await, address, ALPN);
    let mut stream = connector.connect().await.unwrap();
    let chunk = vec![0_u8; 16 * 1024];
    let mut written = 0;
    while let Ok(result) =
        tokio::time::timeout(Duration::from_millis(500), stream.write_all(&chunk)).await
    {
        result.unwrap();
        written += chunk.len();
        assert!(written <= 4 * 1024 * 1024, "the writer never stopped");
    }
    let _held = held.recv().await.unwrap();
    assert!(
        written <= 512 * 1024,
        "{written} bytes were accepted for a peer that read nothing"
    );
    server.abort();
}

/// Serves streams on every admitted connection, but only answers on those
/// after the first `silent` connections; returns the count of connections.
fn serve_echo_after(
    endpoint: IrohEndpoint,
    silent: usize,
) -> (
    tokio::task::JoinHandle<()>,
    Arc<std::sync::atomic::AtomicUsize>,
) {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let accepted = Arc::new(AtomicUsize::new(0));
    let count = accepted.clone();
    let server = tokio::spawn(async move {
        while let Some(connection) = endpoint.accept().await {
            let answers = count.fetch_add(1, Ordering::SeqCst) >= silent;
            tokio::spawn(async move {
                let mut held = Vec::new();
                while let Some(mut stream) = connection.accept_stream().await {
                    if !answers {
                        held.push(stream);
                        continue;
                    }
                    tokio::spawn(async move {
                        let mut buffer = [0_u8; 64];
                        while let Ok(read) = stream.read(&mut buffer).await {
                            if read == 0 || stream.write_all(&buffer[..read]).await.is_err() {
                                break;
                            }
                        }
                    });
                }
            });
        }
    });
    (server, accepted)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_connection_that_never_answered_a_handshake_is_dialed_anew() {
    let gate = Arc::new(ListGate::default());
    gate.allow(endpoint_key(&CLIENT_SECRET));
    let host = host(gate).await;
    let (server, accepted) = serve_echo_after(host.clone(), 1);
    let connector = IrohConnector::new(client().await, address_of(&host), ALPN);

    let mut silent = connector.connect().await.unwrap();
    silent.write_all(b"hello").await.unwrap();
    connector.handshake_settled(false);

    let mut stream = connector.connect().await.unwrap();
    assert_eq!(echo(&mut stream, b"again").await.unwrap(), b"again");
    assert_eq!(accepted.load(std::sync::atomic::Ordering::SeqCst), 2);
    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_connection_another_session_still_uses_survives_a_failed_handshake() {
    let gate = Arc::new(ListGate::default());
    gate.allow(endpoint_key(&CLIENT_SECRET));
    let host = host(gate).await;
    let (server, accepted) = serve_echo_after(host.clone(), 0);
    let connector = IrohConnector::new(client().await, address_of(&host), ALPN);

    let mut first = connector.connect().await.unwrap();
    assert_eq!(echo(&mut first, b"one").await.unwrap(), b"one");
    connector.handshake_settled(true);
    // Another handshake on the same connection, unanswered, while the first
    // session still runs on it.
    let failed = connector.connect().await.unwrap();
    connector.handshake_settled(false);
    drop(failed);

    let mut second = connector.connect().await.unwrap();
    assert_eq!(echo(&mut second, b"two").await.unwrap(), b"two");
    assert_eq!(echo(&mut first, b"still").await.unwrap(), b"still");
    assert_eq!(accepted.load(std::sync::atomic::Ordering::SeqCst), 1);
    server.abort();
}

/// After a phone sleeps or changes networks, the connection it reconnects on
/// can be dead though it answered before: its session's stream has ended, so
/// an unanswered handshake dials anew instead of waiting out QUIC's idle
/// timeout on it.
#[tokio::test(flavor = "multi_thread")]
async fn a_connection_no_session_uses_is_dialed_anew_after_a_failed_handshake() {
    let gate = Arc::new(ListGate::default());
    gate.allow(endpoint_key(&CLIENT_SECRET));
    let host = host(gate).await;
    let (server, accepted) = serve_echo_after(host.clone(), 0);
    let connector = IrohConnector::new(client().await, address_of(&host), ALPN);

    let mut first = connector.connect().await.unwrap();
    assert_eq!(echo(&mut first, b"one").await.unwrap(), b"one");
    connector.handshake_settled(true);
    drop(first);
    let failed = connector.connect().await.unwrap();
    connector.handshake_settled(false);
    drop(failed);

    let mut again = connector.connect().await.unwrap();
    assert_eq!(echo(&mut again, b"again").await.unwrap(), b"again");
    assert_eq!(accepted.load(std::sync::atomic::Ordering::SeqCst), 2);
    server.abort();
}

/// Admits everyone, but takes its time over one peer.
struct SlowGate {
    slow: EndpointKey,
}

impl PeerGate for SlowGate {
    fn admit(&self, peer: EndpointKey, alpn: &[u8]) -> bool {
        if peer == self.slow {
            std::thread::sleep(Duration::from_secs(4));
        }
        alpn == ALPN
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_slow_handshake_does_not_hold_up_another_peer() {
    const OTHER_SECRET: [u8; 32] = [13; 32];
    let host = IrohEndpoint::bind(EndpointConfig {
        secret: HOST_SECRET,
        relay: RelaySetup::Off,
        alpns: vec![ALPN.to_vec()],
        gate: Arc::new(SlowGate {
            slow: endpoint_key(&CLIENT_SECRET),
        }),
        direct: DirectPaths::Bind("127.0.0.1:0".parse().unwrap()),
    })
    .await
    .unwrap();
    let address = address_of(&host);
    let slow = IrohConnector::new(client().await, address.clone(), ALPN);
    let other = IrohConnector::new(
        IrohEndpoint::bind(EndpointConfig {
            secret: OTHER_SECRET,
            relay: RelaySetup::Off,
            alpns: Vec::new(),
            gate: Arc::new(OpenGate),
            direct: DirectPaths::Bind("127.0.0.1:0".parse().unwrap()),
        })
        .await
        .unwrap(),
        address,
        ALPN,
    );
    let accepting = tokio::spawn({
        let host = host.clone();
        async move { host.accept().await.map(|connection| connection.peer()) }
    });
    let _slow = tokio::spawn(async move { slow.connect().await.map(drop) });
    tokio::time::sleep(Duration::from_millis(300)).await;
    let _other = tokio::spawn(async move { other.connect().await.map(drop) });
    let first = tokio::time::timeout(Duration::from_secs(2), accepting)
        .await
        .expect("the other peer waited for the slow handshake")
        .unwrap();
    assert_eq!(first, Some(endpoint_key(&OTHER_SECRET)));
}
