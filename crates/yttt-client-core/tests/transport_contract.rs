use std::{fs, time::Duration};

use tempfile::TempDir;
use yttt_client_core::ClientCore;
use yttt_core::model::ids::{ClientInstanceId, ProfileId, ProjectId, TerminalSessionId};
use yttt_host::{HostBootstrap, run};
use yttt_protocol::{
    BuildIdentity, ProtocolRange, RESOURCE_PROTOCOL_VERSION, Request, Response,
    terminal::{
        AttachTerminal, TerminalExecutionSpec, TerminalGeometry, TerminalLeaseMode,
        TerminalSpawnSpec,
    },
};
use yttt_transport::{
    AuthToken, ClientIdentity, TransportConnector, TransportListener, memory_pair,
};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt as _;

struct ContractHost<C> {
    _temp: TempDir,
    bootstrap: HostBootstrap,
    token: [u8; 32],
    connector: C,
    task: tokio::task::JoinHandle<Result<(), yttt_host::HostError>>,
}

impl<C> Drop for ContractHost<C> {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn start_host<L, C>(listener: L, connector: C) -> ContractHost<C>
where
    L: TransportListener,
    C: TransportConnector + Clone,
{
    let temp = tempfile::tempdir().unwrap();
    let runtime_root = temp.path().join("runtime");
    fs::create_dir_all(&runtime_root).unwrap();
    #[cfg(unix)]
    fs::set_permissions(&runtime_root, fs::Permissions::from_mode(0o700)).unwrap();
    let auth_token_file = runtime_root.join("auth-token");
    let token = [0x3c; 32];
    fs::write(&auth_token_file, token).unwrap();
    #[cfg(unix)]
    fs::set_permissions(&auth_token_file, fs::Permissions::from_mode(0o600)).unwrap();
    let bootstrap = HostBootstrap {
        profile_id: ProfileId::new("transport-contract"),
        runtime_root,
        auth_token_file,
        ssh_host_keys_file: temp.path().join("ssh-host-keys.toml"),
        credential_namespace: "dev.yttt.ssh.transport-contract".to_string(),
        build: BuildIdentity {
            product_version: "0.2.0".to_string(),
            build_fingerprint: "transport-contract-build".to_string(),
            resource_compatibility: "transport-contract-resource-v1".to_string(),
        },
    };
    let task = tokio::spawn(run(bootstrap.clone(), || async move {
        Ok::<_, yttt_transport::TransportError>(listener)
    }));
    tokio::time::timeout(Duration::from_secs(5), async {
        while !bootstrap.ready_file().exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("Host ready timeout");
    ContractHost {
        _temp: temp,
        bootstrap,
        token,
        connector,
        task,
    }
}

async fn connect_client<C: TransportConnector + Clone>(
    host: &ContractHost<C>,
    id: &str,
) -> ClientCore {
    ClientCore::connect(
        host.connector.clone(),
        ClientIdentity {
            supported: ProtocolRange::exact(RESOURCE_PROTOCOL_VERSION),
            build: host.bootstrap.build.clone(),
            profile_id: host.bootstrap.profile_id.clone(),
            client_instance_id: ClientInstanceId::new(id),
            host_epoch_hint: None,
            can_force_stop: false,
            channel: yttt_protocol::ConnectionChannel::Control,
            terminal_session_id: None,
        },
        AuthToken::from_bytes(host.token),
    )
    .await
    .unwrap()
}

fn contract_spawn_spec() -> TerminalSpawnSpec {
    TerminalSpawnSpec {
        session_id: TerminalSessionId::new("contract-session"),
        project_id: ProjectId::new("contract-project"),
        cwd: yttt_protocol::ProjectRelativePath::root(),
        execution: TerminalExecutionSpec::Command {
            shell: "/bin/sh".to_string(),
            program: "/bin/sh".to_string(),
            args: vec![
                "-lc".to_string(),
                "printf contract-ready; sleep 30".to_string(),
            ],
            return_to_shell: false,
        },
        geometry: TerminalGeometry {
            cols: 80,
            rows: 24,
            cell_width: 8,
            cell_height: 16,
        },
        geometry_epoch: 1,
        query_palette: Vec::new(),
        palette_revision: 1,
        scrollback_limit: 1_000,
        environment: Vec::new(),
        removed_environment: Vec::new(),
    }
}

async fn assert_host_contract<C: TransportConnector + Clone>(host: ContractHost<C>) {
    let owner = connect_client(&host, "contract-owner").await;
    let Response::Resources(catalog) = owner.request(Request::ListResources).await.unwrap() else {
        panic!("ListResources must return a catalog");
    };
    assert_eq!(catalog.profile_id, host.bootstrap.profile_id);
    assert!(catalog.terminals.is_empty());

    let spec = contract_spawn_spec();
    let session_id = spec.session_id.clone();
    let spawned = owner.request(Request::SpawnTerminal(spec)).await.unwrap();
    let Response::TerminalSpawned {
        lease,
        session_epoch,
    } = spawned
    else {
        panic!("unexpected spawn response: {spawned:?}");
    };
    assert_eq!(lease.session_id, session_id);
    assert_eq!(lease.mode, TerminalLeaseMode::Interactive);

    let observer = connect_client(&host, "contract-observer").await;
    let attached = observer
        .request(Request::AttachTerminal(AttachTerminal {
            session_id: session_id.clone(),
            known_session_epoch: Some(session_epoch),
            after_sequence: None,
            mode: TerminalLeaseMode::Observer,
            query_palette: Vec::new(),
            palette_revision: 1,
            geometry: TerminalGeometry {
                cols: 80,
                rows: 24,
                cell_width: 8,
                cell_height: 16,
            },
            geometry_epoch: 1,
        }))
        .await
        .unwrap();
    let Response::TerminalAttached {
        lease: observer_lease,
        checkpoint,
    } = attached
    else {
        panic!("unexpected attach response: {attached:?}");
    };
    assert_eq!(observer_lease.session_id, session_id);
    assert_eq!(observer_lease.mode, TerminalLeaseMode::Observer);
    assert_eq!(checkpoint.viewport.session_id, session_id);

    let Response::Resources(catalog) = observer.request(Request::ListResources).await.unwrap()
    else {
        panic!("observer catalog must be available");
    };
    let placement = catalog
        .terminals
        .iter()
        .find(|placement| placement.session_id == session_id)
        .expect("catalog must include the shared terminal");
    let placement = serde_json::to_value(placement).unwrap();
    assert!(placement.get("tab_id").is_none());
    assert!(placement.get("pane_id").is_none());

    owner.shutdown().await;
    observer.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn memory_transport_satisfies_host_contract() {
    let (listener, connector) = memory_pair();
    let host = start_host(listener, connector).await;
    assert_host_contract(host).await;
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn local_transport_satisfies_host_contract() {
    use yttt_transport_local::{LocalConnector, LocalEndpoint, LocalListener};

    let temp = tempfile::tempdir().unwrap();
    let runtime_root = temp.path().join("runtime");
    fs::create_dir_all(&runtime_root).unwrap();
    fs::set_permissions(&runtime_root, fs::Permissions::from_mode(0o700)).unwrap();
    let auth_token_file = runtime_root.join("auth-token");
    let token = [0x3d; 32];
    fs::write(&auth_token_file, token).unwrap();
    fs::set_permissions(&auth_token_file, fs::Permissions::from_mode(0o600)).unwrap();
    let bootstrap = HostBootstrap {
        profile_id: ProfileId::new("transport-contract-local"),
        runtime_root: runtime_root.clone(),
        auth_token_file,
        ssh_host_keys_file: temp.path().join("ssh-host-keys.toml"),
        credential_namespace: "dev.yttt.ssh.transport-contract-local".to_string(),
        build: BuildIdentity {
            product_version: "0.2.0".to_string(),
            build_fingerprint: "transport-contract-local-build".to_string(),
            resource_compatibility: "transport-contract-resource-v1".to_string(),
        },
    };
    let endpoint = LocalEndpoint::for_profile(bootstrap.profile_id.clone(), runtime_root);
    let connector = LocalConnector::new(endpoint.clone());
    let task = tokio::spawn(run(bootstrap.clone(), || LocalListener::bind(endpoint)));
    tokio::time::timeout(Duration::from_secs(5), async {
        while !bootstrap.ready_file().exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("Host ready timeout");
    let host = ContractHost {
        _temp: temp,
        bootstrap,
        token,
        connector,
        task,
    };
    assert_host_contract(host).await;
}
