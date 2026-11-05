//! A device pairs with a real Host over P2P, works, and is revoked.

use std::{fs, path::Path, sync::Arc, time::Duration};

use yttt_client_core::{
    ClientCore, ClientCoreError, ConnectionState,
    p2p::{PairError, PairedHost, pair},
};
use yttt_core::model::ids::{ClientInstanceId, ProfileId};
use yttt_protocol::{
    BuildIdentity, EndpointKey, FailureCode, ProtocolRange, RESOURCE_PROTOCOL_VERSION, Request,
    Response,
    p2p::{DevicePreset, PairRejectReason, PairingCode},
    remote_access::{
        PairingState, RemoteAccessRequest, RemoteAccessResponse, RemoteAccessState,
        RemoteAccessStatus,
    },
    workspace::WorkspaceProjectConfig,
};
use yttt_transport::{AuthToken, ClientIdentity, memory_pair};
use yttt_transport_iroh::{EndpointConfig, IrohEndpoint, OpenGate};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt as _;

const TOKEN: [u8; 32] = [0x5b; 32];
const PROFILE: &str = "p2p-pairing";

fn bootstrap(root: &Path) -> yttt_host::HostBootstrap {
    let runtime_root = root.join("runtime");
    fs::create_dir_all(&runtime_root).unwrap();
    let auth_token_file = runtime_root.join("auth-token");
    fs::write(&auth_token_file, TOKEN).unwrap();
    #[cfg(unix)]
    {
        fs::set_permissions(&runtime_root, fs::Permissions::from_mode(0o700)).unwrap();
        fs::set_permissions(&auth_token_file, fs::Permissions::from_mode(0o600)).unwrap();
    }
    yttt_host::HostBootstrap {
        profile_id: ProfileId::new(PROFILE),
        runtime_root,
        state_root: root.join("state"),
        config_root: root.join("state/config"),
        project_config: WorkspaceProjectConfig::Project,
        auth_token_file,
        ssh_host_keys_file: root.join("ssh-host-keys.toml"),
        credential_namespace: "dev.yttt.ssh.p2p-pairing".to_string(),
        build: build(),
        lifetime: yttt_host::HostLifetime::Independent,
    }
}

fn build() -> BuildIdentity {
    BuildIdentity {
        product_version: "0.3.0".to_string(),
        build_fingerprint: "p2p-pairing".to_string(),
        resource_compatibility: "p2p-pairing-v1".to_string(),
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

/// Starts a Host with P2P enabled and returns its local administrator.
async fn start_host(root: &Path) -> ClientCore {
    let bootstrap = bootstrap(root);
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
    let admin = ClientCore::connect(
        connector,
        identity("admin", None),
        AuthToken::from_bytes(TOKEN),
    )
    .await
    .unwrap();
    admin
        .request(Request::RemoteAccess(RemoteAccessRequest::SetP2p {
            enabled: true,
            relay_url: None,
        }))
        .await
        .unwrap();
    assert_eq!(
        status(&admin).await.p2p.effective,
        RemoteAccessState::Listening
    );
    admin
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

async fn invitation(admin: &ClientCore) -> PairingCode {
    let RemoteAccessResponse::PairingInvitation { code, .. } =
        remote_access(admin, RemoteAccessRequest::StartPairing).await
    else {
        panic!("expected an invitation");
    };
    PairingCode::decode(&code.0).unwrap()
}

async fn device_endpoint(secret: [u8; 32]) -> IrohEndpoint {
    IrohEndpoint::bind(EndpointConfig {
        secret,
        relay: None,
        alpns: Vec::new(),
        gate: Arc::new(OpenGate),
        // Like a real Client: the Host advertises its interface addresses.
        bind_addr: None,
    })
    .await
    .unwrap()
}

/// Waits until a device waits for confirmation and returns its key and short code.
async fn awaiting_confirmation(admin: &ClientCore) -> (EndpointKey, u32) {
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            if let Some(PairingState::AwaitingConfirmation {
                endpoint_id,
                short_code,
                ..
            }) = status(admin).await.p2p.pairing
            {
                return (endpoint_id, short_code);
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("no device asked to pair")
}

/// Pairs `secret` with the Host, the Host user choosing `preset`.
async fn pair_device(
    admin: &ClientCore,
    secret: [u8; 32],
    preset: DevicePreset,
) -> (IrohEndpoint, PairedHost) {
    let code = invitation(admin).await;
    let endpoint = device_endpoint(secret).await;
    let (shown, shown_rx) = tokio::sync::oneshot::channel();
    let device = endpoint.clone();
    let pairing = tokio::spawn(async move {
        pair(&device, &code, "test device", move |short_code| {
            let _ = shown.send(short_code);
        })
        .await
    });
    let (endpoint_id, host_short_code) = awaiting_confirmation(admin).await;
    assert_eq!(endpoint_id, endpoint.id());
    assert_eq!(shown_rx.await.unwrap(), host_short_code);
    remote_access(
        admin,
        RemoteAccessRequest::ConfirmPairing {
            endpoint_id,
            preset,
        },
    )
    .await;
    let paired = pairing.await.unwrap().unwrap();
    (endpoint, paired)
}

async fn connect_device(
    endpoint: IrohEndpoint,
    paired: &PairedHost,
    secret: [u8; 32],
) -> ClientCore {
    ClientCore::connect(
        paired.connector(endpoint),
        identity("device", Some(paired.environment_id.clone())),
        paired.credential(secret),
    )
    .await
    .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_paired_device_works_until_it_is_revoked() {
    let temp = tempfile::tempdir().unwrap();
    let admin = start_host(temp.path()).await;
    let secret = [21; 32];
    let (endpoint, paired) = pair_device(&admin, secret, DevicePreset::Full).await;
    assert_eq!(Some(paired.host), status(&admin).await.p2p.endpoint_id);
    assert_eq!(status(&admin).await.p2p.pairing, None);

    let device = connect_device(endpoint, &paired, secret).await;
    device.request(Request::ListResources).await.unwrap();
    let sessions = status(&admin).await.clients;
    let device_key = yttt_transport_iroh::endpoint_key(&secret);
    assert!(
        sessions
            .iter()
            .any(|session| session.device == Some(device_key))
    );

    let RemoteAccessResponse::Devices(devices) =
        remote_access(&admin, RemoteAccessRequest::ListDevices).await
    else {
        panic!("expected devices");
    };
    assert_eq!(devices.len(), 1);
    assert_eq!(devices[0].label, "test device");

    remote_access(
        &admin,
        RemoteAccessRequest::RevokeDevice {
            endpoint_id: devices[0].endpoint_id,
            reset_tls: false,
        },
    )
    .await;
    let mut state = device.subscribe_state();
    tokio::time::timeout(Duration::from_secs(20), async {
        while matches!(*state.borrow_and_update(), ConnectionState::Ready { .. }) {
            state.changed().await.unwrap();
        }
    })
    .await
    .expect("the revoked device stayed connected");
    assert!(device.request(Request::ListResources).await.is_err());
    assert!(
        status(&admin)
            .await
            .clients
            .iter()
            .all(|session| session.device.is_none())
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_observing_device_cannot_type() {
    let temp = tempfile::tempdir().unwrap();
    let admin = start_host(temp.path()).await;
    let secret = [22; 32];
    let (endpoint, paired) = pair_device(&admin, secret, DevicePreset::Observe).await;
    let device = connect_device(endpoint, &paired, secret).await;

    device.request(Request::ListResources).await.unwrap();
    let denied = device.request(Request::TakeTerminalController).await;
    assert!(
        matches!(&denied, Err(ClientCoreError::Protocol(failure)) if failure.code == FailureCode::PermissionDenied),
        "{denied:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_declined_device_is_not_paired() {
    let temp = tempfile::tempdir().unwrap();
    let admin = start_host(temp.path()).await;
    let code = invitation(&admin).await;
    let endpoint = device_endpoint([23; 32]).await;
    let pairing = tokio::spawn(async move { pair(&endpoint, &code, "declined", |_| {}).await });
    awaiting_confirmation(&admin).await;
    remote_access(&admin, RemoteAccessRequest::CancelPairing).await;

    assert!(matches!(
        pairing.await.unwrap(),
        Err(PairError::Rejected(PairRejectReason::Declined))
    ));
    let RemoteAccessResponse::Devices(devices) =
        remote_access(&admin, RemoteAccessRequest::ListDevices).await
    else {
        panic!("expected devices");
    };
    assert!(devices.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_second_holder_of_the_code_withdraws_the_invitation() {
    let temp = tempfile::tempdir().unwrap();
    let admin = start_host(temp.path()).await;
    let code = invitation(&admin).await;
    let first = device_endpoint([24; 32]).await;
    let first_code = code.clone();
    let first_pairing =
        tokio::spawn(async move { pair(&first, &first_code, "first", |_| {}).await });
    awaiting_confirmation(&admin).await;

    let second = device_endpoint([25; 32]).await;
    let second_result = pair(&second, &code, "second", |_| {}).await;
    assert!(matches!(
        second_result,
        Err(PairError::Rejected(PairRejectReason::InvitationClosed))
    ));
    assert!(matches!(
        first_pairing.await.unwrap(),
        Err(PairError::Rejected(PairRejectReason::InvitationClosed))
    ));
    assert_eq!(
        status(&admin).await.p2p.pairing,
        Some(PairingState::Compromised)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_client_refuses_a_host_that_is_not_the_paired_one() {
    let temp = tempfile::tempdir().unwrap();
    let admin = start_host(temp.path()).await;
    let secret = [26; 32];
    let (endpoint, paired) = pair_device(&admin, secret, DevicePreset::Full).await;
    // The connection reaches the real Host, but the device pinned another Host key.
    let connector = paired.connector(endpoint);
    let mut pinned_elsewhere = paired.clone();
    pinned_elsewhere.host = yttt_transport_iroh::endpoint_key(&[0x42; 32]);
    let started = std::time::Instant::now();
    let connected = ClientCore::connect(
        connector,
        identity("device", Some(paired.environment_id.clone())),
        pinned_elsewhere.credential(secret),
    )
    .await;
    assert!(
        matches!(&connected, Err(ClientCoreError::Connection(_))),
        "{:?}",
        connected.as_ref().err()
    );
    // Refused as an identity failure, not after waiting for a timeout.
    assert!(started.elapsed() < Duration::from_secs(10));
    assert!(
        status(&admin)
            .await
            .clients
            .iter()
            .all(|session| session.device.is_none())
    );
}

/// Needs a relay, e.g. `packaging/relay-dev/docker-compose.yml`, at `YTTT_TEST_RELAY_URL`.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs a local iroh relay"]
async fn a_device_pairs_and_works_with_only_the_relay_as_a_hint() {
    let relay_url = std::env::var("YTTT_TEST_RELAY_URL").expect("set YTTT_TEST_RELAY_URL");
    let temp = tempfile::tempdir().unwrap();
    let admin = start_host(temp.path()).await;
    remote_access(
        &admin,
        RemoteAccessRequest::SetP2p {
            enabled: true,
            relay_url: Some(relay_url.clone()),
        },
    )
    .await;
    let mut code = invitation(&admin).await;
    assert_eq!(code.relay_url.as_deref(), Some(relay_url.as_str()));
    code.direct_addrs.clear();

    let secret = [28; 32];
    let endpoint = IrohEndpoint::bind(EndpointConfig {
        secret,
        relay: Some(yttt_transport_iroh::RelayTarget {
            url: relay_url,
            token: None,
        }),
        alpns: Vec::new(),
        gate: Arc::new(OpenGate),
        bind_addr: None,
    })
    .await
    .unwrap();
    let device = endpoint.clone();
    let pairing = tokio::spawn(async move { pair(&device, &code, "relay device", |_| {}).await });
    let (endpoint_id, _) = awaiting_confirmation(&admin).await;
    remote_access(
        &admin,
        RemoteAccessRequest::ConfirmPairing {
            endpoint_id,
            preset: DevicePreset::Full,
        },
    )
    .await;
    let paired = pairing.await.unwrap().unwrap();
    assert!(paired.direct_addrs.is_empty());

    let client = connect_device(endpoint, &paired, secret).await;
    client.request(Request::ListResources).await.unwrap();
}
