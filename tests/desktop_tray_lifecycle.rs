use std::{path::Path, time::Duration};

use tempfile::tempdir;
use yttt::{
    config::profile::{
        AppProfile, EnvironmentKind, HostConnectPolicy, ProfilePersistence, ProjectConfigPolicy,
    },
    host_runtime::DesktopHostRuntime,
    model::ids::{ProfileId, ProjectId},
};
use yttt_protocol::{
    HostBlocker, HostPath, LifecycleRequest, LifecycleResponse, Request, Response,
    project::{ProjectRequest, ProjectResponse},
};

fn isolated_profile(root: &Path) -> AppProfile {
    AppProfile::scoped(
        ProfileId::new(format!("tray-lifecycle-{}", uuid::Uuid::new_v4())),
        EnvironmentKind::Test,
        ProfilePersistence::Ephemeral,
        root,
        ProjectConfigPolicy::Overlay,
        HostConnectPolicy::ExplicitEndpoint(root.join("runtime/host.sock")),
    )
}

fn platform_path(path: &Path) -> HostPath {
    HostPath::from_path(path).expect("test project root must be absolute")
}
fn wait_for_host_exit(profile: &AppProfile) {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while std::time::Instant::now() < deadline
        && (profile.paths().runtime.join("host-ready.json").exists()
            || profile.paths().runtime.join("host.pid").exists())
    {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(!profile.paths().runtime.join("host-ready.json").exists());
    assert!(!profile.paths().runtime.join("host.pid").exists());
}

#[test]
fn desktop_disconnect_stops_owned_host_even_with_active_resources() {
    let temporary = tempdir().unwrap();
    let profile = isolated_profile(temporary.path());
    let executable = std::path::PathBuf::from(env!("CARGO_BIN_EXE_yttt"));
    let project_root = temporary.path().join("active-project");
    std::fs::create_dir_all(&project_root).unwrap();
    let project_id = ProjectId::new("tray-active-project");
    let desktop =
        DesktopHostRuntime::start_with_executable(profile.clone(), executable.clone()).unwrap();
    let ready =
        yttt_host::read_ready_metadata(&profile.paths().runtime.join("host-ready.json")).unwrap();
    assert_eq!(ready.lifetime, yttt_host::HostLifetime::DesktopOwned);
    let Response::Project(ProjectResponse::Registered { .. }) = desktop
        .request_blocking_typed(Request::Project(ProjectRequest::Register {
            project_id: project_id.clone(),
            root: platform_path(&project_root),
        }))
        .unwrap()
    else {
        panic!("Host did not register the tray lifecycle test project");
    };

    let stop = desktop
        .request_lifecycle(LifecycleRequest::StopIfIdle, false)
        .recv_timeout(Duration::from_secs(5))
        .unwrap()
        .unwrap();
    assert!(matches!(
        stop,
        LifecycleResponse::Busy { blockers }
            if blockers.contains(&HostBlocker::Project(project_id.clone()))
    ));

    desktop.shutdown_client();
    drop(desktop);
    wait_for_host_exit(&profile);

    let reopened = DesktopHostRuntime::start_with_executable(profile.clone(), executable).unwrap();
    let Response::Project(ProjectResponse::Registered {
        registration_epoch, ..
    }) = reopened
        .request_blocking_typed(Request::Project(ProjectRequest::Register {
            project_id: project_id.clone(),
            root: platform_path(&project_root),
        }))
        .unwrap()
    else {
        panic!("replacement Host did not register the project");
    };
    assert_eq!(
        reopened
            .request_blocking_typed(Request::Project(ProjectRequest::Close {
                project_id,
                registration_epoch,
            }))
            .unwrap(),
        Response::Project(ProjectResponse::Closed)
    );
    reopened.shutdown_client();
    drop(reopened);
    wait_for_host_exit(&profile);
}
