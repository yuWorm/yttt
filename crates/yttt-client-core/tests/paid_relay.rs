//! The whole paid-relay path: a control plane on PostgreSQL, a relay node
//! reporting to it, a Host and a phone on one account. The phone has no direct
//! paths, so the session runs through the node, is billed, and ends when the
//! account is suspended.
//!
//! Needs `YTTT_TEST_DATABASE_URL` (see packaging/relay-dev/docker-compose.full.yml).

use std::{fs, path::Path, sync::Arc, time::Duration};

use yttt_client_core::{
    ClientCore, ConnectionState,
    control::{ControlClient, ControlSession, DeviceKind},
    p2p::pair,
};
use yttt_control::{Control, ControlConfig, NodeEntry, PlanConfig};
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
use yttt_relay::{ControlConfig as NodeControl, NodeConfig, RelayNode};
use yttt_relay_ticket::api::{AccountCreated, CreateAccount, SetSuspended};
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

/// A fresh database on the test server, a control plane on it, and a node
/// reporting to it every second.
async fn paid_relay() -> (Control, RelayNode, String) {
    let (control, mut nodes, control_url) = paid_relay_nodes(&[NODE]).await;
    (control, nodes.remove(0), control_url)
}

/// A control plane and one reporting relay node per id; the first is where
/// accounts start.
async fn paid_relay_nodes(ids: &[&str]) -> (Control, Vec<RelayNode>, String) {
    let server = std::env::var("YTTT_TEST_DATABASE_URL")
        .expect("YTTT_TEST_DATABASE_URL names a PostgreSQL server");
    // Tests in this file run side by side, each on its own database.
    static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let name = format!(
        "yttt_paid_relay_test_{}_{}",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    );
    let admin = sqlx::PgPool::connect(&server).await.unwrap();
    let _ = sqlx::raw_sql(&format!("DROP DATABASE IF EXISTS {name}"))
        .execute(&admin)
        .await;
    sqlx::raw_sql(&format!("CREATE DATABASE {name}"))
        .execute(&admin)
        .await
        .unwrap();
    let (prefix, _) = server.rsplit_once('/').unwrap();

    // The node needs the control URL and the control plane the node URL, so
    // the control plane's port is chosen first.
    let control_addr = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap();
    let control_url = format!("http://{control_addr}");
    let ticket_secret = [3; 32];
    let mut nodes = Vec::new();
    for id in ids {
        let mut node_config = NodeConfig::local(
            *id,
            "127.0.0.1:0".parse().unwrap(),
            &[yttt_relay_ticket::public_key(&ticket_secret)],
        );
        node_config.control = Some(NodeControl::from_parts(
            control_url.clone(),
            format!("{id}-token"),
            1,
        ));
        nodes.push(RelayNode::spawn(node_config).await.unwrap());
    }
    let control = Control::spawn(ControlConfig {
        listen: control_addr,
        public_url: None,
        web_root: None,
        relay_binaries_dir: None,
        trusted_proxy: false,
        signup: None,
        database_url: format!("{prefix}/{name}"),
        ticket_secret: yttt_relay_ticket::hex(&ticket_secret),
        ticket_ttl_secs: 30 * 24 * 3600,
        session_ttl_secs: 3600,
        dev_mode: true,
        oauth: Default::default(),
        plans: vec![PlanConfig {
            id: "basic".into(),
            rate_bps: 5_000_000,
            burst_bytes: 1_000_000,
            monthly_bytes: 1 << 30,
            throttled_rate_bps: 10_000,
            max_devices: 5,
        }],
        nodes: ids
            .iter()
            .zip(&nodes)
            .map(|(id, node)| NodeEntry {
                id: (*id).into(),
                url: format!("http://{}", node.addr()),
                token: format!("{id}-token"),
            })
            .collect(),
    })
    .await
    .unwrap();
    (control, nodes, control_url)
}

async fn account(control_url: &str) -> ControlSession {
    let created: AccountCreated = reqwest::Client::new()
        .post(format!("{control_url}/v1/dev/accounts"))
        .json(&CreateAccount {
            email: "owner@example.com".into(),
            password: "correct horse".into(),
            plan: "basic".into(),
            node: NODE.into(),
        })
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let session = ControlClient::new(control_url)
        .unwrap()
        .login("owner@example.com", "correct horse")
        .await
        .unwrap();
    assert_eq!(session.account_id, created.account_id);
    session
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs PostgreSQL in YTTT_TEST_DATABASE_URL"]
async fn a_paid_relay_carries_billed_sessions_until_the_account_is_suspended() {
    let (control, node, control_url) = paid_relay().await;
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
            relay_url: None,
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
    assert_eq!(code.relay_url.as_deref(), Some(relay_url.as_str()));
    let pairing = tokio::spawn({
        let phone = phone.clone();
        async move { pair(&phone, &code, "phone", |_| {}).await }
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

/// Signs in on the console and returns its cookie.
async fn console(control_url: &str) -> String {
    let response = reqwest::Client::new()
        .post(format!("{control_url}/v1/web/session"))
        .header("x-yttt-web", "1")
        .json(&serde_json::json!({"email": "owner@example.com", "password": "correct horse"}))
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());
    response.headers()["set-cookie"]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string()
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs PostgreSQL in YTTT_TEST_DATABASE_URL"]
async fn the_app_and_a_headless_device_sign_in_without_a_password() {
    let (control, _node, control_url) = paid_relay().await;
    let session = account(&control_url).await;
    let cookie = console(&control_url).await;
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
#[ignore = "needs PostgreSQL in YTTT_TEST_DATABASE_URL"]
async fn a_host_moves_to_another_node_and_its_device_follows() {
    let (control, nodes, control_url) = paid_relay_nodes(&[NODE, "local-2"]).await;
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
            relay_url: None,
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
        async move { pair(&phone, &code, "phone", |_| {}).await }
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
    paired.relay_url = Some(ticket.relay_url.clone());
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
