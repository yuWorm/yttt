use std::{fs, path::PathBuf};

use crate::config::paths::AppConfigPaths;

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct AppSettings {
    pub theme: ThemeSettings,
    pub terminal: TerminalSettings,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            theme: ThemeSettings::default(),
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
pub struct TerminalSettings {
    pub font_family: String,
    pub font_size: f32,
    pub line_height: f32,
    pub padding: f32,
    pub scrollback: usize,
}

impl Default for TerminalSettings {
    fn default() -> Self {
        Self {
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

    settings
}
