use std::{
    fs,
    path::{Path, PathBuf},
};

use crate::config::paths::AppConfigPaths;

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct AppSettings {
    pub theme: ThemeSettings,
    pub notifications: NotificationSettings,
    pub terminal: TerminalSettings,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            theme: ThemeSettings::default(),
            notifications: NotificationSettings::default(),
            terminal: TerminalSettings::default(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct ThemeSettings {
    pub name: String,
    pub terminal: Option<String>,
}

impl Default for ThemeSettings {
    fn default() -> Self {
        Self {
            name: "yttt-dark".to_string(),
            terminal: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct NotificationSettings {
    pub system: bool,
}

impl Default for NotificationSettings {
    fn default() -> Self {
        Self { system: false }
    }
}

pub const AUTO_SHELL: &str = "auto";

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct TerminalSettings {
    pub shell: String,
    pub font_family: String,
    pub font_size: f32,
    pub line_height: f32,
    pub padding: f32,
    pub scrollback: usize,
}

impl Default for TerminalSettings {
    fn default() -> Self {
        Self {
            shell: AUTO_SHELL.to_string(),
            font_family: "monospace".to_string(),
            font_size: 13.0,
            line_height: 1.15,
            padding: 6.0,
            scrollback: 10000,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct LoadedSettings {
    pub settings: AppSettings,
    pub warnings: Vec<SettingsLoadWarning>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum SettingsLoadWarning {
    InvalidToml { path: PathBuf, message: String },
    InvalidTerminalValue { field: &'static str },
}

#[derive(Debug, thiserror::Error)]
pub enum SettingsLoadError {
    #[error("failed to create settings config directory {path}: {source}")]
    CreateConfigDirectory {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to read settings file at {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to serialize default settings at {path}: {source}")]
    SerializeDefaults {
        path: PathBuf,
        source: toml::ser::Error,
    },
    #[error("failed to write default settings at {path}: {source}")]
    WriteDefaults {
        path: PathBuf,
        source: std::io::Error,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum SettingsSaveError {
    #[error("failed to create settings config directory {path}: {source}")]
    CreateConfigDirectory {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to serialize settings at {path}: {source}")]
    Serialize {
        path: PathBuf,
        source: toml::ser::Error,
    },
    #[error("failed to write settings at {path}: {source}")]
    Write {
        path: PathBuf,
        source: std::io::Error,
    },
}

pub fn load_or_create_settings(
    paths: &AppConfigPaths,
) -> Result<LoadedSettings, SettingsLoadError> {
    let path = ensure_settings_file(paths)?;
    let source = fs::read_to_string(&path).map_err(|source| SettingsLoadError::Read {
        path: path.clone(),
        source,
    })?;

    let mut warnings = Vec::new();
    let settings = match toml::from_str::<AppSettings>(&source) {
        Ok(settings) => settings,
        Err(error) => {
            warnings.push(SettingsLoadWarning::InvalidToml {
                path,
                message: error.to_string(),
            });
            AppSettings::default()
        }
    };
    let settings = validate_settings(settings, &mut warnings);

    Ok(LoadedSettings { settings, warnings })
}

pub fn save_settings(
    paths: &AppConfigPaths,
    settings: &AppSettings,
) -> Result<PathBuf, SettingsSaveError> {
    let path = paths.settings_file();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| SettingsSaveError::CreateConfigDirectory {
            path: parent.to_path_buf(),
            source,
        })?;
    }

    let source =
        toml::to_string_pretty(settings).map_err(|source| SettingsSaveError::Serialize {
            path: path.clone(),
            source,
        })?;
    fs::write(&path, source).map_err(|source| SettingsSaveError::Write {
        path: path.clone(),
        source,
    })?;

    Ok(path)
}

pub fn detect_shell_candidates() -> Vec<String> {
    let shell_env = std::env::var("SHELL").ok();
    detect_shell_candidates_with(shell_env.as_deref(), |path| Path::new(path).exists())
}

pub fn detect_shell_candidates_with(
    shell_env: Option<&str>,
    exists: impl Fn(&str) -> bool,
) -> Vec<String> {
    let mut candidates = Vec::new();
    if let Some(shell_env) = shell_env.map(str::trim).filter(|shell| !shell.is_empty()) {
        push_unique(&mut candidates, shell_env);
    }

    for shell in ["/bin/zsh", "/bin/bash"] {
        if exists(shell) {
            push_unique(&mut candidates, shell);
        }
    }
    push_unique(&mut candidates, "sh");

    candidates
}

pub fn resolve_default_shell(shell: &str, candidates: &[String]) -> String {
    let shell = shell.trim();
    if !shell.is_empty() && shell != AUTO_SHELL {
        return shell.to_string();
    }

    candidates
        .first()
        .cloned()
        .unwrap_or_else(|| "sh".to_string())
}

fn ensure_settings_file(paths: &AppConfigPaths) -> Result<PathBuf, SettingsLoadError> {
    let path = paths.settings_file();
    if path.exists() {
        return Ok(path);
    }

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| SettingsLoadError::CreateConfigDirectory {
            path: parent.to_path_buf(),
            source,
        })?;
    }

    let source = toml::to_string_pretty(&AppSettings::default()).map_err(|source| {
        SettingsLoadError::SerializeDefaults {
            path: path.clone(),
            source,
        }
    })?;
    fs::write(&path, source).map_err(|source| SettingsLoadError::WriteDefaults {
        path: path.clone(),
        source,
    })?;

    Ok(path)
}

fn validate_settings(
    mut settings: AppSettings,
    warnings: &mut Vec<SettingsLoadWarning>,
) -> AppSettings {
    let defaults = TerminalSettings::default();

    if settings.terminal.font_size <= 0.0 {
        settings.terminal.font_size = defaults.font_size;
        warnings.push(SettingsLoadWarning::InvalidTerminalValue { field: "font_size" });
    }
    if settings.terminal.line_height <= 0.0 {
        settings.terminal.line_height = defaults.line_height;
        warnings.push(SettingsLoadWarning::InvalidTerminalValue {
            field: "line_height",
        });
    }
    if settings.terminal.padding < 0.0 {
        settings.terminal.padding = defaults.padding;
        warnings.push(SettingsLoadWarning::InvalidTerminalValue { field: "padding" });
    }
    if settings.terminal.scrollback == 0 {
        settings.terminal.scrollback = defaults.scrollback;
        warnings.push(SettingsLoadWarning::InvalidTerminalValue {
            field: "scrollback",
        });
    }
    if settings.terminal.shell.trim().is_empty() {
        settings.terminal.shell = defaults.shell;
        warnings.push(SettingsLoadWarning::InvalidTerminalValue { field: "shell" });
    }

    settings
}

fn push_unique(values: &mut Vec<String>, value: &str) {
    if values.iter().all(|existing| existing != value) {
        values.push(value.to_string());
    }
}
