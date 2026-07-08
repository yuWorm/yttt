use std::{collections::HashMap, fs, path::PathBuf};

use crate::{commands::CommandRegistry, config::paths::AppConfigPaths};

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq, Eq)]
pub struct Keybinding {
    pub keys: String,
    pub command: String,
}

#[derive(Clone, Debug, Default, serde::Deserialize, serde::Serialize, PartialEq, Eq)]
pub struct KeybindingsConfig {
    #[serde(default)]
    pub bindings: Vec<Keybinding>,
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
    #[error("failed to serialize default keybindings at {path}: {source}")]
    SerializeDefaults {
        path: PathBuf,
        source: toml::ser::Error,
    },
    #[error("failed to write default keybindings at {path}: {source}")]
    WriteDefaults {
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

    Ok(LoadedKeybindings {
        warnings: keybinding_warnings(&config, registry),
        config,
    })
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

    let source = toml::to_string_pretty(&default_keybindings()).map_err(|source| {
        KeybindingsLoadError::SerializeDefaults {
            path: path.clone(),
            source,
        }
    })?;
    fs::write(&path, source).map_err(|source| KeybindingsLoadError::WriteDefaults {
        path: path.clone(),
        source,
    })?;

    Ok(path)
}

pub fn default_keybindings() -> KeybindingsConfig {
    KeybindingsConfig {
        bindings: vec![
            binding("cmd-o", "project.open"),
            binding("ctrl-o", "project.open"),
            binding("cmd-p", "command_palette.open"),
            binding("ctrl-p", "command_palette.open"),
            binding("cmd-shift-o", "project.palette"),
            binding("ctrl-shift-o", "project.palette"),
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
