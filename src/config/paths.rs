use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppConfigPaths {
    config_dir: PathBuf,
}

impl AppConfigPaths {
    pub fn from_config_dir(config_dir: impl Into<PathBuf>) -> Self {
        Self {
            config_dir: config_dir.into(),
        }
    }

    pub fn for_app() -> Self {
        Self::from_config_dir(default_config_dir())
    }

    pub fn config_dir(&self) -> &Path {
        &self.config_dir
    }

    pub fn project_layout_file(&self, project_path: &Path) -> PathBuf {
        project_path.join(".yttt").join("layout.toml")
    }

    pub fn default_layout_file(&self) -> PathBuf {
        self.config_dir.join("default-layout.toml")
    }

    pub fn local_project_dir(&self, project_path: &Path) -> PathBuf {
        let project_path = project_path
            .canonicalize()
            .unwrap_or_else(|_| project_path.to_path_buf());
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

    pub fn keybindings_file(&self) -> PathBuf {
        self.config_dir.join("keybindings.toml")
    }

    pub fn settings_file(&self) -> PathBuf {
        self.config_dir.join("settings.toml")
    }

    pub fn themes_dir(&self) -> PathBuf {
        self.config_dir.join("themes")
    }

    pub fn icon_themes_dir(&self) -> PathBuf {
        self.themes_dir().join("icons")
    }
}

fn fallback_config_dir() -> PathBuf {
    fallback_config_dir_from_parts(
        std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from),
        std::env::var_os("HOME").map(PathBuf::from),
        std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
    )
}

fn default_config_dir() -> PathBuf {
    fallback_config_dir()
}

fn fallback_config_dir_from_parts(
    xdg_config_home: Option<PathBuf>,
    home: Option<PathBuf>,
    current_dir: PathBuf,
) -> PathBuf {
    if let Some(config_home) = xdg_config_home.filter(|path| !path.as_os_str().is_empty()) {
        return config_home.join("yttt");
    }

    if let Some(home) = home.filter(|path| !path.as_os_str().is_empty()) {
        return home.join(".config").join("yttt");
    }

    current_dir.join(".yttt")
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
    fn fallback_config_dir_prefers_xdg_config_home() {
        let dir = fallback_config_dir_from_parts(
            Some(PathBuf::from("/tmp/xdg")),
            Some(PathBuf::from("/Users/example")),
            PathBuf::from("/repo"),
        );

        assert_eq!(dir, PathBuf::from("/tmp/xdg/yttt"));
    }

    #[test]
    fn fallback_config_dir_uses_home_config_when_xdg_is_missing() {
        let dir = fallback_config_dir_from_parts(
            None,
            Some(PathBuf::from("/Users/example")),
            PathBuf::from("/repo"),
        );

        assert_eq!(dir, PathBuf::from("/Users/example/.config/yttt"));
    }

    #[test]
    fn fallback_config_dir_uses_local_directory_only_without_home() {
        let dir = fallback_config_dir_from_parts(None, None, PathBuf::from("/repo"));

        assert_eq!(dir, PathBuf::from("/repo/.yttt"));
    }
}
