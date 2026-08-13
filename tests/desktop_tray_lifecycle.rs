use std::{path::Path, time::Duration};

use tempfile::tempdir;
use yttt::{
    config::profile::{
        AppProfile, EnvironmentKind, HostConnectPolicy, ProfilePersistence, ProjectConfigPolicy,
    },
    host_launcher::HostLauncher,
    host_runtime::DesktopHostRuntime,
    model::ids::{ProfileId, ProjectId},
};
use yttt_protocol::{
    HostBlocker, LifecycleRequest, LifecycleResponse, Request, Response,
    project::{PlatformPath, ProjectRequest, ProjectResponse},
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

#[cfg(unix)]
fn platform_path(path: &Path) -> PlatformPath {
    use std::os::unix::ffi::OsStrExt as _;
    PlatformPath::Unix(path.as_os_str().as_bytes().to_vec())
}

#[cfg(windows)]
fn platform_path(path: &Path) -> PlatformPath {
    use std::os::windows::ffi::OsStrExt as _;
    PlatformPath::Windows(path.as_os_str().encode_wide().collect())
}

#[test]
fn desktop_disconnect_and_stop_if_idle_preserve_active_host_resources() {
    let temporary = tempdir().unwrap();
    let profile = isolated_profile(temporary.path());
    let executable = std::path::PathBuf::from(env!("CARGO_BIN_EXE_yttt"));
    let project_root = temporary.path().join("active-project");
    std::fs::create_dir_all(&project_root).unwrap();
    let project_id = ProjectId::new("tray-active-project");
    let desktop =
        DesktopHostRuntime::start_with_executable(profile.clone(), executable.clone()).unwrap();
    let Response::Project(ProjectResponse::Registered {
        registration_epoch, ..
    }) = desktop
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
    let lifecycle_runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let launcher = HostLauncher::new(profile.clone(), executable.clone());
    let status = lifecycle_runtime.block_on(async {
        let mut lifecycle = launcher.connect_lifecycle(false).await.unwrap();
        lifecycle.request(LifecycleRequest::Status).await.unwrap()
    });
    assert!(matches!(
        status,
        LifecycleResponse::Status(status)
            if status.project_count == 1 && status.client_count == 0
    ));

    let reopened = DesktopHostRuntime::start_with_executable(profile, executable).unwrap();
    assert_eq!(
        reopened
            .request_blocking_typed(Request::Project(ProjectRequest::Close {
                project_id,
                registration_epoch,
            }))
            .unwrap(),
        Response::Project(ProjectResponse::Closed)
    );
    assert_eq!(
        reopened
            .request_lifecycle(LifecycleRequest::ForceStop, true)
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
            .unwrap(),
        LifecycleResponse::Draining
    );
    reopened.shutdown_client();
}
