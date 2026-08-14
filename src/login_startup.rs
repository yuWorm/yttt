use std::{
    ffi::OsString,
    fs::{self, File, OpenOptions},
    io,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard},
};

use fs2::FileExt as _;

use crate::config::profile::{AppProfile, EnvironmentKind, ProfilePersistence};

#[cfg(any(target_os = "linux", test))]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

const START_HOST_ARG: &str = "--start-host";
const PROFILE_ID_ARG: &str = "--profile-id";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoginStartupStatus {
    Disabled,
    Enabled,
    RequiresApproval,
    Unavailable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoginStartupMethod {
    MacOsServiceManagement,
    WindowsCurrentUserRun,
    LinuxSystemdUser,
    LinuxXdgAutostart,
    Unsupported,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LoginStartupState {
    pub status: LoginStartupStatus,
    pub method: LoginStartupMethod,
}

impl LoginStartupState {
    pub const fn unavailable() -> Self {
        Self {
            status: LoginStartupStatus::Unavailable,
            method: LoginStartupMethod::Unsupported,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum LoginStartupError {
    #[error("explicit user confirmation is required before enabling login startup")]
    ConsentRequired,
    #[error("login startup is available only for the persistent production profile")]
    UnsupportedProfile,
    #[error("login startup failed: {0}")]
    Backend(#[from] io::Error),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoginStartupSpec {
    executable: PathBuf,
    profile_id: String,
    state_root: PathBuf,
    eligible: bool,
    operation_lock_path: PathBuf,
}

impl LoginStartupSpec {
    pub fn new(profile: &AppProfile, executable: impl Into<PathBuf>) -> Self {
        Self {
            executable: executable.into(),
            profile_id: profile.id().as_str().to_string(),
            state_root: profile.paths().state.clone(),
            eligible: profile.environment() == EnvironmentKind::Production
                && profile.persistence() == ProfilePersistence::Persistent,
            operation_lock_path: profile.paths().state.join("login-startup-operation.lock"),
        }
    }

    pub fn executable(&self) -> &Path {
        &self.executable
    }

    pub fn profile_id(&self) -> &str {
        &self.profile_id
    }

    #[cfg(target_os = "macos")]
    fn state_root(&self) -> &Path {
        &self.state_root
    }

    fn operation_lock_path(&self) -> &Path {
        &self.operation_lock_path
    }

    pub fn arguments(&self) -> [OsString; 3] {
        [
            OsString::from(START_HOST_ARG),
            OsString::from(PROFILE_ID_ARG),
            OsString::from(&self.profile_id),
        ]
    }

    fn ensure_eligible(&self) -> Result<(), LoginStartupError> {
        if self.eligible {
            Ok(())
        } else {
            Err(LoginStartupError::UnsupportedProfile)
        }
    }
}

pub trait LoginStartupBackend: Send + Sync {
    fn status(&self, spec: &LoginStartupSpec) -> io::Result<LoginStartupState>;
    fn reconcile(&self, spec: &LoginStartupSpec) -> io::Result<LoginStartupState> {
        self.status(spec)
    }
    fn enable(&self, spec: &LoginStartupSpec) -> io::Result<LoginStartupState>;
    fn disable(&self, spec: &LoginStartupSpec) -> io::Result<LoginStartupState>;
}

#[derive(Clone)]
pub struct LoginStartupManager {
    spec: LoginStartupSpec,
    backend: Arc<dyn LoginStartupBackend>,
    operation_lock: Arc<Mutex<()>>,
}

impl LoginStartupManager {
    pub fn for_current_platform(profile: &AppProfile) -> io::Result<Self> {
        let executable = std::env::current_exe()?;
        Ok(Self::with_backend(
            LoginStartupSpec::new(profile, executable),
            platform_backend(),
        ))
    }

    pub fn with_backend(spec: LoginStartupSpec, backend: Arc<dyn LoginStartupBackend>) -> Self {
        Self {
            spec,
            backend,
            operation_lock: Arc::new(Mutex::new(())),
        }
    }

    pub fn status(&self) -> Result<LoginStartupState, LoginStartupError> {
        if !self.spec.eligible {
            return Ok(LoginStartupState::unavailable());
        }
        let _operation = self.lock_operation()?;
        Ok(self.backend.status(&self.spec)?)
    }

    pub fn reconcile(&self) -> Result<LoginStartupState, LoginStartupError> {
        if !self.spec.eligible {
            return Ok(LoginStartupState::unavailable());
        }
        let _operation = self.lock_operation()?;
        Ok(self.backend.reconcile(&self.spec)?)
    }

    pub fn set_enabled(
        &self,
        enabled: bool,
        user_confirmed: bool,
    ) -> Result<LoginStartupState, LoginStartupError> {
        self.spec.ensure_eligible()?;
        if enabled && !user_confirmed {
            return Err(LoginStartupError::ConsentRequired);
        }
        let _operation = self.lock_operation()?;
        if enabled {
            Ok(self.backend.enable(&self.spec)?)
        } else {
            Ok(self.backend.disable(&self.spec)?)
        }
    }

    fn lock_operation(&self) -> io::Result<LoginStartupOperationLock<'_>> {
        let in_process = self
            .operation_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let parent = self.spec.operation_lock_path().parent().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "lock path has no parent")
        })?;
        fs::create_dir_all(parent)?;
        let mut options = OpenOptions::new();
        options.create(true).read(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        let file = options.open(self.spec.operation_lock_path())?;
        file.lock_exclusive()?;
        Ok(LoginStartupOperationLock {
            _in_process: in_process,
            file,
        })
    }
}

struct LoginStartupOperationLock<'a> {
    _in_process: MutexGuard<'a, ()>,
    file: File,
}

impl Drop for LoginStartupOperationLock<'_> {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

#[cfg(any(target_os = "windows", test))]
fn quote_windows_argument(argument: &str) -> String {
    let mut quoted = String::with_capacity(argument.len() + 2);
    quoted.push('"');
    let mut backslashes = 0;
    for character in argument.chars() {
        match character {
            '\\' => backslashes += 1,
            '"' => {
                quoted.extend(std::iter::repeat_n('\\', backslashes * 2 + 1));
                quoted.push('"');
                backslashes = 0;
            }
            _ => {
                quoted.extend(std::iter::repeat_n('\\', backslashes));
                backslashes = 0;
                quoted.push(character);
            }
        }
    }
    quoted.extend(std::iter::repeat_n('\\', backslashes * 2));
    quoted.push('"');
    quoted
}

fn platform_backend() -> Arc<dyn LoginStartupBackend> {
    #[cfg(target_os = "macos")]
    {
        return Arc::new(macos::MacOsLoginStartupBackend);
    }
    #[cfg(target_os = "windows")]
    {
        return Arc::new(windows::WindowsLoginStartupBackend);
    }
    #[cfg(target_os = "linux")]
    {
        return Arc::new(linux::LinuxLoginStartupBackend::for_current_user());
    }
    #[allow(unreachable_code)]
    Arc::new(UnsupportedLoginStartupBackend)
}

struct UnsupportedLoginStartupBackend;

impl LoginStartupBackend for UnsupportedLoginStartupBackend {
    fn status(&self, _spec: &LoginStartupSpec) -> io::Result<LoginStartupState> {
        Ok(LoginStartupState::unavailable())
    }

    fn enable(&self, _spec: &LoginStartupSpec) -> io::Result<LoginStartupState> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "login startup is unsupported on this platform",
        ))
    }

    fn disable(&self, _spec: &LoginStartupSpec) -> io::Result<LoginStartupState> {
        Ok(LoginStartupState::unavailable())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use crate::{
        config::profile::{
            EnvironmentKind, HostConnectPolicy, ProfilePersistence, ProjectConfigPolicy,
        },
        model::ids::ProfileId,
    };

    use super::*;

    struct FakeBackend {
        calls: Mutex<Vec<&'static str>>,
    }

    impl FakeBackend {
        fn new() -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
            }
        }
    }

    impl LoginStartupBackend for FakeBackend {
        fn status(&self, _spec: &LoginStartupSpec) -> io::Result<LoginStartupState> {
            self.calls.lock().unwrap().push("status");
            Ok(LoginStartupState {
                status: LoginStartupStatus::Disabled,
                method: LoginStartupMethod::LinuxXdgAutostart,
            })
        }

        fn reconcile(&self, _spec: &LoginStartupSpec) -> io::Result<LoginStartupState> {
            self.calls.lock().unwrap().push("reconcile");
            Ok(LoginStartupState {
                status: LoginStartupStatus::Disabled,
                method: LoginStartupMethod::LinuxXdgAutostart,
            })
        }

        fn enable(&self, _spec: &LoginStartupSpec) -> io::Result<LoginStartupState> {
            self.calls.lock().unwrap().push("enable");
            Ok(LoginStartupState {
                status: LoginStartupStatus::Enabled,
                method: LoginStartupMethod::LinuxXdgAutostart,
            })
        }

        fn disable(&self, _spec: &LoginStartupSpec) -> io::Result<LoginStartupState> {
            self.calls.lock().unwrap().push("disable");
            Ok(LoginStartupState {
                status: LoginStartupStatus::Disabled,
                method: LoginStartupMethod::LinuxXdgAutostart,
            })
        }
    }

    fn production_profile(root: &Path) -> AppProfile {
        AppProfile::scoped(
            ProfileId::new("default"),
            EnvironmentKind::Production,
            ProfilePersistence::Persistent,
            root,
            ProjectConfigPolicy::Normal,
            HostConnectPolicy::ProfileDiscovery,
        )
    }

    #[test]
    fn enabling_requires_explicit_confirmation_before_backend_side_effects() {
        let temp = tempfile::tempdir().unwrap();
        let backend = Arc::new(FakeBackend::new());
        let manager = LoginStartupManager::with_backend(
            LoginStartupSpec::new(&production_profile(temp.path()), "/opt/yttt/bin/yttt"),
            backend.clone(),
        );

        assert!(matches!(
            manager.set_enabled(true, false),
            Err(LoginStartupError::ConsentRequired)
        ));
        assert!(backend.calls.lock().unwrap().is_empty());

        let state = manager.set_enabled(true, true).unwrap();
        assert_eq!(state.status, LoginStartupStatus::Enabled);
        assert_eq!(*backend.calls.lock().unwrap(), ["enable"]);
    }

    #[test]
    fn status_and_disable_use_the_injected_backend() {
        let temp = tempfile::tempdir().unwrap();
        let backend = Arc::new(FakeBackend::new());
        let manager = LoginStartupManager::with_backend(
            LoginStartupSpec::new(&production_profile(temp.path()), "/opt/yttt/bin/yttt"),
            backend.clone(),
        );

        assert_eq!(
            manager.status().unwrap().status,
            LoginStartupStatus::Disabled
        );
        assert_eq!(
            manager.set_enabled(false, false).unwrap().status,
            LoginStartupStatus::Disabled
        );
        assert_eq!(*backend.calls.lock().unwrap(), ["status", "disable"]);
    }

    #[test]
    fn reconcile_uses_the_injected_backend() {
        let temp = tempfile::tempdir().unwrap();
        let backend = Arc::new(FakeBackend::new());
        let manager = LoginStartupManager::with_backend(
            LoginStartupSpec::new(&production_profile(temp.path()), "/opt/yttt/bin/yttt"),
            backend.clone(),
        );

        assert_eq!(
            manager.reconcile().unwrap().status,
            LoginStartupStatus::Disabled
        );
        assert_eq!(*backend.calls.lock().unwrap(), ["reconcile"]);
    }

    #[test]
    fn profile_operation_lock_serializes_independent_managers() {
        let temp = tempfile::tempdir().unwrap();
        let spec = LoginStartupSpec::new(&production_profile(temp.path()), "/opt/yttt/bin/yttt");
        let first = LoginStartupManager::with_backend(spec.clone(), Arc::new(FakeBackend::new()));
        let second = LoginStartupManager::with_backend(spec, Arc::new(FakeBackend::new()));
        let held = first.lock_operation().unwrap();
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);

        let thread = std::thread::spawn(move || {
            sender.send(second.status().unwrap()).unwrap();
        });
        assert!(matches!(
            receiver.recv_timeout(std::time::Duration::from_millis(100)),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout)
        ));

        drop(held);
        assert_eq!(
            receiver
                .recv_timeout(std::time::Duration::from_secs(2))
                .unwrap()
                .status,
            LoginStartupStatus::Disabled
        );
        thread.join().unwrap();
    }

    #[test]
    fn registered_arguments_are_stable_profile_scoped_and_secret_free() {
        let temp = tempfile::tempdir().unwrap();
        let spec = LoginStartupSpec::new(
            &production_profile(temp.path()),
            "/Applications/yttt.app/Contents/MacOS/yttt",
        );
        let arguments = spec.arguments();

        assert_eq!(
            arguments,
            ["--start-host", "--profile-id", "default"].map(OsString::from)
        );
        let joined = arguments
            .iter()
            .map(|argument| argument.to_string_lossy())
            .collect::<Vec<_>>()
            .join(" ");
        for secret_name in ["token", "password", "credential", "auth"] {
            assert!(!joined.to_ascii_lowercase().contains(secret_name));
        }
    }

    #[test]
    fn windows_registration_arguments_follow_command_line_to_argv_w_quoting() {
        assert_eq!(quote_windows_argument("plain"), r#""plain""#);
        assert_eq!(
            quote_windows_argument(r"C:\Program Files\yttt.exe"),
            r#""C:\Program Files\yttt.exe""#
        );
        assert_eq!(quote_windows_argument(r#"a\"b"#), r#""a\\\"b""#);
        assert_eq!(quote_windows_argument(r"ends\"), r#""ends\\""#);
    }

    #[test]
    fn non_production_profiles_cannot_register_login_startup() {
        let temp = tempfile::tempdir().unwrap();
        let profile = AppProfile::scoped(
            ProfileId::new("development-test"),
            EnvironmentKind::Development,
            ProfilePersistence::Persistent,
            temp.path(),
            ProjectConfigPolicy::Overlay,
            HostConnectPolicy::ProfileDiscovery,
        );
        let backend = Arc::new(FakeBackend::new());
        let manager = LoginStartupManager::with_backend(
            LoginStartupSpec::new(&profile, "/tmp/yttt"),
            backend.clone(),
        );

        assert_eq!(manager.status().unwrap(), LoginStartupState::unavailable());
        assert!(matches!(
            manager.set_enabled(true, true),
            Err(LoginStartupError::UnsupportedProfile)
        ));
        assert!(backend.calls.lock().unwrap().is_empty());
    }
}
