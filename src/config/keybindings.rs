use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

use crate::{commands::CommandRegistry, config::paths::AppConfigPaths};

pub const KEYBINDINGS_SCHEMA_VERSION: u32 = 2;

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq, Eq)]
pub struct Keybinding {
    pub keys: String,
    pub command: String,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq, Eq)]
pub struct KeybindingsConfig {
    #[serde(default)]
    pub schema_version: u32,
    #[serde(default)]
    pub bindings: Vec<Keybinding>,
}

impl Default for KeybindingsConfig {
    fn default() -> Self {
        Self {
            schema_version: KEYBINDINGS_SCHEMA_VERSION,
            bindings: Vec::new(),
        }
    }
}

impl KeybindingsConfig {
    pub fn conflicts(&self) -> Vec<KeybindingConflict> {
        let mut by_keys: HashMap<String, Vec<String>> = HashMap::new();
        for binding in &self.bindings {
            by_keys
                .entry(normalize_keys(&binding.keys))
                .or_default()
                .push(binding.command.clone());
        }

        let mut conflicts: Vec<_> = by_keys
            .into_iter()
            .filter_map(|(keys, commands)| {
                if commands.len() > 1 {
                    Some(KeybindingConflict { keys, commands })
                } else {
                    None
                }
            })
            .collect();
        conflicts.sort_by(|left, right| left.keys.cmp(&right.keys));
        conflicts
    }

    pub fn invalid_commands(&self, registry: &CommandRegistry) -> Vec<String> {
        let mut invalid: Vec<_> = self
            .bindings
            .iter()
            .filter(|binding| !registry.contains_str(&binding.command))
            .map(|binding| binding.command.clone())
            .collect();
        invalid.sort();
        invalid.dedup();
        invalid
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeybindingConflict {
    pub keys: String,
    pub commands: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoadedKeybindings {
    pub config: KeybindingsConfig,
    pub warnings: Vec<KeybindingLoadWarning>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeybindingLoadWarning {
    Conflict(KeybindingConflict),
    InvalidCommand(String),
}

#[derive(Debug, thiserror::Error)]
pub enum KeybindingsLoadError {
    #[error("failed to create keybindings config directory {path}: {source}")]
    CreateConfigDirectory {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to read keybindings file at {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to parse keybindings file at {path}: {source}")]
    Parse {
        path: PathBuf,
        source: toml::de::Error,
    },
    #[error("failed to serialize keybindings config at {path}: {source}")]
    SerializeConfig {
        path: PathBuf,
        source: toml::ser::Error,
    },
    #[error("failed to write keybindings config at {path}: {source}")]
    WriteConfig {
        path: PathBuf,
        source: std::io::Error,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum KeybindingsSaveError {
    #[error("failed to create keybindings config directory {path}: {source}")]
    CreateConfigDirectory {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to serialize keybindings at {path}: {source}")]
    Serialize {
        path: PathBuf,
        source: toml::ser::Error,
    },
    #[error("failed to write keybindings at {path}: {source}")]
    Write {
        path: PathBuf,
        source: std::io::Error,
    },
}

pub fn load_keybindings(
    paths: &AppConfigPaths,
    registry: &CommandRegistry,
) -> Result<LoadedKeybindings, KeybindingsLoadError> {
    let path = ensure_keybindings_file(paths)?;
    let source = fs::read_to_string(&path).map_err(|source| KeybindingsLoadError::Read {
        path: path.clone(),
        source,
    })?;
    let config: KeybindingsConfig =
        toml::from_str(&source).map_err(|source| KeybindingsLoadError::Parse {
            path: path.clone(),
            source,
        })?;
    let config = migrate_keybindings_config(&path, config)?;

    Ok(LoadedKeybindings {
        warnings: keybinding_warnings(&config, registry),
        config,
    })
}

pub fn save_keybindings(
    paths: &AppConfigPaths,
    config: &KeybindingsConfig,
) -> Result<PathBuf, KeybindingsSaveError> {
    let path = paths.keybindings_file();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| {
            KeybindingsSaveError::CreateConfigDirectory {
                path: parent.to_path_buf(),
                source,
            }
        })?;
    }

    let source =
        toml::to_string_pretty(config).map_err(|source| KeybindingsSaveError::Serialize {
            path: path.clone(),
            source,
        })?;
    fs::write(&path, source).map_err(|source| KeybindingsSaveError::Write {
        path: path.clone(),
        source,
    })?;

    Ok(path)
}

pub fn ensure_keybindings_file(paths: &AppConfigPaths) -> Result<PathBuf, KeybindingsLoadError> {
    let path = paths.keybindings_file();
    if path.exists() {
        return Ok(path);
    }

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| {
            KeybindingsLoadError::CreateConfigDirectory {
                path: parent.to_path_buf(),
                source,
            }
        })?;
    }

    write_keybindings_config(&path, &default_keybindings())?;

    Ok(path)
}

fn migrate_keybindings_config(
    path: &Path,
    mut config: KeybindingsConfig,
) -> Result<KeybindingsConfig, KeybindingsLoadError> {
    if config.schema_version >= KEYBINDINGS_SCHEMA_VERSION {
        return Ok(config);
    }

    let uses_legacy_defaults = match config.schema_version {
        0 => config.bindings == legacy_v0_default_bindings(),
        1 => config.bindings == legacy_v1_default_bindings(),
        _ => false,
    };
    if uses_legacy_defaults {
        config = default_keybindings();
    } else {
        config.schema_version = KEYBINDINGS_SCHEMA_VERSION;
    }
    write_keybindings_config(path, &config)?;
    Ok(config)
}

fn legacy_v0_default_bindings() -> Vec<Keybinding> {
    let mut bindings = legacy_v1_default_bindings();
    bindings.retain(|binding| {
        !matches!(
            binding.command.as_str(),
            "file.save" | "project_panel.toggle"
        )
    });
    bindings
}

fn legacy_v1_default_bindings() -> Vec<Keybinding> {
    let mut bindings = default_keybindings().bindings;
    bindings.retain(|binding| binding.command != "project.opened_palette");
    bindings
}

fn write_keybindings_config(
    path: &Path,
    config: &KeybindingsConfig,
) -> Result<(), KeybindingsLoadError> {
    let source =
        toml::to_string_pretty(config).map_err(|source| KeybindingsLoadError::SerializeConfig {
            path: path.to_path_buf(),
            source,
        })?;
    fs::write(path, source).map_err(|source| KeybindingsLoadError::WriteConfig {
        path: path.to_path_buf(),
        source,
    })
}

pub fn default_keybindings() -> KeybindingsConfig {
    KeybindingsConfig {
        schema_version: KEYBINDINGS_SCHEMA_VERSION,
        bindings: vec![
            binding("cmd-o", "project.open"),
            binding("ctrl-o", "project.open"),
            binding("cmd-p", "command_palette.open"),
            binding("ctrl-p", "command_palette.open"),
            binding("cmd-,", "settings.open"),
            binding("ctrl-,", "settings.open"),
            binding("cmd-s", "file.save"),
            binding("ctrl-s", "file.save"),
            binding("cmd-shift-e", "project_panel.toggle"),
            binding("ctrl-shift-e", "project_panel.toggle"),
            binding("cmd-shift-o", "project.palette"),
            binding("ctrl-shift-o", "project.palette"),
            binding("cmd-shift-p", "project.opened_palette"),
            binding("ctrl-shift-p", "project.opened_palette"),
            binding("cmd-j", "tab.palette"),
            binding("ctrl-j", "tab.palette"),
            binding("cmd-k", "pane.palette"),
            binding("ctrl-k", "pane.palette"),
            binding("cmd-t", "tab.new"),
            binding("ctrl-t", "tab.new"),
            binding("cmd-d", "pane.split_vertical"),
            binding("ctrl-d", "pane.split_vertical"),
            binding("cmd-shift-d", "pane.split_horizontal"),
            binding("ctrl-shift-d", "pane.split_horizontal"),
            binding("cmd-w", "pane.close"),
            binding("ctrl-w", "pane.close"),
            binding("cmd-alt-left", "pane.focus_left"),
            binding("cmd-alt-right", "pane.focus_right"),
            binding("cmd-alt-up", "pane.focus_up"),
            binding("cmd-alt-down", "pane.focus_down"),
            binding("ctrl-alt-left", "pane.focus_left"),
            binding("ctrl-alt-right", "pane.focus_right"),
            binding("ctrl-alt-up", "pane.focus_up"),
            binding("ctrl-alt-down", "pane.focus_down"),
            binding("cmd-alt-shift-left", "pane.resize_left"),
            binding("cmd-alt-shift-right", "pane.resize_right"),
            binding("cmd-alt-shift-up", "pane.resize_up"),
            binding("cmd-alt-shift-down", "pane.resize_down"),
            binding("ctrl-alt-shift-left", "pane.resize_left"),
            binding("ctrl-alt-shift-right", "pane.resize_right"),
            binding("ctrl-alt-shift-up", "pane.resize_up"),
            binding("ctrl-alt-shift-down", "pane.resize_down"),
        ],
    }
}

fn binding(keys: &str, command: &str) -> Keybinding {
    Keybinding {
        keys: keys.to_string(),
        command: command.to_string(),
    }
}

fn normalize_keys(keys: &str) -> String {
    keys.trim().to_ascii_lowercase()
}

fn keybinding_warnings(
    config: &KeybindingsConfig,
    registry: &CommandRegistry,
) -> Vec<KeybindingLoadWarning> {
    let mut warnings: Vec<_> = config
        .conflicts()
        .into_iter()
        .map(KeybindingLoadWarning::Conflict)
        .collect();
    warnings.extend(
        config
            .invalid_commands(registry)
            .into_iter()
            .map(KeybindingLoadWarning::InvalidCommand),
    );
    warnings
}
