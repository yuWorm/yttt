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
    EndpointConfig, IrohConnector, IrohEndpoint, OpenGate, PeerAddr, RelayTarget,
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
        expires_unix: now + 600,
    })
}

async fn endpoint(
    secret: [u8; 32],
    url: &str,
    token: Option<String>,
    alpns: Vec<Vec<u8>>,
) -> IrohEndpoint {
    IrohEndpoint::bind(EndpointConfig {
        secret,
        relay: Some(RelayTarget {
            url: url.to_owned(),
            token,
        }),
        alpns,
        gate: Arc::new(OpenGate),
        bind_addr: None,
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

    let host = endpoint(HOST, &url, Some(ticket(&HOST)), vec![ALPN.to_vec()]).await;
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

    let phone = endpoint(PHONE, &url, Some(ticket(&PHONE)), Vec::new()).await;
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
    let host = endpoint(HOST, &url, None, vec![ALPN.to_vec()]).await;
    assert!(
        tokio::time::timeout(Duration::from_secs(3), host.online())
            .await
            .is_err()
    );
    assert_eq!(node.account(ACCOUNT), None);
    host.close().await;
    node.shutdown().await;
}
