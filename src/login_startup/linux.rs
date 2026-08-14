use std::{
    fs::{self, OpenOptions},
    io::{self, Write as _},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::Arc,
};

use sha2::{Digest as _, Sha256};

use super::{
    LoginStartupBackend, LoginStartupMethod, LoginStartupSpec, LoginStartupState,
    LoginStartupStatus,
};

pub(super) struct LinuxLoginStartupBackend {
    config_home: Option<PathBuf>,
    systemctl: Arc<dyn SystemctlRunner>,
}

impl LinuxLoginStartupBackend {
    pub(super) fn for_current_user() -> Self {
        Self {
            config_home: xdg_config_home(),
            systemctl: Arc::new(ProcessSystemctlRunner),
        }
    }

    #[cfg(test)]
    fn new(config_home: PathBuf, systemctl: Arc<dyn SystemctlRunner>) -> Self {
        Self {
            config_home: Some(config_home),
            systemctl,
        }
    }

    fn paths(&self, spec: &LoginStartupSpec) -> io::Result<LinuxRegistrationPaths> {
        let config_home = self.config_home.as_ref().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::Unsupported,
                "HOME and XDG_CONFIG_HOME are unavailable",
            )
        })?;
        let id = registration_id(spec.profile_id());
        Ok(LinuxRegistrationPaths {
            systemd_unit: config_home
                .join("systemd/user")
                .join(format!("yttt-host-{id}.service")),
            xdg_desktop: config_home
                .join("autostart")
                .join(format!("yttt-host-{id}.desktop")),
            unit_name: format!("yttt-host-{id}.service"),
        })
    }

    fn systemd_available(&self) -> bool {
        self.systemctl
            .succeeds(&["--user", "show-environment"])
            .unwrap_or(false)
    }

    fn status_with_paths(
        &self,
        spec: &LoginStartupSpec,
        paths: &LinuxRegistrationPaths,
    ) -> io::Result<LoginStartupState> {
        let unit_source = systemd_unit_source(spec)?;
        if paths.systemd_unit.is_file() {
            let matches = fs::read_to_string(&paths.systemd_unit)? == unit_source;
            let enabled = matches
                && self.systemd_available()
                && self.systemctl.succeeds(&[
                    "--user",
                    "is-enabled",
                    "--quiet",
                    &paths.unit_name,
                ])?;
            return Ok(LoginStartupState {
                status: if enabled {
                    LoginStartupStatus::Enabled
                } else {
                    LoginStartupStatus::Disabled
                },
                method: LoginStartupMethod::LinuxSystemdUser,
            });
        }

        let desktop_source = xdg_desktop_source(spec)?;
        if paths.xdg_desktop.is_file() {
            return Ok(LoginStartupState {
                status: if fs::read_to_string(&paths.xdg_desktop)? == desktop_source {
                    LoginStartupStatus::Enabled
                } else {
                    LoginStartupStatus::Disabled
                },
                method: LoginStartupMethod::LinuxXdgAutostart,
            });
        }

        Ok(LoginStartupState {
            status: LoginStartupStatus::Disabled,
            method: if self.systemd_available() {
                LoginStartupMethod::LinuxSystemdUser
            } else {
                LoginStartupMethod::LinuxXdgAutostart
            },
        })
    }
}

impl LoginStartupBackend for LinuxLoginStartupBackend {
    fn status(&self, spec: &LoginStartupSpec) -> io::Result<LoginStartupState> {
        let Some(_) = self.config_home else {
            return Ok(LoginStartupState::unavailable());
        };
        let paths = self.paths(spec)?;
        self.status_with_paths(spec, &paths)
    }

    fn enable(&self, spec: &LoginStartupSpec) -> io::Result<LoginStartupState> {
        let paths = self.paths(spec)?;
        if self.systemd_available() {
            remove_file_if_present(&paths.xdg_desktop)?;
            write_atomic(&paths.systemd_unit, systemd_unit_source(spec)?.as_bytes())?;
            require_systemctl(
                &*self.systemctl,
                &["--user", "daemon-reload"],
                "reload the systemd user manager",
            )?;
            require_systemctl(
                &*self.systemctl,
                &["--user", "enable", "--now", &paths.unit_name],
                "enable the yttt systemd user service",
            )?;
        } else {
            remove_file_if_present(&paths.systemd_unit)?;
            write_atomic(&paths.xdg_desktop, xdg_desktop_source(spec)?.as_bytes())?;
        }
        self.status_with_paths(spec, &paths)
    }

    fn disable(&self, spec: &LoginStartupSpec) -> io::Result<LoginStartupState> {
        let paths = self.paths(spec)?;
        if paths.systemd_unit.exists() && self.systemd_available() {
            require_systemctl(
                &*self.systemctl,
                &["--user", "disable", "--now", &paths.unit_name],
                "disable the yttt systemd user service",
            )?;
        }
        let removed_unit = remove_file_if_present(&paths.systemd_unit)?;
        remove_file_if_present(&paths.xdg_desktop)?;
        if removed_unit && self.systemd_available() {
            require_systemctl(
                &*self.systemctl,
                &["--user", "daemon-reload"],
                "reload the systemd user manager",
            )?;
        }
        self.status_with_paths(spec, &paths)
    }
}

trait SystemctlRunner: Send + Sync {
    fn succeeds(&self, arguments: &[&str]) -> io::Result<bool>;
}

struct ProcessSystemctlRunner;

impl SystemctlRunner for ProcessSystemctlRunner {
    fn succeeds(&self, arguments: &[&str]) -> io::Result<bool> {
        Ok(Command::new("systemctl")
            .args(arguments)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()?
            .success())
    }
}

struct LinuxRegistrationPaths {
    systemd_unit: PathBuf,
    xdg_desktop: PathBuf,
    unit_name: String,
}

fn require_systemctl(
    runner: &dyn SystemctlRunner,
    arguments: &[&str],
    operation: &str,
) -> io::Result<()> {
    if runner.succeeds(arguments)? {
        Ok(())
    } else {
        Err(io::Error::other(format!("failed to {operation}")))
    }
}

fn xdg_config_home() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
                .map(|home| home.join(".config"))
        })
}

fn registration_id(profile_id: &str) -> String {
    let readable = profile_id
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '-'
            }
        })
        .collect::<String>();
    let readable = readable.trim_matches('-');
    let readable = if readable.is_empty() {
        "profile"
    } else {
        readable
    };
    let digest = Sha256::digest(profile_id.as_bytes());
    format!("{readable}-{:x}", digest)[..readable.len() + 1 + 12].to_string()
}

fn systemd_unit_source(spec: &LoginStartupSpec) -> io::Result<String> {
    let command = command_arguments(spec)?
        .into_iter()
        .map(|argument| quote_systemd_argument(&argument))
        .collect::<io::Result<Vec<_>>>()?
        .join(" ");
    // `--start-host` exits after spawning or attaching. Keep the unit active so systemd does not
    // tear down the spawned Host's cgroup. `KillMode=process` lets unregistering stop only the
    // already-exited launcher; the Host remains governed by its resource catalog.
    Ok(format!(
        "[Unit]\nDescription=yttt Host\n\n[Service]\nType=oneshot\nRemainAfterExit=yes\nKillMode=process\nExecStart={command}\n\n[Install]\nWantedBy=default.target\n"
    ))
}

fn xdg_desktop_source(spec: &LoginStartupSpec) -> io::Result<String> {
    let command = command_arguments(spec)?
        .into_iter()
        .map(|argument| quote_desktop_exec_argument(&argument))
        .collect::<io::Result<Vec<_>>>()?
        .join(" ");
    Ok(format!(
        "[Desktop Entry]\nType=Application\nName=yttt Host\nComment=Start the profile-isolated yttt Host after login\nExec={command}\nTerminal=false\nNoDisplay=true\nX-GNOME-Autostart-enabled=true\n"
    ))
}

fn command_arguments(spec: &LoginStartupSpec) -> io::Result<Vec<String>> {
    let arguments = spec.arguments();
    std::iter::once(spec.executable().as_os_str())
        .chain(arguments.iter().map(std::ffi::OsString::as_os_str))
        .map(|argument| {
            argument.to_str().map(str::to_owned).ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "a login startup path or argument is not valid Unicode",
                )
            })
        })
        .collect()
}

fn quote_systemd_argument(argument: &str) -> io::Result<String> {
    reject_line_breaks(argument)?;
    Ok(format!(
        "\"{}\"",
        argument
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('%', "%%")
    ))
}

fn quote_desktop_exec_argument(argument: &str) -> io::Result<String> {
    reject_line_breaks(argument)?;
    Ok(format!(
        "\"{}\"",
        argument
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('`', "\\`")
            .replace('$', "\\$")
            .replace('%', "%%")
    ))
}

fn reject_line_breaks(value: &str) -> io::Result<()> {
    if value.contains(['\n', '\r', '\0']) {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "login startup arguments cannot contain line breaks or NUL bytes",
        ))
    } else {
        Ok(())
    }
}

fn write_atomic(path: &Path, contents: &[u8]) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "path has no parent"))?;
    fs::create_dir_all(parent)?;
    let temporary = parent.join(format!(
        ".{}.{}.tmp",
        path.file_name().unwrap_or_default().to_string_lossy(),
        uuid::Uuid::new_v4()
    ));
    let result = (|| {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)?;
        file.write_all(contents)?;
        file.sync_all()?;
        fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn remove_file_if_present(path: &Path) -> io::Result<bool> {
    match fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::VecDeque, sync::Mutex};

    use crate::{
        config::profile::{
            AppProfile, EnvironmentKind, HostConnectPolicy, ProfilePersistence, ProjectConfigPolicy,
        },
        model::ids::ProfileId,
    };

    use super::*;

    struct FakeSystemctl {
        results: Mutex<VecDeque<bool>>,
        calls: Mutex<Vec<Vec<String>>>,
    }

    impl FakeSystemctl {
        fn with_results(results: impl IntoIterator<Item = bool>) -> Self {
            Self {
                results: Mutex::new(results.into_iter().collect()),
                calls: Mutex::new(Vec::new()),
            }
        }
    }

    impl SystemctlRunner for FakeSystemctl {
        fn succeeds(&self, arguments: &[&str]) -> io::Result<bool> {
            self.calls.lock().unwrap().push(
                arguments
                    .iter()
                    .map(|argument| (*argument).to_string())
                    .collect(),
            );
            Ok(self.results.lock().unwrap().pop_front().unwrap_or(false))
        }
    }

    fn spec(root: &Path) -> LoginStartupSpec {
        let profile = AppProfile::scoped(
            ProfileId::new("default"),
            EnvironmentKind::Production,
            ProfilePersistence::Persistent,
            root,
            ProjectConfigPolicy::Normal,
            HostConnectPolicy::ProfileDiscovery,
        );
        LoginStartupSpec::new(&profile, "/opt/yttt/bin/yttt")
    }

    #[test]
    fn linux_systemd_user_service_keeps_the_spawned_host_alive() {
        let temp = tempfile::tempdir().unwrap();
        let systemctl = Arc::new(FakeSystemctl::with_results([true, true, true, true, true]));
        let backend = LinuxLoginStartupBackend::new(temp.path().to_path_buf(), systemctl.clone());

        let state = backend.enable(&spec(temp.path())).unwrap();

        assert_eq!(state.status, LoginStartupStatus::Enabled);
        assert_eq!(state.method, LoginStartupMethod::LinuxSystemdUser);
        let unit = fs::read_to_string(
            temp.path()
                .join("systemd/user")
                .join(format!("yttt-host-{}.service", registration_id("default"))),
        )
        .unwrap();
        assert!(unit.contains("Type=oneshot"));
        assert!(unit.contains("RemainAfterExit=yes"));
        assert!(unit.contains("KillMode=process"));
        assert!(unit.contains("\"--start-host\" \"--profile-id\" \"default\""));
        assert!(!unit.to_ascii_lowercase().contains("token"));
        assert!(systemctl.calls.lock().unwrap().iter().any(|call| call
            == &[
                "--user",
                "enable",
                "--now",
                &format!("yttt-host-{}.service", registration_id("default"))
            ]));
    }

    #[test]
    fn disabling_systemd_registration_removes_the_unit_and_reloads_the_user_manager() {
        let temp = tempfile::tempdir().unwrap();
        let systemctl = Arc::new(FakeSystemctl::with_results([true; 10]));
        let backend = LinuxLoginStartupBackend::new(temp.path().to_path_buf(), systemctl.clone());
        let spec = spec(temp.path());
        let paths = backend.paths(&spec).unwrap();

        backend.enable(&spec).unwrap();
        let state = backend.disable(&spec).unwrap();

        assert_eq!(state.status, LoginStartupStatus::Disabled);
        assert!(!paths.systemd_unit.exists());
        assert!(
            systemctl
                .calls
                .lock()
                .unwrap()
                .iter()
                .any(|call| call == &["--user", "disable", "--now", &paths.unit_name])
        );
    }

    #[test]
    fn linux_falls_back_to_xdg_autostart_without_systemd_user_manager() {
        let temp = tempfile::tempdir().unwrap();
        let systemctl = Arc::new(FakeSystemctl::with_results([false]));
        let backend = LinuxLoginStartupBackend::new(temp.path().to_path_buf(), systemctl);

        let state = backend.enable(&spec(temp.path())).unwrap();

        assert_eq!(state.status, LoginStartupStatus::Enabled);
        assert_eq!(state.method, LoginStartupMethod::LinuxXdgAutostart);
        let desktop = fs::read_to_string(
            temp.path()
                .join("autostart")
                .join(format!("yttt-host-{}.desktop", registration_id("default"))),
        )
        .unwrap();
        assert!(desktop.contains("NoDisplay=true"));
        assert!(desktop.contains("\"--start-host\" \"--profile-id\" \"default\""));
        assert!(!desktop.to_ascii_lowercase().contains("token"));
    }

    #[test]
    fn disabling_xdg_registration_removes_the_desktop_entry() {
        let temp = tempfile::tempdir().unwrap();
        let systemctl = Arc::new(FakeSystemctl::with_results([false, false]));
        let backend = LinuxLoginStartupBackend::new(temp.path().to_path_buf(), systemctl);
        let spec = spec(temp.path());
        let paths = backend.paths(&spec).unwrap();

        backend.enable(&spec).unwrap();
        let state = backend.disable(&spec).unwrap();

        assert_eq!(state.status, LoginStartupStatus::Disabled);
        assert!(!paths.xdg_desktop.exists());
    }
}
