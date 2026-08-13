use std::fs;

use tempfile::tempdir;
use yttt::{
    config::{
        default_layout::{BuiltinAgent, DefaultLayoutTemplate},
        layout_loader::{ProjectOpenError, export_project_layout},
        profile::{
            AppProfile, EnvironmentKind, HostConnectPolicy, ProfilePersistence, ProjectConfigPolicy,
        },
    },
    model::ids::{ClientInstanceId, HostId, ProfileId, TerminalSessionId},
    runtime::agent_sessions::scan_agent_sessions,
};

#[test]
fn profile_and_host_ids_round_trip_without_mixing_domains() {
    let profile = ProfileId::new("default");
    let host = HostId::new("host-01");
    let client = ClientInstanceId::new("client-01");
    let terminal = TerminalSessionId::new("terminal-01");

    assert_eq!(serde_json::to_string(&profile).unwrap(), "\"default\"");
    assert_eq!(
        serde_json::from_str::<ProfileId>("\"default\"").unwrap(),
        profile
    );
    assert_eq!(host.as_str(), "host-01");
    assert_eq!(client.as_str(), "client-01");
    assert_eq!(terminal.as_str(), "terminal-01");
}

#[test]
fn production_development_and_isolated_profiles_have_disjoint_namespaces() {
    let temp = tempdir().unwrap();
    let production = AppProfile::scoped(
        ProfileId::new("default"),
        EnvironmentKind::Production,
        ProfilePersistence::Persistent,
        temp.path().join("production"),
        ProjectConfigPolicy::Normal,
        HostConnectPolicy::ProfileDiscovery,
    );
    let development = AppProfile::scoped(
        ProfileId::new("dev-a"),
        EnvironmentKind::Development,
        ProfilePersistence::Persistent,
        temp.path().join("development"),
        ProjectConfigPolicy::Overlay,
        HostConnectPolicy::ProfileDiscovery,
    );
    let isolated = AppProfile::scoped(
        ProfileId::new("isolated"),
        EnvironmentKind::Test,
        ProfilePersistence::Ephemeral,
        temp.path().join("isolated"),
        ProjectConfigPolicy::Overlay,
        HostConnectPolicy::ExplicitEndpoint(temp.path().join("isolated.sock")),
    );

    for left in [&production, &development, &isolated] {
        let roots = left.paths().all_roots();
        for (index, root) in roots.iter().enumerate() {
            assert!(
                roots.iter().skip(index + 1).all(|other| root != other),
                "profile roots must not alias: {roots:?}"
            );
        }
    }

    assert_ne!(production.paths().config, development.paths().config);
    assert_ne!(development.paths().runtime, isolated.paths().runtime);
    assert_ne!(production.paths().logs, isolated.paths().logs);
    assert_ne!(
        production.credential_namespace(),
        development.credential_namespace()
    );
    assert_ne!(
        development.credential_namespace(),
        isolated.credential_namespace()
    );
}

#[test]
fn development_builds_use_stable_executable_scoped_runtime_namespaces() {
    let temp = tempdir().unwrap();
    let first = AppProfile::development_for_executable(&temp.path().join("worktree-a/yttt"));
    let first_again = AppProfile::development_for_executable(&temp.path().join("worktree-a/yttt"));
    let second = AppProfile::development_for_executable(&temp.path().join("worktree-b/yttt"));
    let production = AppProfile::production();

    assert_eq!(first.id(), first_again.id());
    assert_eq!(first.paths(), first_again.paths());
    assert_eq!(first.environment(), EnvironmentKind::Development);
    assert_eq!(first.persistence(), ProfilePersistence::Persistent);
    assert_ne!(first.id(), second.id());
    assert_ne!(first.paths().runtime, second.paths().runtime);
    assert_ne!(first.paths().runtime, production.paths().runtime);
    assert_ne!(
        first.credential_namespace(),
        production.credential_namespace()
    );
}

#[test]
fn explicit_endpoint_never_discovers_or_spawns_a_profile_host() {
    let discovery = HostConnectPolicy::ProfileDiscovery;
    let explicit = HostConnectPolicy::ExplicitEndpoint("/tmp/yttt-test.sock".into());

    assert!(discovery.allows_discovery());
    assert!(discovery.allows_auto_spawn());
    assert!(!explicit.allows_discovery());
    assert!(!explicit.allows_auto_spawn());
}

#[test]
fn profile_debug_does_not_expose_runtime_or_endpoint_paths() {
    let temp = tempdir().unwrap();
    let secret_marker = "capability-secret-location";
    let profile = AppProfile::scoped(
        ProfileId::new("isolated"),
        EnvironmentKind::Test,
        ProfilePersistence::Ephemeral,
        temp.path().join(secret_marker),
        ProjectConfigPolicy::Overlay,
        HostConnectPolicy::ExplicitEndpoint(temp.path().join(secret_marker).join("host.sock")),
    );

    let debug = format!("{profile:?}");
    assert!(!debug.contains(secret_marker));
    assert!(debug.contains("isolated"));
}

#[test]
fn isolated_profile_overlays_project_config_and_agent_roots() {
    let temp = tempdir().unwrap();
    let project = temp.path().join("production-project");
    let production_layout = project.join(".yttt/layout.toml");
    fs::create_dir_all(production_layout.parent().unwrap()).unwrap();
    fs::write(&production_layout, "production sentinel").unwrap();

    let production_agent_root = temp
        .path()
        .join("production-home/.codex/sessions/2026/08/12");
    fs::create_dir_all(&production_agent_root).unwrap();
    fs::write(
        production_agent_root.join("rollout.jsonl"),
        format!(
            "{{\"type\":\"session_meta\",\"payload\":{{\"id\":\"production-session\",\"cwd\":\"{}\"}}}}\n",
            project.display()
        ),
    )
    .unwrap();

    let profile = AppProfile::scoped(
        ProfileId::new("isolated"),
        EnvironmentKind::Test,
        ProfilePersistence::Ephemeral,
        temp.path().join("isolated-profile"),
        ProjectConfigPolicy::Overlay,
        HostConnectPolicy::ExplicitEndpoint(temp.path().join("isolated.sock")),
    );
    let config_paths = profile.config_paths();
    let layout = DefaultLayoutTemplate::builtin().materialize("isolated");

    let written = export_project_layout(&config_paths, &project, &layout).unwrap();
    assert!(written.starts_with(profile.paths().state.join("project-config-overlay")));
    assert_eq!(
        fs::read_to_string(&production_layout).unwrap(),
        "production sentinel"
    );

    let sessions = scan_agent_sessions(
        &[BuiltinAgent::Codex],
        &project,
        config_paths.agent_session_access(),
    )
    .unwrap();
    assert!(sessions.is_empty());
}

#[test]
fn read_only_profile_rejects_project_config_writes() {
    let temp = tempdir().unwrap();
    let project = temp.path().join("project");
    fs::create_dir_all(&project).unwrap();
    let profile = AppProfile::scoped(
        ProfileId::new("read-only"),
        EnvironmentKind::Test,
        ProfilePersistence::Ephemeral,
        temp.path().join("read-only-profile"),
        ProjectConfigPolicy::ReadOnly,
        HostConnectPolicy::ExplicitEndpoint(temp.path().join("read-only.sock")),
    );
    let layout = DefaultLayoutTemplate::builtin().materialize("read-only");

    let error = export_project_layout(&profile.config_paths(), &project, &layout).unwrap_err();
    assert!(matches!(
        error,
        ProjectOpenError::ProjectConfigReadOnly { .. }
    ));
    assert!(!project.join(".yttt").exists());
}
