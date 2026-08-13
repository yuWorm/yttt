#![windows_subsystem = "windows"]

use std::{ffi::OsString, fs, path::PathBuf, time::Duration};

use yttt::{
    config::profile::{
        AppProfile, EnvironmentKind, HostConnectPolicy, ProfilePersistence, ProjectConfigPolicy,
    },
    host_launcher::{HostLauncher, ProcessRole, process_role, run_host_process},
    host_runtime::DesktopHostRuntime,
    model::ids::ProfileId,
};
use yttt_protocol::{Request, Response};

fn desktop_profile() -> AppProfile {
    std::env::var_os("YTTT_PROFILE_ROOT").map_or_else(AppProfile::production, |root| {
        AppProfile::scoped(
            ProfileId::new("performance"),
            EnvironmentKind::Development,
            ProfilePersistence::Ephemeral,
            PathBuf::from(root),
            ProjectConfigPolicy::Overlay,
            HostConnectPolicy::ProfileDiscovery,
        )
    })
}

fn stop_profile_host(profile: AppProfile) -> anyhow::Result<()> {
    let runtime = DesktopHostRuntime::start(profile)?;
    let response = runtime.request_blocking_typed(Request::DrainAndStop)?;
    anyhow::ensure!(
        response == Response::Draining,
        "unexpected Host stop response: {response:?}"
    );
    runtime.shutdown_client();
    Ok(())
}

const INSTALLER_HOST_PREFLIGHT_ARG: &str = "--installer-host-preflight";
const INSTALLER_HOST_PREFLIGHT_TIMEOUT: Duration = Duration::from_secs(10);

fn installer_host_preflight_report(args: &[OsString]) -> Result<Option<PathBuf>, String> {
    let Some(index) = args
        .iter()
        .position(|argument| argument == INSTALLER_HOST_PREFLIGHT_ARG)
    else {
        return Ok(None);
    };
    args.get(index + 1)
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .map(Some)
        .ok_or_else(|| format!("{INSTALLER_HOST_PREFLIGHT_ARG} requires a report path"))
}

fn write_installer_preflight_report(path: &PathBuf, message: &str) {
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = fs::write(path, format!("{message}\n"));
}

fn run_installer_host_preflight(profile: AppProfile, report: PathBuf) -> i32 {
    let runtime_paths = profile.paths().runtime.clone();
    if !runtime_paths.join("host-ready.json").exists() && !runtime_paths.join("host.pid").exists() {
        write_installer_preflight_report(&report, "not-running");
        return 0;
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("failed to initialize installer Host preflight runtime");
    let outcome = runtime.block_on(async {
        let launcher = HostLauncher::for_current_executable(profile)?;
        let mut client = launcher.connect().await?;
        client.request(Request::StopIfIdle).await
    });
    match outcome {
        Ok(Response::HostIdle) => {
            let deadline = std::time::Instant::now() + INSTALLER_HOST_PREFLIGHT_TIMEOUT;
            while std::time::Instant::now() < deadline {
                if !runtime_paths.join("host-ready.json").exists()
                    && !runtime_paths.join("host.pid").exists()
                {
                    write_installer_preflight_report(&report, "stopped");
                    return 0;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            write_installer_preflight_report(&report, "error: timed out waiting for Host exit");
            1
        }
        Ok(Response::HostBusy { blockers }) => {
            write_installer_preflight_report(&report, &format!("busy: {blockers:?}"));
            2
        }
        Ok(response) => {
            write_installer_preflight_report(
                &report,
                &format!("error: unexpected Host response: {response:?}"),
            );
            1
        }
        Err(error) => {
            write_installer_preflight_report(&report, &format!("error: {error}"));
            1
        }
    }
}

fn main() {
    let args: Vec<_> = std::env::args_os().collect();
    match installer_host_preflight_report(&args) {
        Ok(Some(report)) => {
            std::process::exit(run_installer_host_preflight(desktop_profile(), report));
        }
        Ok(None) => {}
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
    match process_role(args.iter()) {
        ProcessRole::Desktop => {
            let profile = desktop_profile();
            if args.iter().any(|arg| arg == "--stop-host") {
                if let Err(error) = stop_profile_host(profile) {
                    eprintln!("failed to stop yttt Host: {error}");
                    std::process::exit(1);
                }
            } else {
                yttt::ui::app::run(profile);
            }
        }
        ProcessRole::Host => {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(1)
                .enable_all()
                .thread_name("yttt-host")
                .build()
                .expect("failed to initialize Host runtime");
            if let Err(error) = runtime.block_on(run_host_process(args)) {
                eprintln!("yttt Host failed: {error}");
                std::process::exit(1);
            }
        }
    }
}
