use yttt_core::model::ids::{ClientInstanceId, HostId, ProfileId, ProjectId, TerminalSessionId};
use yttt_protocol::{
    HostPath, ProjectRelativePath, ResourceCatalog, TerminalPlacement,
    project::{ProjectFileContent, ProjectFileFingerprint, ProjectRequest, ProjectResponse},
    ssh::StoredSshCredential,
    terminal::{TerminalExecutionSpec, TerminalGeometry, TerminalSpawnSpec},
};

fn geometry() -> TerminalGeometry {
    TerminalGeometry {
        cols: 80,
        rows: 24,
        cell_width: 8,
        cell_height: 16,
    }
}

fn spawn_spec() -> TerminalSpawnSpec {
    TerminalSpawnSpec {
        session_id: TerminalSessionId::new("session"),
        project_id: ProjectId::new("project"),
        cwd: ProjectRelativePath::root(),
        execution: TerminalExecutionSpec::Command {
            shell: "/bin/sh".to_string(),
            program: "/bin/sh".to_string(),
            args: vec!["-lc".to_string(), "true".to_string()],
            return_to_shell: false,
        },
        geometry: geometry(),
        geometry_epoch: 1,
        query_palette: Vec::new(),
        palette_revision: 1,
        scrollback_limit: 1_000,
        environment: Vec::new(),
        removed_environment: Vec::new(),
    }
}

#[test]
fn spawn_address_ignores_client_layout_and_uses_project_cwd_and_execution() {
    let left = spawn_spec();
    let mut right = spawn_spec();
    right.session_id = TerminalSessionId::new("other-session");
    assert_eq!(left.address_fingerprint(), right.address_fingerprint());

    right.cwd = ProjectRelativePath::from_utf8("src").unwrap();
    assert_ne!(left.address_fingerprint(), right.address_fingerprint());
}

#[test]
fn client_visible_resource_messages_omit_layout_ids_and_host_absolute_paths() {
    let spec = serde_json::to_value(spawn_spec()).unwrap();
    assert!(spec.get("tab_id").is_none());
    assert!(spec.get("pane_id").is_none());

    let placement = serde_json::to_value(TerminalPlacement {
        session_id: TerminalSessionId::new("session"),
        session_epoch: 1,
        project_id: ProjectId::new("project"),
        geometry: geometry(),
        last_sequence: 0,
        spawn_fingerprint: 1,
        owner: Some(ClientInstanceId::new("client")),
        viewport: None,
    })
    .unwrap();
    assert!(placement.get("tab_id").is_none());
    assert!(placement.get("pane_id").is_none());

    let registered = serde_json::to_value(ProjectResponse::Registered {
        registration_epoch: 1,
        watch_error: None,
        null_device: "/dev/null".to_string(),
    })
    .unwrap();
    assert!(registered.get("canonical_root").is_none());

    let file = serde_json::to_value(ProjectFileContent {
        relative_path: ProjectRelativePath::from_utf8("README.md").unwrap(),
        text: String::new(),
        fingerprint: ProjectFileFingerprint::default(),
    })
    .unwrap();
    assert!(file.get("canonical_path").is_none());

    let credential = serde_json::to_value(StoredSshCredential {
        id: "cred-1".to_string(),
        effective_user: "dev".to_string(),
        private_key_identity: None,
    })
    .unwrap();
    assert!(credential.get("resolved_host").is_none());
    assert!(credential.get("host_key_sha256").is_none());
    assert!(credential.get("port").is_none());

    let catalog = serde_json::to_value(ResourceCatalog {
        profile_id: ProfileId::new("profile"),
        host_id: HostId::new("host"),
        host_epoch: 1,
        revision: 1,
        terminals: Vec::new(),
        ssh_connections: vec!["ssh-1".to_string()],
        projects: Vec::new(),
    })
    .unwrap();
    assert_eq!(
        catalog
            .get("ssh_connections")
            .and_then(|value| value.as_array()),
        Some(&vec![serde_json::json!("ssh-1")])
    );
}

#[test]
fn register_uses_host_path_segments_instead_of_platform_split() {
    let root = HostPath::from_path(&std::env::temp_dir()).unwrap();
    let request = serde_json::to_value(ProjectRequest::Register {
        project_id: ProjectId::new("project"),
        root,
    })
    .unwrap();
    let root = request.get("Register").and_then(|value| value.get("root"));
    assert!(root.and_then(|value| value.get("Unix")).is_none());
    assert!(root.and_then(|value| value.get("Windows")).is_none());
    assert!(root.and_then(|value| value.get("segments")).is_some());
}
