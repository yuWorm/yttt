use std::{path::Path, time::Duration};

use tempfile::tempdir;
use yttt::{
    config::profile::{
        AppProfile, EnvironmentKind, HostConnectPolicy, ProfilePersistence, ProjectConfigPolicy,
    },
    desktop_shell::{DesktopShellClaim, DesktopShellCommand, DesktopShellRuntime},
    model::ids::ProfileId,
};

fn isolated_profile(root: &Path) -> AppProfile {
    let id = ProfileId::new(format!("desktop-shell-{}", uuid::Uuid::new_v4()));
    AppProfile::scoped(
        id,
        EnvironmentKind::Test,
        ProfilePersistence::Ephemeral,
        root,
        ProjectConfigPolicy::Overlay,
        HostConnectPolicy::ExplicitEndpoint(root.join("runtime/host.sock")),
    )
}

#[test]
fn second_desktop_forwards_open_window_to_the_single_shell_owner() {
    let temp = tempdir().unwrap();
    let profile = isolated_profile(temp.path());
    let owner = match DesktopShellRuntime::claim_or_forward(&profile, DesktopShellCommand::Activate)
        .unwrap()
    {
        DesktopShellClaim::Owner(owner) => owner,
        DesktopShellClaim::Forwarded => panic!("first desktop did not own its shell endpoint"),
    };
    let project = temp.path().join("forwarded-project");

    let second = DesktopShellRuntime::claim_or_forward(
        &profile,
        DesktopShellCommand::OpenWindow {
            project_paths: vec![project.clone()],
        },
    )
    .unwrap();

    assert!(matches!(second, DesktopShellClaim::Forwarded));
    assert_eq!(
        owner
            .commands()
            .recv_timeout(Duration::from_secs(2))
            .unwrap(),
        DesktopShellCommand::OpenWindow {
            project_paths: vec![project],
        }
    );
}
