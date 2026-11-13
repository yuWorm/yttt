//! A phone on a paid relay: a control plane on PostgreSQL, a relay node that
//! admits only ticket holders, a Host on the account. The phone has no direct
//! paths, so everything goes through the node. Signed out it cannot pair;
//! signed in it pairs, connects and gives its device slot back when it
//! forgets the Host. The harness follows `yttt-client-core/tests/paid_relay.rs`.
//!
//! Needs `YTTT_TEST_DATABASE_URL` (see packaging/relay-dev/docker-compose.full.yml).

use std::{fs, path::Path, time::Duration};

use yttt_client_core::{ClientCore, control::ControlClient};
use yttt_control::{Control, ControlConfig, NodeEntry, PlanConfig};
use yttt_core::model::ids::{ClientInstanceId, ProfileId};
use yttt_mobile_core::account::{self, Account};
use yttt_mobile_core::session::{MobileError, MobileSession, RelayChoice, RelaySource, pair};
use yttt_protocol::{
    BuildIdentity, ProtocolRange, RESOURCE_PROTOCOL_VERSION, Request, Response,
    p2p::DevicePreset,
    remote_access::{
        PairingState, RelayAccount, RemoteAccessRequest, RemoteAccessResponse, RemoteAccessState,
        RemoteAccessStatus, SecretText,
    },
    workspace::WorkspaceProjectConfig,
};
use yttt_relay::{ControlConfig as NodeControl, NodeConfig, RelayNode};
use yttt_relay_ticket::api::CreateAccount;
use yttt_transport::{AuthToken, ClientIdentity, memory_pair};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt as _;

const TOKEN: [u8; 32] = [0x7c; 32];
const PROFILE: &str = "mobile-paid-relay";
const NODE: &str = "local-1";
const SECRET: [u8; 32] = [43; 32];
const EMAIL: &str = "owner@example.com";
const PASSWORD: &str = "correct horse";

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

/// A fresh database, a control plane on it and a node reporting to it.
async fn paid_relay() -> (Control, RelayNode, String) {
    let server = std::env::var("YTTT_TEST_DATABASE_URL")
        .expect("YTTT_TEST_DATABASE_URL names a PostgreSQL server");
    let name = format!("yttt_mobile_paid_relay_test_{}", std::process::id());
    let admin = sqlx::PgPool::connect(&server).await.unwrap();
    let _ = sqlx::raw_sql(&format!("DROP DATABASE IF EXISTS {name}"))
        .execute(&admin)
        .await;
    sqlx::raw_sql(&format!("CREATE DATABASE {name}"))
        .execute(&admin)
        .await
        .unwrap();
    let (prefix, _) = server.rsplit_once('/').unwrap();
    let control_addr = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap();
    let control_url = format!("http://{control_addr}");
    let ticket_secret = [3; 32];
    let mut node_config = NodeConfig::local(
        NODE,
        "127.0.0.1:0".parse().unwrap(),
        &[yttt_relay_ticket::public_key(&ticket_secret)],
    );
    node_config.control = Some(NodeControl::from_parts(
        control_url.clone(),
        format!("{NODE}-token"),
        1,
    ));
    let node = RelayNode::spawn(node_config).await.unwrap();
    let control = Control::spawn(ControlConfig {
        listen: control_addr,
        public_url: None,
        web_root: None,
        relay_binaries_dir: None,
        trusted_proxy: false,
        signup: None,
        database_url: format!("{prefix}/{name}"),
        ticket_secret: yttt_relay_ticket::hex(&ticket_secret),
        secrets_key: None,
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
        nodes: vec![NodeEntry {
            id: NODE.into(),
            url: format!("http://{}", node.addr()),
            token: format!("{NODE}-token"),
        }],
    })
    .await
    .unwrap();
    (control, node, control_url)
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
#[ignore = "needs PostgreSQL in YTTT_TEST_DATABASE_URL"]
async fn a_signed_in_phone_pairs_and_works_through_a_ticket_relay() {
    let (control, node, control_url) = paid_relay().await;
    let relay_url = format!("http://{}", node.addr());
    let created = reqwest::Client::new()
        .post(format!("{control_url}/v1/dev/accounts"))
        .json(&CreateAccount {
            email: EMAIL.into(),
            password: PASSWORD.into(),
            plan: "basic".into(),
            node: NODE.into(),
        })
        .send()
        .await
        .unwrap();
    assert!(created.status().is_success());

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
    eventually("the Host on the node", async || {
        node.account(host_session.account_id)
            .filter(|account| account.connections == 1)
    })
    .await;

    // Signed out, the node turns the phone away and pairing fails.
    let code = invite(&admin).await;
    let refused = tokio::time::timeout(
        Duration::from_secs(60),
        pair(
            &code,
            "iPhone",
            SECRET,
            RelayChoice::OnlyFromHost,
            None,
            |_| {},
        ),
    )
    .await
    .expect("pairing without a ticket should fail, not hang");
    assert!(
        matches!(refused, Err(MobileError::Pairing(_))),
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
                |_| {},
            )
            .await
        }
    });
    confirm(&admin).await;
    let record = pairing.await.unwrap().unwrap();
    assert_eq!(record.host.relay_url.as_deref(), Some(relay_url.as_str()));
    assert_eq!(devices(&account).await, before + 1);

    // It connects the same way and the Host sees the key it derived.
    let phone = MobileSession::connect(
        &record,
        SECRET,
        RelayChoice::OnlyFromHost,
        Some(&account),
        "iPhone",
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
