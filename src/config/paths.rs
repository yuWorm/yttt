use std::path::{Path, PathBuf};

use directories::ProjectDirs;

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
        ProjectDirs::from("dev", "yttt", "yttt")
            .map(|dirs| Self::from_config_dir(dirs.config_dir().to_path_buf()))
            .unwrap_or_else(|| Self::from_config_dir(fallback_config_dir()))
    }

    pub fn config_dir(&self) -> &Path {
        &self.config_dir
    }

    pub fn project_layout_file(&self, project_path: &Path) -> PathBuf {
        project_path.join(".yttt").join("layout.toml")
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
}

fn fallback_config_dir() -> PathBuf {
    if let Some(config_home) = std::env::var_os("XDG_CONFIG_HOME") {
        return PathBuf::from(config_home).join("yttt");
    }

    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(".yttt")
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
