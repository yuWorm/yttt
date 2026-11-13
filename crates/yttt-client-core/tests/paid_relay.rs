//! The whole paid-relay path: a control plane, a relay node
//! reporting to it, a Host and a phone on one account. The phone has no direct
//! paths, so the session runs through the node, is billed, and ends when the
//! account is suspended.
//!
//! Runs on SQLite; set `YTTT_TEST_DATABASE_URL` to a PostgreSQL server (see
//! packaging/relay-dev/docker-compose.full.yml) to run it there instead.

use std::{fs, path::Path, sync::Arc, time::Duration};

use yttt_client_core::{
    ClientCore, ConnectionState,
    control::{ControlClient, ControlSession, DeviceKind},
    p2p::pair,
};
use yttt_control::test_support::{self, TestRelay};
use yttt_core::model::ids::{ClientInstanceId, ProfileId};
use yttt_protocol::{
    BuildIdentity, ProtocolRange, RESOURCE_PROTOCOL_VERSION, Request, Response,
    p2p::{DevicePreset, PairingCode},
    remote_access::{
        PairingState, RelayAccount, RemoteAccessRequest, RemoteAccessResponse, RemoteAccessState,
        RemoteAccessStatus, SecretText,
    },
    workspace::WorkspaceProjectConfig,
};
use yttt_relay_ticket::api::SetSuspended;
use yttt_transport::{AuthToken, ClientIdentity, memory_pair};
use yttt_transport_iroh::{DirectPaths, EndpointConfig, IrohEndpoint, OpenGate, RelaySetup};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt as _;

const TOKEN: [u8; 32] = [0x5c; 32];
const PROFILE: &str = "paid-relay";
const NODE: &str = "local-1";
const PHONE: [u8; 32] = [61; 32];

fn build() -> BuildIdentity {
    BuildIdentity {
        product_version: "0.3.0".to_string(),
        build_fingerprint: "paid-relay".to_string(),
        resource_compatibility: "paid-relay-v1".to_string(),
    }
}

fn identity(client: &str, expected_environment: Option<String>) -> ClientIdentity {
    ClientIdentity {
        expected_environment,
        credential_generation: 0,
        session_nonce: yttt_transport::new_session_nonce(),
        supported: ProtocolRange::exact(RESOURCE_PROTOCOL_VERSION),
        build: build(),
        profile_id: ProfileId::new(PROFILE),
        client_instance_id: ClientInstanceId::new(client),
        host_epoch_hint: None,
        can_force_stop: false,
        channel: yttt_protocol::ConnectionChannel::Session,
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
        credential_namespace: "dev.yttt.ssh.paid-relay".to_string(),
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
        identity("admin", None),
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

/// A control plane and one reporting relay node per id; the first is where
/// accounts start.
async fn paid_relay_nodes(ids: &[&str]) -> TestRelay {
    TestRelay::spawn(ids).await
}

async fn account(control_url: &str) -> ControlSession {
    ControlClient::new(control_url)
        .unwrap()
        .login(test_support::EMAIL, test_support::PASSWORD)
        .await
        .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_paid_relay_carries_billed_sessions_until_the_account_is_suspended() {
    let relay = paid_relay_nodes(&[NODE]).await;
    relay.create_account(NODE).await;
    let TestRelay {
        control,
        mut nodes,
        control_url,
        ..
    } = relay;
    let node = nodes.remove(0);
    let relay_url = format!("http://{}", node.addr());
    let session = account(&control_url).await;

    // The Host joins the node with its own ticket.
    let temp = tempfile::tempdir().unwrap();
    let admin = start_host(temp.path()).await;
    remote_access(
        &admin,
        RemoteAccessRequest::SetRelayAccount {
            account: Some(RelayAccount {
                control_url: control_url.clone(),
                account_id: session.account_id,
                session: SecretText(session.token.clone()),
            }),
        },
    )
    .await;
    remote_access(
        &admin,
        RemoteAccessRequest::SetP2p {
            enabled: true,
        },
    )
    .await;
    let host_status = status(&admin).await.p2p;
    assert_eq!(host_status.effective, RemoteAccessState::Listening);
    let relay_account = host_status.relay_account.unwrap();
    assert_eq!(relay_account.relay_url.as_deref(), Some(relay_url.as_str()));
    assert_eq!(relay_account.error, None);
    eventually("the Host on the node", async || {
        node.account(session.account_id)
            .filter(|account| account.connections == 1)
    })
    .await;

    // The phone registers its key for this Host, gets a ticket for the
    // Host's node, and has no direct paths.
    let phone_kind = DeviceKind::Client {
        host: host_status.endpoint_id.unwrap().0,
    };
    session
        .register_device(&PHONE, "phone", phone_kind)
        .await
        .unwrap();
    let ticket = session.ticket(&PHONE, phone_kind).await.unwrap();
    let phone = IrohEndpoint::bind(EndpointConfig {
        secret: PHONE,
        relay: RelaySetup::Use(ticket.target()),
        alpns: Vec::new(),
        gate: Arc::new(OpenGate),
        direct: DirectPaths::Off,
    })
    .await
    .unwrap();

    let RemoteAccessResponse::PairingInvitation { code, .. } =
        remote_access(&admin, RemoteAccessRequest::StartPairing).await
    else {
        panic!("expected an invitation");
    };
    let code = PairingCode::decode(&code.0).unwrap();
    assert_eq!(code.control_url.as_deref(), Some(control_url.as_str()));
    let pairing = tokio::spawn({
        let phone = phone.clone();
        let relay_url = relay_url.clone();
        async move { pair(&phone, &code, Some(relay_url), "phone", |_| {}).await }
    });
    let endpoint_id = eventually("the phone asking to pair", async || {
        match status(&admin).await.p2p.pairing {
            Some(PairingState::AwaitingConfirmation { endpoint_id, .. }) => Some(endpoint_id),
            _ => None,
        }
    })
    .await;
    remote_access(
        &admin,
        RemoteAccessRequest::ConfirmPairing {
            endpoint_id,
            preset: DevicePreset::Full,
        },
    )
    .await;
    let paired = pairing.await.unwrap().unwrap();
    let client = ClientCore::connect(
        paired.connector(phone),
        identity("phone", Some(paired.environment_id.clone())),
        paired.credential(PHONE),
    )
    .await
    .unwrap();
    client.request(Request::ListResources).await.unwrap();

    // The node billed what the phone and Host sent, and the control plane has it.
    let billed = eventually("usage at the control plane", async || {
        let account = session.account().await.unwrap();
        (account.month_upstream_bytes > 0).then_some(account.month_upstream_bytes)
    })
    .await;
    assert!(billed <= node.account(session.account_id).unwrap().relayed_bytes);

    // Suspension reaches the node through its next report and cuts the session.
    let suspended = reqwest::Client::new()
        .post(format!(
            "{control_url}/v1/dev/accounts/{}/suspension",
            session.account_id
        ))
        .json(&SetSuspended { suspended: true })
        .send()
        .await
        .unwrap()
        .status();
    assert!(suspended.is_success());
    eventually("the node dropping the account", async || {
        node.account(session.account_id)
            .is_none_or(|account| account.connections == 0)
            .then_some(())
    })
    .await;
    // With its only path gone the session ends at the QUIC idle timeout (30 s).
    let mut state = client.subscribe_state();
    tokio::time::timeout(Duration::from_secs(45), async {
        while matches!(*state.borrow_and_update(), ConnectionState::Ready { .. }) {
            state.changed().await.unwrap();
        }
    })
    .await
    .expect("the session outlived the suspension");
    assert!(session.ticket(&PHONE, phone_kind).await.is_err());

    node.shutdown().await;
    control.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_app_and_a_headless_device_sign_in_without_a_password() {
    let relay = paid_relay_nodes(&[NODE]).await;
    relay.create_account(NODE).await;
    let cookie = relay.console_cookie().await;
    let TestRelay {
        control,
        control_url,
        ..
    } = relay;
    let session = account(&control_url).await;
    let browser = reqwest::Client::new();
    let client = ControlClient::new(&control_url).unwrap();

    // The app opens the console; the person allows it there, and the browser
    // goes back to the app's loopback port.
    let login = client.begin_browser_login("laptop").await.unwrap();
    let page = url::Url::parse(login.authorize_url()).unwrap();
    let mut answer: serde_json::Value = page
        .query_pairs()
        .map(|(key, value)| {
            (
                key.into_owned(),
                serde_json::Value::String(value.into_owned()),
            )
        })
        .collect::<serde_json::Map<_, _>>()
        .into();
    answer["allow"] = true.into();
    let finished = tokio::spawn(login.finish());
    let allowed: serde_json::Value = browser
        .post(format!("{control_url}/v1/oauth/authorize"))
        .header("cookie", &cookie)
        .header("x-yttt-web", "1")
        .json(&answer)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let shown = browser
        .get(allowed["redirect"].as_str().unwrap())
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(shown.contains("Signed in"));
    let app = finished.await.unwrap().unwrap();
    assert_eq!(app.account_id, session.account_id);
    assert_eq!(app.account().await.unwrap().email, "owner@example.com");

    // A headless device shows a code that the person confirms.
    let login = client.begin_device_login("nas").await.unwrap();
    assert_eq!(login.verification_uri, format!("{control_url}/device"));
    assert!(login.verification_uri_complete.ends_with(&login.user_code));
    let user_code = login.user_code.clone();
    let finished = tokio::spawn(login.finish());
    let confirmed = browser
        .post(format!("{control_url}/v1/oauth/device/confirm"))
        .header("cookie", &cookie)
        .header("x-yttt-web", "1")
        .json(&serde_json::json!({"user_code": user_code, "allow": true}))
        .send()
        .await
        .unwrap();
    assert!(confirmed.status().is_success());
    let device = finished.await.unwrap().unwrap();
    assert_eq!(device.account_id, session.account_id);

    // Denied is denied.
    let login = client.begin_device_login("nas").await.unwrap();
    let user_code = login.user_code.clone();
    let finished = tokio::spawn(login.finish());
    browser
        .post(format!("{control_url}/v1/oauth/device/confirm"))
        .header("cookie", &cookie)
        .header("x-yttt-web", "1")
        .json(&serde_json::json!({"user_code": user_code, "allow": false}))
        .send()
        .await
        .unwrap();
    assert!(matches!(
        finished.await.unwrap(),
        Err(yttt_client_core::control::ControlError::Denied)
    ));
    control.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_host_moves_to_another_node_and_its_device_follows() {
    let relay = paid_relay_nodes(&[NODE, "local-2"]).await;
    relay.create_account(NODE).await;
    let TestRelay {
        control,
        nodes,
        control_url,
        ..
    } = relay;
    let (first, second) = (&nodes[0], &nodes[1]);
    let second_url = format!("http://{}", second.addr());
    let session = account(&control_url).await;
    let temp = tempfile::tempdir().unwrap();
    let admin = start_host(temp.path()).await;
    remote_access(
        &admin,
        RemoteAccessRequest::SetRelayAccount {
            account: Some(RelayAccount {
                control_url: control_url.clone(),
                account_id: session.account_id,
                session: SecretText(session.token.clone()),
            }),
        },
    )
    .await;
    remote_access(
        &admin,
        RemoteAccessRequest::SetP2p {
            enabled: true,
        },
    )
    .await;
    let host_key = status(&admin).await.p2p.endpoint_id.unwrap();
    let phone_kind = DeviceKind::Client { host: host_key.0 };
    session
        .register_device(&PHONE, "phone", phone_kind)
        .await
        .unwrap();
    let ticket = session.ticket(&PHONE, phone_kind).await.unwrap();
    assert_eq!(ticket.node_id, NODE);
    let phone = IrohEndpoint::bind(EndpointConfig {
        secret: PHONE,
        relay: RelaySetup::Use(ticket.target()),
        alpns: Vec::new(),
        gate: Arc::new(OpenGate),
        direct: DirectPaths::Off,
    })
    .await
    .unwrap();
    let RemoteAccessResponse::PairingInvitation { code, .. } =
        remote_access(&admin, RemoteAccessRequest::StartPairing).await
    else {
        panic!("expected an invitation");
    };
    let code = PairingCode::decode(&code.0).unwrap();
    let pairing = tokio::spawn({
        let phone = phone.clone();
        let relay_url = ticket.relay_url.clone();
        async move { pair(&phone, &code, Some(relay_url), "phone", |_| {}).await }
    });
    let endpoint_id = eventually("the phone asking to pair", async || {
        match status(&admin).await.p2p.pairing {
            Some(PairingState::AwaitingConfirmation { endpoint_id, .. }) => Some(endpoint_id),
            _ => None,
        }
    })
    .await;
    remote_access(
        &admin,
        RemoteAccessRequest::ConfirmPairing {
            endpoint_id,
            preset: DevicePreset::Full,
        },
    )
    .await;
    let mut paired = pairing.await.unwrap().unwrap();
    let client = ClientCore::connect(
        paired.connector(phone.clone()),
        identity("phone", Some(paired.environment_id.clone())),
        paired.credential(PHONE),
    )
    .await
    .unwrap();
    client.request(Request::ListResources).await.unwrap();
    // The phone hears where the Host relays, signed by the Host.
    let mut placement = client.relay_placement();
    let first_placement = eventually("the Host's placement", async || {
        placement.borrow_and_update().clone()
    })
    .await;
    assert_eq!(status(&admin).await.p2p.endpoint_id, Some(host_key));
    assert_eq!(first_placement.verify(&host_key.0).unwrap().node_id, NODE);

    // The Host lists the nodes it may move to, then moves.
    let RemoteAccessResponse::RelayNodes(choices) =
        remote_access(&admin, RemoteAccessRequest::ListRelayNodes).await
    else {
        panic!("expected relay nodes");
    };
    assert_eq!(choices.len(), 2);
    assert!(choices.iter().all(|choice| choice.latency_ms.is_some()));
    remote_access(
        &admin,
        RemoteAccessRequest::SetRelayNode {
            node_id: "local-2".into(),
        },
    )
    .await;
    let account = status(&admin).await.p2p.relay_account.unwrap();
    assert_eq!(account.node_id.as_deref(), Some("local-2"));
    assert_eq!(account.relay_url.as_deref(), Some(second_url.as_str()));
    // Its session through the old node ended with the move; asking again,
    // the phone gets the newer placement the Host signed.
    let moved = session
        .placed_ticket(&PHONE, phone_kind)
        .await
        .unwrap()
        .host_placement
        .expect("the Host's placement");
    assert!(moved.placement.seq > first_placement.placement.seq);
    let moved_signed = moved.clone();
    let moved = moved.verify(&host_key.0).unwrap();
    assert_eq!(
        (moved.node_id.as_str(), moved.relay_url.as_str()),
        ("local-2", second_url.as_str())
    );

    // The old node lets go of both: the Host left, and the phone is refused.
    eventually("the old node dropping the account", async || {
        first
            .account(session.account_id)
            .is_none_or(|account| account.connections == 0)
            .then_some(())
    })
    .await;
    drop(client);
    phone.close().await;

    // Reconnecting, the phone asks again and lands on the Host's new node.
    let ticket = session.ticket(&PHONE, phone_kind).await.unwrap();
    assert_eq!(ticket.node_id, "local-2");
    assert!(paired.accept_placement(&moved_signed).unwrap());
    assert!(
        !paired.accept_placement(&first_placement).unwrap(),
        "an older placement never replaces it"
    );
    let phone = IrohEndpoint::bind(EndpointConfig {
        secret: PHONE,
        relay: RelaySetup::Use(ticket.target()),
        alpns: Vec::new(),
        gate: Arc::new(OpenGate),
        direct: DirectPaths::Off,
    })
    .await
    .unwrap();
    let client = ClientCore::connect(
        paired.connector(phone),
        identity("phone-again", Some(paired.environment_id.clone())),
        paired.credential(PHONE),
    )
    .await
    .unwrap();
    client.request(Request::ListResources).await.unwrap();
    eventually("both on the new node", async || {
        second
            .account(session.account_id)
            .filter(|account| account.connections >= 2)
    })
    .await;
    control.shutdown().await;
}
