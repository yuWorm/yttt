//! iroh endpoints, as Host and Client build them, reaching each other through
//! a ticketed node.

use std::{
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use yttt_relay::{NodeConfig, RelayNode};
use yttt_relay_ticket::{Ticket, TicketSigner, public_key};
use yttt_transport::TransportConnector as _;
use yttt_transport_iroh::{
    DirectPaths, EndpointConfig, IrohConnector, IrohEndpoint, OpenGate, PeerAddr, RelaySetup,
    RelayTarget,
};

const NODE: &str = "local-1";
const CONTROL_SECRET: [u8; 32] = [1; 32];
const HOST: [u8; 32] = [51; 32];
const PHONE: [u8; 32] = [52; 32];
const ACCOUNT: u64 = 7;
const ALPN: &[u8] = b"yttt/test/1";

fn ticket(secret: &[u8; 32]) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    TicketSigner::from_secret(CONTROL_SECRET).issue(&Ticket {
        account_id: ACCOUNT,
        endpoint_id: public_key(secret),
        node_id: NODE.into(),
        rate_bps: 50_000_000,
        burst_bytes: 50_000_000,
        issued_unix: now,
        expires_unix: now + 600,
    })
}

async fn endpoint(
    direct: DirectPaths,
    secret: [u8; 32],
    url: &str,
    token: Option<String>,
    alpns: Vec<Vec<u8>>,
) -> IrohEndpoint {
    IrohEndpoint::bind(EndpointConfig {
        secret,
        relay: RelaySetup::Use(RelayTarget {
            url: url.to_owned(),
            token,
        }),
        alpns,
        gate: Arc::new(OpenGate),
        direct,
    })
    .await
    .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn ticketed_endpoints_meet_through_the_node_with_only_its_url() {
    let node = RelayNode::spawn(NodeConfig::local(
        NODE,
        "127.0.0.1:0".parse().unwrap(),
        &[TicketSigner::from_secret(CONTROL_SECRET).public_key()],
    ))
    .await
    .unwrap();
    let url = format!("http://{}", node.addr());

    let host = endpoint(
        DirectPaths::Any,
        HOST,
        &url,
        Some(ticket(&HOST)),
        vec![ALPN.to_vec()],
    )
    .await;
    tokio::time::timeout(Duration::from_secs(10), host.online())
        .await
        .expect("the Host reaches its relay with a ticket");
    let server = tokio::spawn({
        let host = host.clone();
        async move {
            let connection = host.accept().await.unwrap();
            let mut stream = connection.accept_stream().await.unwrap();
            let mut buffer = [0_u8; 5];
            stream.read_exact(&mut buffer).await.unwrap();
            stream.write_all(&buffer).await.unwrap();
            stream.flush().await.unwrap();
            connection.closed().await;
        }
    });

    let phone = endpoint(
        DirectPaths::Any,
        PHONE,
        &url,
        Some(ticket(&PHONE)),
        Vec::new(),
    )
    .await;
    let connector = IrohConnector::new(
        phone.clone(),
        PeerAddr {
            key: host.id(),
            relay_url: Some(url.clone()),
            direct_addrs: Vec::new(),
        },
        ALPN,
    );
    let mut stream = connector.connect().await.unwrap();
    stream.write_all(b"hello").await.unwrap();
    let mut reply = [0_u8; 5];
    tokio::time::timeout(Duration::from_secs(10), stream.read_exact(&mut reply))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&reply, b"hello");

    let account = node.account(ACCOUNT).expect("the account used the node");
    assert_eq!(account.connections, 2);
    assert!(account.relayed_bytes > 0);

    phone.close().await;
    host.close().await;
    server.abort();
    node.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn an_endpoint_without_a_ticket_never_comes_online() {
    let node = RelayNode::spawn(NodeConfig::local(
        NODE,
        "127.0.0.1:0".parse().unwrap(),
        &[TicketSigner::from_secret(CONTROL_SECRET).public_key()],
    ))
    .await
    .unwrap();
    let url = format!("http://{}", node.addr());
    let host = endpoint(DirectPaths::Any, HOST, &url, None, vec![ALPN.to_vec()]).await;
    assert!(
        tokio::time::timeout(Duration::from_secs(3), host.online())
            .await
            .is_err()
    );
    assert_eq!(node.account(ACCOUNT), None);
    host.close().await;
    node.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn endpoints_without_direct_paths_send_everything_through_the_node() {
    let node = RelayNode::spawn(NodeConfig::local(
        NODE,
        "127.0.0.1:0".parse().unwrap(),
        &[TicketSigner::from_secret(CONTROL_SECRET).public_key()],
    ))
    .await
    .unwrap();
    let url = format!("http://{}", node.addr());
    let host = endpoint(
        DirectPaths::Off,
        HOST,
        &url,
        Some(ticket(&HOST)),
        vec![ALPN.to_vec()],
    )
    .await;
    assert!(host.direct_addrs().is_empty());
    let server = tokio::spawn({
        let host = host.clone();
        async move {
            let connection = host.accept().await.unwrap();
            let mut stream = connection.accept_stream().await.unwrap();
            let mut buffer = vec![0_u8; 100_000];
            stream.read_exact(&mut buffer).await.unwrap();
            stream.write_all(b"done").await.unwrap();
            connection.closed().await;
        }
    });
    let phone = endpoint(
        DirectPaths::Off,
        PHONE,
        &url,
        Some(ticket(&PHONE)),
        Vec::new(),
    )
    .await;
    let connector = IrohConnector::new(
        phone.clone(),
        PeerAddr {
            key: host.id(),
            relay_url: Some(url.clone()),
            direct_addrs: Vec::new(),
        },
        ALPN,
    );
    let mut stream = connector.connect().await.unwrap();
    stream.write_all(&vec![7_u8; 100_000]).await.unwrap();
    let mut reply = [0_u8; 4];
    tokio::time::timeout(Duration::from_secs(20), stream.read_exact(&mut reply))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        connector.path_kind().await,
        yttt_transport_iroh::PathKind::Relay
    );
    let report = connector.path_report().await.unwrap();
    assert!(
        report.paths.iter().all(|path| path.relay) && report.hint().is_some(),
        "{report}"
    );
    assert!(node.account(ACCOUNT).unwrap().relayed_bytes > 100_000);

    phone.close().await;
    host.close().await;
    server.abort();
    node.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn an_endpoint_waiting_for_a_ticket_joins_the_node_once_it_has_one() {
    let node = RelayNode::spawn(NodeConfig::local(
        NODE,
        "127.0.0.1:0".parse().unwrap(),
        &[TicketSigner::from_secret(CONTROL_SECRET).public_key()],
    ))
    .await
    .unwrap();
    let url = format!("http://{}", node.addr());
    let host = IrohEndpoint::bind(EndpointConfig {
        secret: HOST,
        relay: RelaySetup::Pending,
        alpns: vec![ALPN.to_vec()],
        gate: Arc::new(OpenGate),
        direct: DirectPaths::Off,
    })
    .await
    .unwrap();
    host.set_relay(&RelayTarget {
        url: url.clone(),
        token: Some(ticket(&HOST)),
    })
    .await
    .unwrap();
    tokio::time::timeout(Duration::from_secs(10), host.online())
        .await
        .expect("the Host joined the node after getting its ticket");
    assert_eq!(node.account(ACCOUNT).map(|a| a.connections), Some(1));
    host.close().await;
    node.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_self_hosted_node_relays_for_endpoints_without_tickets() {
    let mut config = NodeConfig::local(NODE, "127.0.0.1:0".parse().unwrap(), &[]);
    config.open = Some(yttt_relay::OpenConfig {
        rate_bps: 10_000_000,
        burst_bytes: 1_000_000,
    });
    let node = RelayNode::spawn(config).await.unwrap();
    let url = format!("http://{}", node.addr());
    let host = endpoint(DirectPaths::Off, HOST, &url, None, vec![ALPN.to_vec()]).await;
    let server = tokio::spawn({
        let host = host.clone();
        async move {
            let connection = host.accept().await.unwrap();
            let mut stream = connection.accept_stream().await.unwrap();
            let mut buffer = [0_u8; 5];
            stream.read_exact(&mut buffer).await.unwrap();
            stream.write_all(&buffer).await.unwrap();
            connection.closed().await;
        }
    });
    let phone = endpoint(DirectPaths::Off, PHONE, &url, None, Vec::new()).await;
    let connector = IrohConnector::new(
        phone.clone(),
        PeerAddr {
            key: host.id(),
            relay_url: Some(url.clone()),
            direct_addrs: Vec::new(),
        },
        ALPN,
    );
    let mut stream = connector.connect().await.unwrap();
    stream.write_all(b"hello").await.unwrap();
    let mut reply = [0_u8; 5];
    tokio::time::timeout(Duration::from_secs(10), stream.read_exact(&mut reply))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&reply, b"hello");
    phone.close().await;
    host.close().await;
    server.abort();
    node.shutdown().await;

    let mut both = NodeConfig::local(NODE, "127.0.0.1:0".parse().unwrap(), &[]);
    both.open = Some(yttt_relay::OpenConfig {
        rate_bps: 1,
        burst_bytes: 1,
    });
    both.control = Some(yttt_relay::ControlConfig::from_parts(
        "http://control".into(),
        "token".into(),
        15,
    ));
    assert!(RelayNode::spawn(both).await.is_err());
}
