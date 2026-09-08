use std::{
    fs, io,
    path::{Path, PathBuf},
};

use super::profile::{AgentSessionAccess, AppProfile, EnvironmentKind, ProjectConfigPolicy};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectConfigStore {
    policy: ProjectConfigPolicy,
    overlay_root: PathBuf,
}

impl ProjectConfigStore {
    fn new(policy: ProjectConfigPolicy, overlay_root: PathBuf) -> Self {
        Self {
            policy,
            overlay_root,
        }
    }

    pub fn policy(&self) -> ProjectConfigPolicy {
        self.policy
    }

    fn project_layout_file(&self, project_path: &Path) -> PathBuf {
        match self.policy {
            ProjectConfigPolicy::Normal | ProjectConfigPolicy::ReadOnly => {
                project_path.join(".yttt").join("layout.toml")
            }
            ProjectConfigPolicy::Overlay => {
                let project_path =
                    canonicalize_path(project_path).unwrap_or_else(|_| project_path.to_path_buf());
                self.overlay_root
                    .join(encode_path(&project_path))
                    .join("layout.toml")
            }
        }
    }

    fn project_layout_write_file(&self, project_path: &Path) -> Option<PathBuf> {
        (self.policy != ProjectConfigPolicy::ReadOnly)
            .then(|| self.project_layout_file(project_path))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppConfigPaths {
    config_dir: PathBuf,
    environment: EnvironmentKind,
    project_config: ProjectConfigStore,
    agent_sessions: AgentSessionAccess,
    profile: Option<AppProfile>,
}

impl AppConfigPaths {
    pub fn from_config_dir(config_dir: impl Into<PathBuf>) -> Self {
        let config_dir = config_dir.into();
        super::storage::allow_test_root(&config_dir);
        Self {
            project_config: ProjectConfigStore::new(
                ProjectConfigPolicy::Normal,
                config_dir.join("project-config-overlay"),
            ),
            config_dir,
            environment: EnvironmentKind::Test,
            agent_sessions: AgentSessionAccess::disabled(),
            profile: None,
        }
    }

    pub fn for_app() -> Self {
        AppProfile::production().config_paths()
    }

    pub fn from_profile(profile: &AppProfile) -> Self {
        if profile.environment() == EnvironmentKind::Test {
            super::storage::allow_test_root(&profile.paths().config);
            super::storage::allow_test_root(&profile.paths().state);
        }
        Self {
            config_dir: profile.paths().config.clone(),
            environment: profile.environment(),
            project_config: ProjectConfigStore::new(
                profile.project_config_policy(),
                profile.paths().state.join("project-config-overlay"),
            ),
            agent_sessions: profile.agent_sessions().clone(),
            profile: Some(profile.clone()),
        }
    }

    pub fn profile(&self) -> Option<&AppProfile> {
        self.profile.as_ref()
    }

    pub fn config_dir(&self) -> &Path {
        &self.config_dir
    }

    pub fn environment(&self) -> EnvironmentKind {
        self.environment
    }

    pub fn project_config_policy(&self) -> ProjectConfigPolicy {
        self.project_config.policy()
    }

    pub fn agent_session_access(&self) -> &AgentSessionAccess {
        &self.agent_sessions
    }

    pub fn project_layout_file(&self, project_path: &Path) -> PathBuf {
        if self.environment == EnvironmentKind::Test {
            super::storage::allow_test_root(project_path);
        }
        self.project_config.project_layout_file(project_path)
    }

    pub fn project_layout_write_file(&self, project_path: &Path) -> Option<PathBuf> {
        if self.environment == EnvironmentKind::Test {
            super::storage::allow_test_root(project_path);
        }
        self.project_config.project_layout_write_file(project_path)
    }

    pub fn default_layout_file(&self) -> PathBuf {
        self.config_dir.join("default-layout.toml")
    }

    pub fn local_project_dir(&self, project_path: &Path) -> PathBuf {
        if self.environment == EnvironmentKind::Test {
            super::storage::allow_test_root(project_path);
        }
        let project_path =
            canonicalize_path(project_path).unwrap_or_else(|_| project_path.to_path_buf());
        self.config_dir
            .join("projects")
            .join(encode_path(&project_path))
    }

    pub fn local_layout_file(&self, project_path: &Path) -> PathBuf {
        self.local_project_dir(project_path).join("layout.toml")
    }

    pub fn recent_projects_file(&self) -> PathBuf {
        self.config_dir.join("recent-projects.toml")
    }

    pub fn ssh_connections_file(&self) -> PathBuf {
        self.config_dir.join("ssh-connections.toml")
    }

    pub fn ssh_host_keys_file(&self) -> PathBuf {
        self.config_dir.join("ssh-host-keys.toml")
    }

    pub fn keybindings_file(&self) -> PathBuf {
        self.config_dir.join("keybindings.toml")
    }

    pub fn bars_file(&self) -> PathBuf {
        self.config_dir.join("bars.toml")
    }

    pub fn settings_file(&self) -> PathBuf {
        self.config_dir.join("settings.toml")
    }

    pub fn update_state_file(&self) -> PathBuf {
        self.config_dir.join("update-state.toml")
    }

    pub fn terminal_placements_file(&self) -> PathBuf {
        self.config_dir.join("terminal-placements.json")
    }

    pub fn themes_dir(&self) -> PathBuf {
        self.config_dir.join("themes")
    }

    pub fn icon_themes_dir(&self) -> PathBuf {
        self.themes_dir().join("icons")
    }

    pub fn agent_provider_dir(&self, provider: &str) -> PathBuf {
        self.config_dir.join("agent-providers").join(provider)
    }

    pub fn agent_state_path(&self) -> PathBuf {
        self.config_dir.join("agent-state.json")
    }
}

pub fn canonicalize_path(path: &Path) -> io::Result<PathBuf> {
    super::storage::canonicalize(path)
}

pub fn display_path(path: &Path) -> String {
    windows_path_for_display(dunce::simplified(path).to_string_lossy().as_ref())
}

fn windows_path_for_display(path: &str) -> String {
    if let Some(path) = path.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{path}")
    } else {
        path.strip_prefix(r"\\?\").unwrap_or(path).to_string()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ConfigPlatform {
    MacOs,
    Windows,
    Linux,
}

impl ConfigPlatform {
    fn current() -> Self {
        if cfg!(target_os = "macos") {
            Self::MacOs
        } else if cfg!(target_os = "windows") {
            Self::Windows
        } else {
            Self::Linux
        }
    }
}

pub(crate) fn native_config_dir() -> PathBuf {
    let xdg_config_home = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from);
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let user_profile = std::env::var_os("USERPROFILE").map(PathBuf::from);
    let current_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let native = native_config_dir_from_parts(
        ConfigPlatform::current(),
        xdg_config_home.clone(),
        home.clone(),
        user_profile.clone(),
        std::env::var_os("APPDATA").map(PathBuf::from),
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from),
        current_dir.clone(),
    );
    let legacy = legacy_config_dir_from_parts(xdg_config_home, home.or(user_profile), current_dir);

    migrate_legacy_config_dir(native, legacy)
}

fn native_config_dir_from_parts(
    platform: ConfigPlatform,
    xdg_config_home: Option<PathBuf>,
    home: Option<PathBuf>,
    user_profile: Option<PathBuf>,
    app_data: Option<PathBuf>,
    local_app_data: Option<PathBuf>,
    current_dir: PathBuf,
) -> PathBuf {
    if let Some(config_home) = non_empty_path(xdg_config_home) {
        return config_home.join("yttt");
    }

    match platform {
        ConfigPlatform::MacOs => non_empty_path(home)
            .map(|home| {
                home.join("Library")
                    .join("Application Support")
                    .join("yttt")
            })
            .unwrap_or_else(|| current_dir.join(".yttt")),
        ConfigPlatform::Windows => non_empty_path(app_data)
            .or_else(|| non_empty_path(local_app_data))
            .or_else(|| {
                non_empty_path(user_profile).map(|home| home.join("AppData").join("Roaming"))
            })
            .or_else(|| non_empty_path(home).map(|home| home.join("AppData").join("Roaming")))
            .map(|config_home| config_home.join("yttt"))
            .unwrap_or_else(|| current_dir.join(".yttt")),
        ConfigPlatform::Linux => non_empty_path(home)
            .map(|home| home.join(".config").join("yttt"))
            .unwrap_or_else(|| current_dir.join(".yttt")),
    }
}

fn legacy_config_dir_from_parts(
    xdg_config_home: Option<PathBuf>,
    home: Option<PathBuf>,
    current_dir: PathBuf,
) -> PathBuf {
    if let Some(config_home) = non_empty_path(xdg_config_home) {
        return config_home.join("yttt");
    }
    non_empty_path(home)
        .map(|home| home.join(".config").join("yttt"))
        .unwrap_or_else(|| current_dir.join(".yttt"))
}

fn non_empty_path(path: Option<PathBuf>) -> Option<PathBuf> {
    path.filter(|path| !path.as_os_str().is_empty())
}

fn migrate_legacy_config_dir(native: PathBuf, legacy: PathBuf) -> PathBuf {
    if native == legacy || native.exists() || !legacy.exists() {
        return native;
    }

    let Some(parent) = native.parent() else {
        return legacy;
    };
    if fs::create_dir_all(parent).is_ok() && fs::rename(&legacy, &native).is_ok() {
        native
    } else {
        legacy
    }
}

fn encode_path(path: &Path) -> String {
    let value = path.to_string_lossy();
    let mut encoded = String::new();

    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.') {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02x}"));
        }
    }

    encoded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xdg_config_home_overrides_platform_defaults() {
        let dir = native_config_dir_from_parts(
            ConfigPlatform::Windows,
            Some(PathBuf::from("/tmp/xdg")),
            Some(PathBuf::from("/home/example")),
            Some(PathBuf::from("C:/Users/example")),
            Some(PathBuf::from("C:/Users/example/AppData/Roaming")),
            None,
            PathBuf::from("/repo"),
        );

        assert_eq!(dir, PathBuf::from("/tmp/xdg/yttt"));
    }

    #[test]
    fn macos_uses_application_support() {
        let dir = native_config_dir_from_parts(
            ConfigPlatform::MacOs,
            None,
            Some(PathBuf::from("/Users/example")),
            None,
            None,
            None,
            PathBuf::from("/repo"),
        );

        assert_eq!(
            dir,
            PathBuf::from("/Users/example/Library/Application Support/yttt")
        );
    }

    #[test]
    fn windows_prefers_roaming_app_data() {
        let dir = native_config_dir_from_parts(
            ConfigPlatform::Windows,
            None,
            None,
            Some(PathBuf::from("C:/Users/example")),
            Some(PathBuf::from("C:/Users/example/AppData/Roaming")),
            Some(PathBuf::from("C:/Users/example/AppData/Local")),
            PathBuf::from("C:/repo"),
        );

        assert_eq!(dir, PathBuf::from("C:/Users/example/AppData/Roaming/yttt"));
    }

    #[test]
    fn linux_uses_home_config_without_xdg() {
        let dir = native_config_dir_from_parts(
            ConfigPlatform::Linux,
            None,
            Some(PathBuf::from("/home/example")),
            None,
            None,
            None,
            PathBuf::from("/repo"),
        );

        assert_eq!(dir, PathBuf::from("/home/example/.config/yttt"));
    }

    #[test]
    fn native_config_dir_falls_back_to_current_directory() {
        let dir = native_config_dir_from_parts(
            ConfigPlatform::Linux,
            None,
            None,
            None,
            None,
            None,
            PathBuf::from("/repo"),
        );

        assert_eq!(dir, PathBuf::from("/repo/.yttt"));
    }

    #[test]
    fn existing_legacy_config_is_moved_to_native_location() {
        let temp = tempfile::tempdir().unwrap();
        let legacy = temp.path().join(".config").join("yttt");
        let native = temp
            .path()
            .join("Library")
            .join("Application Support")
            .join("yttt");
        fs::create_dir_all(&legacy).unwrap();
        fs::write(legacy.join("settings.toml"), "version = 1").unwrap();

        assert_eq!(
            migrate_legacy_config_dir(native.clone(), legacy.clone()),
            native
        );
        assert!(!legacy.exists());
        assert_eq!(
            fs::read_to_string(native.join("settings.toml")).unwrap(),
            "version = 1"
        );
    }

    #[test]
    fn windows_verbatim_paths_are_readable_in_the_ui() {
        assert_eq!(
            windows_path_for_display(r"\\?\C:\Users\example\project"),
            r"C:\Users\example\project"
        );
        assert_eq!(
            windows_path_for_display(r"\\?\UNC\server\share\project"),
            r"\\server\share\project"
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn canonical_project_paths_prefer_drive_letter_form() {
        let temp = tempfile::tempdir().unwrap();

        let canonical = canonicalize_path(temp.path()).unwrap();

        assert!(!canonical.to_string_lossy().starts_with(r"\\?\"));
    }
}
