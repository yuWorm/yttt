//! A phone on a paid relay: a control plane, a relay node that
//! admits only ticket holders, a Host on the account. The phone has no direct
//! paths, so everything goes through the node. Signed out it cannot pair;
//! signed in it pairs, connects and gives its device slot back when it
//! forgets the Host. The harness follows `yttt-client-core/tests/paid_relay.rs`.
//!
//! Runs on SQLite; set `YTTT_TEST_DATABASE_URL` to a PostgreSQL server (see
//! packaging/relay-dev/docker-compose.full.yml) to run it there instead.

use std::{fs, path::Path, time::Duration};

use yttt_client_core::{ClientCore, control::ControlClient};
use yttt_control::test_support::{EMAIL, PASSWORD, TestRelay};
use yttt_core::model::ids::{ClientInstanceId, ProfileId};
use yttt_mobile_core::account::{self, Account};
use yttt_mobile_core::session::{
    MobileError, MobileSession, PairedHostRecord, RelayChoice, RelaySource, pair,
};
use yttt_protocol::{
    BuildIdentity, ProtocolRange, RESOURCE_PROTOCOL_VERSION, Request, Response,
    p2p::DevicePreset,
    remote_access::{
        PairingState, RelayAccount, RemoteAccessRequest, RemoteAccessResponse, RemoteAccessState,
        RemoteAccessStatus, SecretText,
    },
    workspace::WorkspaceProjectConfig,
};
use yttt_transport::{AuthToken, ClientIdentity, memory_pair};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt as _;

const TOKEN: [u8; 32] = [0x7c; 32];
const PROFILE: &str = "mobile-paid-relay";
const NODE: &str = "local-1";
const SECRET: [u8; 32] = [43; 32];

fn build() -> BuildIdentity {
    BuildIdentity {
        product_version: "0.3.0".to_string(),
        build_fingerprint: "mobile-paid-relay".to_string(),
        resource_compatibility: "mobile-paid-relay-v1".to_string(),
    }
}

async fn start_host(root: &Path) -> ClientCore {
    let runtime_root = root.join("runtime");
    fs::create_dir_all(&runtime_root).unwrap();
    let auth_token_file = runtime_root.join("auth-token");
    fs::write(&auth_token_file, TOKEN).unwrap();
    #[cfg(unix)]
    {
        fs::set_permissions(&runtime_root, fs::Permissions::from_mode(0o700)).unwrap();
        fs::set_permissions(&auth_token_file, fs::Permissions::from_mode(0o600)).unwrap();
    }
    let bootstrap = yttt_host::HostBootstrap {
        profile_id: ProfileId::new(PROFILE),
        runtime_root,
        state_root: root.join("state"),
        config_root: root.join("state/config"),
        project_config: WorkspaceProjectConfig::Project,
        auth_token_file,
        ssh_host_keys_file: root.join("ssh-host-keys.toml"),
        credential_namespace: "dev.yttt.ssh.mobile-paid-relay".to_string(),
        build: build(),
        lifetime: yttt_host::HostLifetime::Independent,
    };
    let (listener, connector) = memory_pair();
    let ready = bootstrap.ready_file();
    tokio::spawn(yttt_host::run_with_remote_work(bootstrap, || async move {
        Ok::<_, yttt_transport::TransportError>((listener, None))
    }));
    tokio::time::timeout(Duration::from_secs(5), async {
        while !ready.exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("Host ready timeout");
    ClientCore::connect(
        connector,
        ClientIdentity {
            expected_environment: None,
            credential_generation: 0,
            session_nonce: yttt_transport::new_session_nonce(),
            supported: ProtocolRange::exact(RESOURCE_PROTOCOL_VERSION),
            build: build(),
            profile_id: ProfileId::new(PROFILE),
            client_instance_id: ClientInstanceId::new("admin"),
            host_epoch_hint: None,
            can_force_stop: false,
            channel: yttt_protocol::ConnectionChannel::Session,
        },
        AuthToken::from_bytes(TOKEN),
    )
    .await
    .unwrap()
}

async fn remote_access(admin: &ClientCore, request: RemoteAccessRequest) -> RemoteAccessResponse {
    match admin.request(Request::RemoteAccess(request)).await.unwrap() {
        Response::RemoteAccess(response) => response,
        other => panic!("unexpected response {other:?}"),
    }
}

async fn status(admin: &ClientCore) -> RemoteAccessStatus {
    match remote_access(admin, RemoteAccessRequest::Status).await {
        RemoteAccessResponse::Status(status) => status,
        other => panic!("unexpected response {other:?}"),
    }
}

async fn eventually<T>(what: &str, mut check: impl AsyncFnMut() -> Option<T>) -> T {
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            if let Some(value) = check().await {
                return value;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("timed out waiting for {what}"))
}

/// Starts pairing on the Host; the code to give the phone.
async fn invite(admin: &ClientCore) -> String {
    let RemoteAccessResponse::PairingInvitation { code, .. } =
        remote_access(admin, RemoteAccessRequest::StartPairing).await
    else {
        panic!("expected an invitation");
    };
    code.0.clone()
}

async fn confirm(admin: &ClientCore) {
    let endpoint_id = eventually("the phone asking to pair", async || {
        match status(admin).await.p2p.pairing {
            Some(PairingState::AwaitingConfirmation { endpoint_id, .. }) => Some(endpoint_id),
            _ => None,
        }
    })
    .await;
    remote_access(
        admin,
        RemoteAccessRequest::ConfirmPairing {
            endpoint_id,
            preset: DevicePreset::Full,
        },
    )
    .await;
}

async fn devices(account: &Account) -> usize {
    account.session.devices().await.unwrap().devices.len()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_signed_in_phone_pairs_and_works_through_a_ticket_relay() {
    let relay = TestRelay::spawn(&[NODE]).await;
    relay.create_account(NODE).await;
    let relay_url = relay.node_url(0);
    let TestRelay {
        control,
        mut nodes,
        control_url,
        ..
    } = relay;
    let node = nodes.remove(0);

    // The Host joins the node with its own ticket.
    let host_session = ControlClient::new(&control_url)
        .unwrap()
        .login(EMAIL, PASSWORD)
        .await
        .unwrap();
    let temp = tempfile::tempdir().unwrap();
    let admin = start_host(temp.path()).await;
    remote_access(
        &admin,
        RemoteAccessRequest::SetRelayAccount {
            account: Some(RelayAccount {
                control_url: control_url.clone(),
                account_id: host_session.account_id,
                session: SecretText(host_session.token.clone()),
            }),
        },
    )
    .await;
    remote_access(&admin, RemoteAccessRequest::SetP2p { enabled: true }).await;
    let host_status = status(&admin).await.p2p;
    assert_eq!(host_status.effective, RemoteAccessState::Listening);
    eventually("the Host on the node", async || {
        node.account(host_session.account_id)
            .filter(|account| account.connections == 1)
    })
    .await;

    // Signed out, the code names the Host's control plane to sign in to.
    let code = invite(&admin).await;
    let refused = pair(
        &code,
        "iPhone",
        SECRET,
        RelayChoice::OnlyFromHost,
        None,
        false,
        |_| {},
    )
    .await;
    assert!(
        matches!(&refused, Err(MobileError::SignInRequired { control_url: url }) if *url == control_url),
        "{refused:?}"
    );
    // Asked to pair over the local network instead, a phone limited to the
    // relay has nothing to pair through.
    let refused = tokio::time::timeout(
        Duration::from_secs(60),
        pair(
            &code,
            "iPhone",
            SECRET,
            RelayChoice::OnlyFromHost,
            None,
            true,
            |_| {},
        ),
    )
    .await
    .expect("pairing without a ticket should fail, not hang");
    assert!(
        matches!(refused, Err(MobileError::Transport(_))),
        "{refused:?}"
    );
    remote_access(&admin, RemoteAccessRequest::CancelPairing).await;

    // Signed in, the phone registers a key for this Host and pairs through
    // the node with a ticket.
    let account = Account::sign_in(&control_url, EMAIL, PASSWORD)
        .await
        .unwrap();
    assert_eq!(account.email, EMAIL);
    let bad = Account::sign_in(&control_url, EMAIL, "wrong").await;
    assert!(bad.is_err());
    let before = devices(&account).await;
    let code = invite(&admin).await;
    let pairing = tokio::spawn({
        let account = account.clone();
        async move {
            pair(
                &code,
                "iPhone",
                SECRET,
                RelayChoice::OnlyFromHost,
                Some(&account),
                false,
                |_| {},
            )
            .await
        }
    });
    confirm(&admin).await;
    let record = pairing.await.unwrap().unwrap();
    let placement = record
        .host
        .placement
        .as_ref()
        .expect("the Host's placement");
    assert_eq!(placement.placement.relay_url, relay_url);
    assert!(record.host.ticket.is_some(), "the ticket is cached");
    assert_eq!(devices(&account).await, before + 1);

    // It connects the same way and the Host sees the key it derived.
    let phone = MobileSession::connect(
        &record,
        SECRET,
        RelayChoice::OnlyFromHost,
        Some(&account),
        "iPhone",
        std::sync::Arc::new(|_: &yttt_client_core::p2p::PairedHost| {}),
    )
    .await
    .unwrap();
    assert_eq!(phone.relay_source(), RelaySource::Account);
    phone
        .client()
        .request(Request::ListResources)
        .await
        .unwrap();
    let key = yttt_transport_iroh::endpoint_key(&account::host_key(&SECRET, &record.host.host));
    let listed = account.session.devices().await.unwrap();
    let entry = listed
        .devices
        .iter()
        .find(|device| device.endpoint_id == yttt_relay_ticket::hex(&key.0))
        .expect("the phone's key for this Host is on the account");
    assert!(entry.label.starts_with("iPhone → "), "{}", entry.label);
    phone.close().await;

    // Forgetting the Host gives the slot back.
    account.release(&SECRET, &record.host.host).await.unwrap();
    assert_eq!(devices(&account).await, before);

    node.shutdown().await;
    control.shutdown().await;
}

/// What the app would store: the record each time the core hands it over.
fn record_store() -> (
    yttt_client_core::relay_follow::RecordChanged,
    std::sync::Arc<parking_lot::Mutex<Vec<PairedHostRecord>>>,
) {
    let stored = std::sync::Arc::new(parking_lot::Mutex::new(Vec::new()));
    let sink = stored.clone();
    (
        std::sync::Arc::new(move |host: &yttt_client_core::p2p::PairedHost| {
            sink.lock().push(PairedHostRecord { host: host.clone() });
        }),
        stored,
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn a_connected_phone_follows_its_host_to_another_node_and_connects_from_its_cache() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(std::env::var("TEST_LOG").unwrap_or_else(|_| "off".into()))
        .with_test_writer()
        .try_init();
    let relay = TestRelay::spawn(&[NODE, "local-2"]).await;
    relay.create_account(NODE).await;
    let second_url = relay.node_url(1);
    let TestRelay {
        control,
        nodes,
        control_url,
        ..
    } = relay;
    let host_session = ControlClient::new(&control_url)
        .unwrap()
        .login(EMAIL, PASSWORD)
        .await
        .unwrap();
    let temp = tempfile::tempdir().unwrap();
    let admin = start_host(temp.path()).await;
    remote_access(
        &admin,
        RemoteAccessRequest::SetRelayAccount {
            account: Some(RelayAccount {
                control_url: control_url.clone(),
                account_id: host_session.account_id,
                session: SecretText(host_session.token.clone()),
            }),
        },
    )
    .await;
    remote_access(&admin, RemoteAccessRequest::SetP2p { enabled: true }).await;
    eventually("the Host on the first node", async || {
        nodes[0]
            .account(host_session.account_id)
            .filter(|account| account.connections == 1)
    })
    .await;
    let account = Account::sign_in(&control_url, EMAIL, PASSWORD)
        .await
        .unwrap();
    let code = invite(&admin).await;
    let pairing = tokio::spawn({
        let account = account.clone();
        async move {
            pair(
                &code,
                "iPhone",
                SECRET,
                RelayChoice::OnlyFromHost,
                Some(&account),
                false,
                |_| {},
            )
            .await
        }
    });
    confirm(&admin).await;
    let record = pairing.await.unwrap().unwrap();

    // Connected through the relay alone, the phone follows the Host's move
    // without anyone touching it.
    let (sink, stored) = record_store();
    let phone = MobileSession::connect(
        &record,
        SECRET,
        RelayChoice::OnlyFromHost,
        Some(&account),
        "iPhone",
        sink,
    )
    .await
    .unwrap();
    phone
        .client()
        .request(Request::ListResources)
        .await
        .unwrap();
    remote_access(
        &admin,
        RemoteAccessRequest::SetRelayNode {
            node_id: "local-2".into(),
        },
    )
    .await;
    eventually("the phone and the Host on the second node", async || {
        nodes[1]
            .account(host_session.account_id)
            .filter(|account| account.connections >= 2)
    })
    .await;
    // Within a few seconds of the move, with no one touching the phone.
    let moved_at = std::time::Instant::now();
    tokio::time::timeout(Duration::from_secs(15), async {
        while phone
            .client()
            .request(Request::ListResources)
            .await
            .is_err()
        {
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    })
    .await
    .expect("the session works again");
    eprintln!(
        "the phone worked again {:?} after the move",
        moved_at.elapsed()
    );
    let latest = stored
        .lock()
        .last()
        .cloned()
        .expect("the record was stored");
    let placement = latest.host.placement.as_ref().unwrap();
    assert_eq!(
        (
            placement.placement.node_id.as_str(),
            placement.placement.relay_url.as_str()
        ),
        ("local-2", second_url.as_str())
    );
    assert_eq!(
        latest
            .host
            .ticket
            .as_ref()
            .map(|ticket| ticket.node_id.as_str()),
        Some("local-2")
    );
    phone.close().await;

    // With the control plane gone, the stored record and its ticket still
    // reach the Host through its node.
    control.shutdown().await;
    let (sink, _) = record_store();
    let phone = tokio::time::timeout(
        Duration::from_secs(30),
        MobileSession::connect(
            &latest,
            SECRET,
            RelayChoice::OnlyFromHost,
            Some(&account),
            "iPhone",
            sink,
        ),
    )
    .await
    .expect("connects without waiting long on the control plane")
    .unwrap();
    assert_eq!(phone.relay_source(), RelaySource::Account);
    phone
        .client()
        .request(Request::ListResources)
        .await
        .unwrap();
    phone.close().await;

    // A control plane that accepts and never answers does not hold up a
    // phone whose stored ticket is still good: it connects at once and
    // asks in the background.
    let silent = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let silent_url = format!("http://{}", silent.local_addr().unwrap());
    let _held = tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok((stream, _)) = silent.accept().await {
            held.push(stream);
        }
    });
    let mut stalled = account.clone();
    stalled.session.control_url = silent_url;
    let (sink, _) = record_store();
    let started = std::time::Instant::now();
    let phone = MobileSession::connect(
        &latest,
        SECRET,
        RelayChoice::OnlyFromHost,
        Some(&stalled),
        "iPhone",
        sink,
    )
    .await
    .unwrap();
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "waited {:?} on a silent control plane",
        started.elapsed()
    );
    phone
        .client()
        .request(Request::ListResources)
        .await
        .unwrap();
    phone.close().await;
    for node in nodes {
        node.shutdown().await;
    }
}
