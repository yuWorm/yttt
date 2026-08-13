#![windows_subsystem = "windows"]

use std::{
    ffi::{OsStr, OsString},
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

use yttt::{
    config::profile::{
        AppProfile, EnvironmentKind, HostConnectPolicy, ProfilePersistence, ProjectConfigPolicy,
    },
    desktop_shell::{DesktopShellClaim, DesktopShellCommand, DesktopShellRuntime},
    host_launcher::{HostLauncher, ProcessRole, process_role, run_host_process},
    model::ids::ProfileId,
};
use yttt_protocol::{LifecycleRequest, LifecycleResponse};

fn desktop_profile() -> AppProfile {
    if let Some(root) = std::env::var_os("YTTT_PROFILE_ROOT") {
        return AppProfile::scoped(
            ProfileId::new("performance"),
            EnvironmentKind::Development,
            ProfilePersistence::Ephemeral,
            PathBuf::from(root),
            ProjectConfigPolicy::Overlay,
            HostConnectPolicy::ProfileDiscovery,
        );
    }
    #[cfg(debug_assertions)]
    {
        let executable =
            std::env::current_exe().unwrap_or_else(|_| PathBuf::from("yttt-development"));
        AppProfile::development_for_executable(&executable)
    }
    #[cfg(not(debug_assertions))]
    {
        AppProfile::production()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DesktopCliCommand {
    HostStatus,
    StartHost,
    StopHost,
    RestartHost,
    ForceStopHost,
    OpenLogs,
}

fn desktop_cli_command(args: &[OsString]) -> Result<Option<DesktopCliCommand>, String> {
    let mut selected = None;
    for (flag, command) in [
        ("--host-status", DesktopCliCommand::HostStatus),
        ("--start-host", DesktopCliCommand::StartHost),
        ("--stop-host", DesktopCliCommand::StopHost),
        ("--restart-host", DesktopCliCommand::RestartHost),
        ("--force-stop-host", DesktopCliCommand::ForceStopHost),
        ("--open-logs", DesktopCliCommand::OpenLogs),
    ] {
        if args.iter().any(|argument| argument == OsStr::new(flag))
            && selected.replace(command).is_some()
        {
            return Err("only one Host lifecycle command may be used at a time".to_string());
        }
    }
    Ok(selected)
}

fn run_desktop_cli(profile: AppProfile, command: DesktopCliCommand) -> i32 {
    if command == DesktopCliCommand::OpenLogs {
        if let Err(error) = fs::create_dir_all(&profile.paths().logs)
            .and_then(|()| yttt::ui::app::platform::reveal_path(&profile.paths().logs))
        {
            eprintln!("failed to open yttt logs: {error}");
            return 1;
        }
        return 0;
    }
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("failed to initialize Host lifecycle runtime: {error}");
            return 1;
        }
    };
    let launcher = match HostLauncher::for_current_executable(profile.clone()) {
        Ok(launcher) => launcher,
        Err(error) => {
            eprintln!("failed to initialize Host launcher: {error}");
            return 1;
        }
    };
    let result = runtime.block_on(async {
        match command {
            DesktopCliCommand::HostStatus => {
                if !host_runtime_artifacts_exist(profile.paths().runtime.as_path()) {
                    return Ok("Host stopped".to_string());
                }
                let mut client = launcher.connect_lifecycle(false).await?;
                match client.request(LifecycleRequest::Status).await? {
                    LifecycleResponse::Status(status) => Ok(format!(
                        "Host {:?}: {} terminals, {} clients, {} projects, {} SSH connections, {} jobs",
                        status.state,
                        status.terminal_count,
                        status.client_count,
                        status.project_count,
                        status.ssh_connection_count,
                        status.agent_count
                    )),
                    response => Err(anyhow::anyhow!(
                        "unexpected Host status response: {response:?}"
                    )),
                }
            }
            DesktopCliCommand::StartHost => {
                let process = launcher.launch_or_attach().await?;
                let disposition = if process.spawned() {
                    "started"
                } else {
                    "already running"
                };
                Ok(format!("Host {disposition}"))
            }
            DesktopCliCommand::StopHost | DesktopCliCommand::RestartHost => {
                if host_runtime_artifacts_exist(profile.paths().runtime.as_path()) {
                    let mut client = launcher.connect_lifecycle(false).await?;
                    match client.request(LifecycleRequest::StopIfIdle).await? {
                        LifecycleResponse::Stopping => {}
                        LifecycleResponse::Busy { blockers } => {
                            return Err(anyhow::anyhow!(
                                "Host is busy with {} blocker(s): {blockers:?}",
                                blockers.len()
                            ));
                        }
                        response => {
                            return Err(anyhow::anyhow!(
                                "unexpected Host stop response: {response:?}"
                            ));
                        }
                    }
                    wait_for_host_artifacts_to_clear(profile.paths().runtime.as_path())?;
                }
                if command == DesktopCliCommand::RestartHost {
                    let process = launcher.launch_or_attach().await?;
                    drop(process);
                    Ok("Host restarted".to_string())
                } else {
                    Ok("Host stopped".to_string())
                }
            }
            DesktopCliCommand::ForceStopHost => {
                if !host_runtime_artifacts_exist(profile.paths().runtime.as_path()) {
                    return Ok("Host stopped".to_string());
                }
                let mut client = launcher.connect_lifecycle(true).await?;
                match client.request(LifecycleRequest::ForceStop).await? {
                    LifecycleResponse::Draining => {
                        wait_for_host_artifacts_to_clear(profile.paths().runtime.as_path())?;
                        Ok("Host force-stopped".to_string())
                    }
                    response => Err(anyhow::anyhow!(
                        "unexpected Host force-stop response: {response:?}"
                    )),
                }
            }
            DesktopCliCommand::OpenLogs => unreachable!(),
        }
    });
    match result {
        Ok(message) => {
            println!("{message}");
            0
        }
        Err(error) => {
            eprintln!("{error}");
            if error.to_string().contains("Host is busy") {
                2
            } else {
                1
            }
        }
    }
}

fn host_runtime_artifacts_exist(runtime_root: &Path) -> bool {
    runtime_root.join("host-ready.json").exists() || runtime_root.join("host.pid").exists()
}

fn wait_for_host_artifacts_to_clear(runtime_root: &Path) -> anyhow::Result<()> {
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while std::time::Instant::now() < deadline {
        if !host_runtime_artifacts_exist(runtime_root) {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    anyhow::bail!("timed out waiting for Host exit")
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
        let mut client = launcher.connect_lifecycle(false).await?;
        client.request(LifecycleRequest::StopIfIdle).await
    });
    match outcome {
        Ok(LifecycleResponse::Stopping) => {
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
        Ok(LifecycleResponse::Busy { blockers }) => {
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
            match desktop_cli_command(&args) {
                Ok(Some(command)) => {
                    std::process::exit(run_desktop_cli(profile, command));
                }
                Ok(None) => {}
                Err(error) => {
                    eprintln!("{error}");
                    std::process::exit(1);
                }
            }
            let project_paths = yttt::ui::app::startup::startup_project_paths();
            let command = if project_paths.is_empty() {
                DesktopShellCommand::Activate
            } else {
                DesktopShellCommand::OpenWindow { project_paths }
            };
            match DesktopShellRuntime::claim_or_forward(&profile, command.clone()) {
                Ok(DesktopShellClaim::Owner(desktop_shell)) => {
                    yttt::ui::app::run(profile, desktop_shell, command);
                }
                Ok(DesktopShellClaim::Forwarded) => {}
                Err(error) => {
                    eprintln!("failed to activate yttt desktop shell: {error}");
                    std::process::exit(1);
                }
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
