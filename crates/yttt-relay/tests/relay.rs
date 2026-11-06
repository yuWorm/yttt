use std::{
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use futures_util::{SinkExt as _, StreamExt as _};
use iroh_base::{RelayUrl, SecretKey};
use iroh_relay::{
    client::{Client, ClientBuilder},
    protos::relay::{ClientToRelayMsg, Datagrams, RelayToClientMsg},
};
use yttt_relay::{NodeConfig, RelayNode};
use yttt_relay_ticket::{
    Ticket, TicketSigner,
    api::{AccountStanding, Standing},
    public_key,
};

const NODE: &str = "local-1";
const CONTROL_SECRET: [u8; 32] = [1; 32];
const FAST: u32 = 50_000_000;

struct Device {
    secret: [u8; 32],
    account: u64,
}

const ALICE_LAPTOP: Device = Device {
    secret: [21; 32],
    account: 7,
};
const ALICE_PHONE: Device = Device {
    secret: [22; 32],
    account: 7,
};
const BOB: Device = Device {
    secret: [31; 32],
    account: 9,
};

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

async fn node() -> RelayNode {
    let signer = TicketSigner::from_secret(CONTROL_SECRET);
    RelayNode::spawn(NodeConfig::local(
        NODE,
        "127.0.0.1:0".parse().unwrap(),
        &[signer.public_key()],
    ))
    .await
    .unwrap()
}

fn ticket(device: &Device, rate_bps: u32, burst_bytes: u32, expires_unix: u64) -> String {
    issued_ticket(device, rate_bps, burst_bytes, now(), expires_unix)
}

fn issued_ticket(
    device: &Device,
    rate_bps: u32,
    burst_bytes: u32,
    issued_unix: u64,
    expires_unix: u64,
) -> String {
    TicketSigner::from_secret(CONTROL_SECRET).issue(&Ticket {
        account_id: device.account,
        endpoint_id: public_key(&device.secret),
        node_id: NODE.into(),
        rate_bps,
        burst_bytes,
        issued_unix,
        expires_unix,
    })
}

fn standing(account_id: u64, standing: Standing) -> AccountStanding {
    AccountStanding {
        account_id,
        standing,
        revoked_endpoints: Vec::new(),
    }
}

fn fast_ticket(device: &Device) -> String {
    ticket(device, FAST, FAST, now() + 600)
}

async fn connect(
    node: &RelayNode,
    device: &Device,
    token: Option<String>,
) -> Result<Client, iroh_relay::client::ConnectError> {
    let url: RelayUrl = format!("http://{}", node.addr()).parse().unwrap();
    let tls = iroh_relay::tls::CaTlsConfig::default()
        .client_config(Arc::new(rustls::crypto::ring::default_provider()))
        .unwrap();
    let mut builder = ClientBuilder::new(
        url,
        SecretKey::from_bytes(&device.secret),
        iroh_dns::dns::DnsResolver::new(),
    )
    .tls_client_config(tls);
    if let Some(token) = token {
        builder = builder.auth_token(token);
    }
    builder.connect().await
}

async fn send(client: &mut Client, to: &Device, bytes: usize) {
    client
        .send(ClientToRelayMsg::Datagrams {
            dst_endpoint_id: SecretKey::from_bytes(&to.secret).public(),
            datagrams: Datagrams::from(vec![0xa5_u8; bytes]),
        })
        .await
        .unwrap();
}

/// Reads until `total` datagram bytes arrived.
async fn receive(client: &mut Client, total: usize) {
    let mut received = 0;
    while received < total {
        let message = tokio::time::timeout(Duration::from_secs(20), client.next())
            .await
            .expect("datagrams arrive")
            .expect("the connection stays up")
            .unwrap();
        if let RelayToClientMsg::Datagrams { datagrams, .. } = message {
            received += datagrams.contents.len();
        }
    }
}

/// Waits for the relay to drop `client`.
async fn dropped(client: &mut Client) {
    tokio::time::timeout(Duration::from_secs(10), async {
        while let Some(Ok(_)) = client.next().await {}
    })
    .await
    .expect("the relay dropped the connection");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_device_without_a_valid_ticket_is_refused() {
    let node = node().await;
    let other_node = TicketSigner::from_secret(CONTROL_SECRET).issue(&Ticket {
        account_id: 7,
        endpoint_id: public_key(&ALICE_LAPTOP.secret),
        node_id: "elsewhere".into(),
        rate_bps: FAST,
        burst_bytes: FAST,
        issued_unix: now(),
        expires_unix: now() + 600,
    });
    let untrusted = TicketSigner::from_secret([2; 32]).issue(&Ticket {
        account_id: 7,
        endpoint_id: public_key(&ALICE_LAPTOP.secret),
        node_id: NODE.into(),
        rate_bps: FAST,
        burst_bytes: FAST,
        issued_unix: now(),
        expires_unix: now() + 600,
    });
    for token in [
        None,
        Some("garbage".to_owned()),
        Some(fast_ticket(&ALICE_PHONE)),
        Some(ticket(&ALICE_LAPTOP, FAST, FAST, now() - 1)),
        Some(other_node),
        Some(untrusted),
    ] {
        assert!(connect(&node, &ALICE_LAPTOP, token).await.is_err());
    }
    assert_eq!(node.account(ALICE_LAPTOP.account), None);
    assert!(
        connect(&node, &ALICE_LAPTOP, Some(fast_ticket(&ALICE_LAPTOP)))
            .await
            .is_ok()
    );
    node.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn an_account_is_charged_for_exactly_what_its_devices_send() {
    let node = node().await;
    let mut alice = connect(&node, &ALICE_LAPTOP, Some(fast_ticket(&ALICE_LAPTOP)))
        .await
        .unwrap();
    let mut bob = connect(&node, &BOB, Some(fast_ticket(&BOB))).await.unwrap();

    const COUNT: usize = 50;
    const SIZE: usize = 1_000;
    for _ in 0..COUNT {
        send(&mut alice, &BOB, SIZE).await;
    }
    receive(&mut bob, COUNT * SIZE).await;

    let charged = node.account(ALICE_LAPTOP.account).unwrap();
    assert_eq!(charged.connections, 1);
    // Each datagram frame adds a destination key and a few header bytes.
    assert!(
        (COUNT * SIZE..COUNT * (SIZE + 64)).contains(&(charged.relayed_bytes as usize)),
        "charged {} bytes",
        charged.relayed_bytes
    );
    assert!(node.account(BOB.account).unwrap().relayed_bytes < 1_024);
    node.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn devices_of_one_account_share_its_rate() {
    const RATE: u32 = 20_000;
    let node = node().await;
    let expires = now() + 600;
    let mut laptop = connect(
        &node,
        &ALICE_LAPTOP,
        Some(ticket(&ALICE_LAPTOP, RATE, 2_000, expires)),
    )
    .await
    .unwrap();
    let mut phone = connect(
        &node,
        &ALICE_PHONE,
        Some(ticket(&ALICE_PHONE, RATE, 2_000, expires)),
    )
    .await
    .unwrap();
    let mut bob = connect(&node, &BOB, Some(fast_ticket(&BOB))).await.unwrap();

    // 40 KB in total at 20 KB/s for the account; each device alone would be
    // under its own 20 KB/s and finish in about a second.
    let started = Instant::now();
    let sending = async {
        for _ in 0..20 {
            send(&mut laptop, &BOB, 1_000).await;
            send(&mut phone, &BOB, 1_000).await;
        }
    };
    tokio::join!(sending, receive(&mut bob, 40_000));
    let elapsed = started.elapsed();
    assert!(
        elapsed >= Duration::from_millis(1_500),
        "the account was not paced: {elapsed:?}"
    );
    node.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn throttling_an_account_slows_its_devices_down() {
    let node = node().await;
    let mut alice = connect(&node, &ALICE_LAPTOP, Some(fast_ticket(&ALICE_LAPTOP)))
        .await
        .unwrap();
    let mut bob = connect(&node, &BOB, Some(fast_ticket(&BOB))).await.unwrap();
    node.apply_standings(&[standing(
        ALICE_LAPTOP.account,
        Standing::Throttled {
            rate_bps: 10_000,
            burst_bytes: 1_000,
        },
    )]);

    let started = Instant::now();
    let sending = async {
        for _ in 0..20 {
            send(&mut alice, &BOB, 1_000).await;
        }
    };
    tokio::join!(sending, receive(&mut bob, 20_000));
    assert!(started.elapsed() >= Duration::from_millis(1_500));
    node.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_live_connection_outlasts_its_ticket_but_cannot_reconnect_on_it() {
    let node = node().await;
    let expiring = ticket(&ALICE_LAPTOP, FAST, FAST, now() + 2);
    let mut alice = connect(&node, &ALICE_LAPTOP, Some(expiring.clone()))
        .await
        .unwrap();
    let mut bob = connect(&node, &BOB, Some(fast_ticket(&BOB))).await.unwrap();
    tokio::time::sleep(Duration::from_secs(3)).await;

    // iroh reuses a relay connection's first token for every reconnect, so
    // the node never cuts a live connection at expiry.
    send(&mut alice, &BOB, 100).await;
    receive(&mut bob, 100).await;
    assert!(
        connect(
            &node,
            &ALICE_PHONE,
            Some(ticket(&ALICE_PHONE, FAST, FAST, now() - 1))
        )
        .await
        .is_err()
    );
    assert!(connect(&node, &ALICE_LAPTOP, Some(expiring)).await.is_err());
    node.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_removed_device_is_dropped_and_only_a_newer_ticket_readmits_it() {
    let node = node().await;
    let mut laptop = connect(&node, &ALICE_LAPTOP, Some(fast_ticket(&ALICE_LAPTOP)))
        .await
        .unwrap();
    let mut phone = connect(&node, &ALICE_PHONE, Some(fast_ticket(&ALICE_PHONE)))
        .await
        .unwrap();
    let mut bob = connect(&node, &BOB, Some(fast_ticket(&BOB))).await.unwrap();

    node.apply_standings(&[AccountStanding {
        account_id: ALICE_PHONE.account,
        standing: Standing::Active {
            rate_bps: FAST,
            burst_bytes: FAST,
        },
        revoked_endpoints: vec![yttt_relay_ticket::hex(&public_key(&ALICE_PHONE.secret))],
    }]);
    dropped(&mut phone).await;
    assert!(
        connect(&node, &ALICE_PHONE, Some(fast_ticket(&ALICE_PHONE)))
            .await
            .is_err()
    );

    // The account's other device keeps relaying.
    send(&mut laptop, &BOB, 100).await;
    receive(&mut bob, 100).await;

    // Registering the phone again yields a ticket issued after the removal.
    let newer = issued_ticket(&ALICE_PHONE, FAST, FAST, now() + 1, now() + 600);
    assert!(connect(&node, &ALICE_PHONE, Some(newer)).await.is_ok());
    node.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_device_that_moved_away_is_dropped_and_refused_on_its_old_ticket() {
    let node = node().await;
    let issued = now() - 10;
    let old = issued_ticket(&ALICE_PHONE, FAST, FAST, issued, now() + 600);
    let mut phone = connect(&node, &ALICE_PHONE, Some(old.clone()))
        .await
        .unwrap();
    let mut laptop = connect(&node, &ALICE_LAPTOP, Some(fast_ticket(&ALICE_LAPTOP)))
        .await
        .unwrap();
    let mut bob = connect(&node, &BOB, Some(fast_ticket(&BOB))).await.unwrap();

    // The control plane's list, as a restarted node receives it.
    node.apply_revocations(&[yttt_relay_ticket::api::Revocation {
        id: 7,
        endpoint_id: yttt_relay_ticket::hex(&public_key(&ALICE_PHONE.secret)),
        revoked_unix: issued + 5,
    }]);
    dropped(&mut phone).await;
    assert!(connect(&node, &ALICE_PHONE, Some(old)).await.is_err());
    send(&mut laptop, &BOB, 100).await;
    receive(&mut bob, 100).await;
    // Coming back later takes a ticket issued after the move.
    let back = issued_ticket(&ALICE_PHONE, FAST, FAST, issued + 6, now() + 600);
    assert!(connect(&node, &ALICE_PHONE, Some(back)).await.is_ok());
    node.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_suspended_account_is_dropped_and_refused_until_reinstated() {
    let node = node().await;
    let mut laptop = connect(&node, &ALICE_LAPTOP, Some(fast_ticket(&ALICE_LAPTOP)))
        .await
        .unwrap();
    let mut phone = connect(&node, &ALICE_PHONE, Some(fast_ticket(&ALICE_PHONE)))
        .await
        .unwrap();
    let mut bob = connect(&node, &BOB, Some(fast_ticket(&BOB))).await.unwrap();

    node.apply_standings(&[standing(ALICE_LAPTOP.account, Standing::Suspended)]);
    dropped(&mut laptop).await;
    dropped(&mut phone).await;
    assert!(
        connect(&node, &ALICE_LAPTOP, Some(fast_ticket(&ALICE_LAPTOP)))
            .await
            .is_err()
    );

    // Other accounts are untouched.
    let mut carol = connect(
        &node,
        &Device {
            secret: [41; 32],
            account: 10,
        },
        Some(ticket(
            &Device {
                secret: [41; 32],
                account: 10,
            },
            FAST,
            FAST,
            now() + 600,
        )),
    )
    .await
    .unwrap();
    send(&mut carol, &BOB, 100).await;
    receive(&mut bob, 100).await;

    node.apply_standings(&[standing(
        ALICE_LAPTOP.account,
        Standing::Active {
            rate_bps: FAST,
            burst_bytes: FAST,
        },
    )]);
    assert!(
        connect(&node, &ALICE_LAPTOP, Some(fast_ticket(&ALICE_LAPTOP)))
            .await
            .is_ok()
    );
    node.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_node_behind_a_proxy_serves_quic_with_the_proxys_certificate() {
    let dir = tempfile::tempdir().unwrap();
    let cert = rcgen::generate_simple_self_signed(vec!["relay.example".to_owned()]).unwrap();
    std::fs::write(dir.path().join("cert.pem"), cert.cert.pem()).unwrap();
    std::fs::write(dir.path().join("key.pem"), cert.key_pair.serialize_pem()).unwrap();
    let files = yttt_relay::TlsFiles {
        cert_chain: dir.path().join("cert.pem"),
        private_key: dir.path().join("key.pem"),
    };
    let mut config = NodeConfig::local(
        NODE,
        "127.0.0.1:0".parse().unwrap(),
        &[public_key(&CONTROL_SECRET)],
    );
    config.quic_listen = Some("127.0.0.1:0".parse().unwrap());
    config.quic_tls = Some(files.clone());
    let node = RelayNode::spawn(config.clone()).await.unwrap();
    assert!(node.quic_addr().is_some());
    // Plain HTTP stays, for the proxy in front.
    let mut client = connect(&node, &ALICE_LAPTOP, Some(fast_ticket(&ALICE_LAPTOP)))
        .await
        .unwrap();
    send(&mut client, &ALICE_LAPTOP, 10).await;
    node.shutdown().await;

    // With its own TLS, QUIC already has a certificate.
    config.tls = Some(files);
    assert!(RelayNode::spawn(config).await.is_err());
}
