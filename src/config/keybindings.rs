use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use gpui::{KeyBindingContextPredicate, Keystroke};

use crate::{commands::CommandRegistry, config::paths::AppConfigPaths};

use super::atomic_write;

pub const KEYBINDINGS_SCHEMA_VERSION: u32 = 6;
pub const DEFAULT_KEYBINDING_CONTEXT: &str = "Workspace";
pub const DEFAULT_KEYBINDING_LEADER: &str = "space";

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq, Eq)]
pub struct Keybinding {
    pub keys: String,
    #[serde(default)]
    pub command: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub unbind: bool,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq, Eq)]
pub struct KeybindingsConfig {
    #[serde(default)]
    pub schema_version: u32,
    #[serde(default = "default_keybinding_leader")]
    pub leader: String,
    #[serde(default)]
    pub bindings: Vec<Keybinding>,
}

impl Default for KeybindingsConfig {
    fn default() -> Self {
        Self {
            schema_version: KEYBINDINGS_SCHEMA_VERSION,
            leader: default_keybinding_leader(),
            bindings: Vec::new(),
        }
    }
}

impl KeybindingsConfig {
    pub fn conflicts(&self) -> Vec<KeybindingConflict> {
        let mut by_keys: HashMap<(Option<String>, String), Vec<String>> = HashMap::new();
        for binding in self.bindings.iter().filter(|binding| !binding.unbind) {
            let commands = by_keys
                .entry((
                    normalize_context(binding.context.as_deref()),
                    resolve_keybinding_sequence(&binding.keys, &self.leader),
                ))
                .or_default();
            if !commands.contains(&binding.command) {
                commands.push(binding.command.clone());
            }
        }

        let mut conflicts: Vec<_> = by_keys
            .into_iter()
            .filter_map(|((context, keys), commands)| {
                if commands.len() > 1 {
                    Some(KeybindingConflict {
                        keys,
                        context,
                        commands,
                    })
                } else {
                    None
                }
            })
            .collect();
        conflicts.sort_by(|left, right| {
            left.context
                .cmp(&right.context)
                .then(left.keys.cmp(&right.keys))
        });
        conflicts
    }

    pub fn invalid_commands(&self, registry: &CommandRegistry) -> Vec<String> {
        let mut invalid: Vec<_> = self
            .bindings
            .iter()
            .filter(|binding| {
                !binding.command.is_empty() && !registry.contains_str(&binding.command)
            })
            .map(|binding| binding.command.clone())
            .collect();
        invalid.sort();
        invalid.dedup();
        invalid
    }

    pub fn invalid_bindings(&self) -> Vec<String> {
        let mut invalid = Vec::new();
        let leader = normalize_keys(&self.leader);
        if leader.split_whitespace().count() != 1
            || leader.eq_ignore_ascii_case("<leader>")
            || Keystroke::parse(&leader).is_err()
        {
            invalid.push(format!("invalid leader key {:?}", self.leader));
        }

        for binding in &self.bindings {
            let keys = resolve_keybinding_sequence(&binding.keys, &leader);
            if keys.is_empty()
                || keys
                    .split_whitespace()
                    .any(|keystroke| Keystroke::parse(keystroke).is_err())
            {
                invalid.push(format!("invalid key sequence {:?}", binding.keys));
            }
            if !binding.unbind && binding.command.trim().is_empty() {
                invalid.push(format!("missing command for {:?}", binding.keys));
            }
            if let Some(context) = binding.context.as_deref()
                && KeyBindingContextPredicate::parse(context).is_err()
            {
                invalid.push(format!(
                    "invalid context {context:?} for {:?}",
                    binding.keys
                ));
            }
        }
        invalid.sort();
        invalid.dedup();
        invalid
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeybindingConflict {
    pub keys: String,
    pub context: Option<String>,
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
    InvalidBinding(String),
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
    let source = crate::config::storage::read_to_string(&path).map_err(|source| {
        KeybindingsLoadError::Read {
            path: path.clone(),
            source,
        }
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
        crate::config::storage::create_dir_all(parent).map_err(|source| {
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
    atomic_write(&path, source.as_bytes()).map_err(|source| KeybindingsSaveError::Write {
        path: path.clone(),
        source,
    })?;

    Ok(path)
}

pub fn ensure_keybindings_file(paths: &AppConfigPaths) -> Result<PathBuf, KeybindingsLoadError> {
    let path = paths.keybindings_file();
    if crate::config::storage::exists(&path) {
        return Ok(path);
    }

    if let Some(parent) = path.parent() {
        crate::config::storage::create_dir_all(parent).map_err(|source| {
            KeybindingsLoadError::CreateConfigDirectory {
                path: parent.to_path_buf(),
                source,
            }
        })?;
    }

    write_keybindings_config(&path, &KeybindingsConfig::default())?;

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
        2 => config.bindings == legacy_v2_default_bindings(),
        3 => config.bindings == legacy_v3_default_bindings(),
        _ => false,
    };
    if uses_legacy_defaults {
        config = KeybindingsConfig::default();
    } else {
        if config.schema_version <= 3 {
            for binding in &mut config.bindings {
                if binding.context.is_none() {
                    binding.context = Some(DEFAULT_KEYBINDING_CONTEXT.to_string());
                }
            }
            config.bindings = sparse_overrides_for_authoritative_bindings(config.bindings);
        }
        remove_legacy_ctrl_w_close(&mut config.bindings);
        migrate_legacy_vim_contexts(&mut config.bindings);
        config.schema_version = KEYBINDINGS_SCHEMA_VERSION;
    }
    write_keybindings_config(path, &config)?;
    Ok(config)
}

fn sparse_overrides_for_authoritative_bindings(authoritative: Vec<Keybinding>) -> Vec<Keybinding> {
    let defaults = default_keybindings().bindings;
    let mut overrides = Vec::with_capacity(defaults.len() + authoritative.len());

    for binding in &defaults {
        if !authoritative
            .iter()
            .any(|candidate| !candidate.unbind && same_binding_assignment(candidate, binding))
        {
            let mut unbind = binding.clone();
            unbind.unbind = true;
            overrides.push(unbind);
        }
    }
    overrides.extend(authoritative.into_iter().filter(|binding| {
        binding.unbind
            || !defaults
                .iter()
                .any(|candidate| same_binding_assignment(candidate, binding))
    }));
    overrides
}

fn remove_legacy_ctrl_w_close(bindings: &mut Vec<Keybinding>) {
    bindings.retain(|binding| {
        let context = normalize_context(binding.context.as_deref());
        !(normalize_keys(&binding.keys) == "ctrl-w"
            && binding.command.trim() == "pane.close"
            && matches!(
                context.as_deref(),
                Some("Workspace" | "Workspace && !WorkspaceVim")
            ))
    });
}

fn migrate_legacy_vim_contexts(bindings: &mut [Keybinding]) {
    const GLOBAL_VIM_CONTEXT: &str = "YtttVim && yttt_vim_scope == global";
    const NEGATED_GLOBAL_VIM_CONTEXT: &str = "!(YtttVim && yttt_vim_scope == global)";

    for binding in bindings {
        if let Some(context) = &mut binding.context {
            *context = context
                .replace("!WorkspaceVim", NEGATED_GLOBAL_VIM_CONTEXT)
                .replace("! WorkspaceVim", NEGATED_GLOBAL_VIM_CONTEXT)
                .replace("WorkspaceVim", GLOBAL_VIM_CONTEXT);
        }
    }
}

fn same_binding_assignment(left: &Keybinding, right: &Keybinding) -> bool {
    normalize_keys(&left.keys) == normalize_keys(&right.keys)
        && normalize_context(left.context.as_deref()) == normalize_context(right.context.as_deref())
        && left.command.trim() == right.command.trim()
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
    let mut bindings = legacy_v2_default_bindings();
    bindings.retain(|binding| binding.command != "project.opened_palette");
    bindings
}

fn legacy_v3_default_bindings() -> Vec<Keybinding> {
    let mut bindings = default_keybindings().bindings;
    bindings.retain(|binding| !matches!(binding.command.as_str(), "tab.next" | "tab.prev"));
    let cmd_w_index = bindings
        .iter()
        .position(|binding| binding.keys == "cmd-w" && binding.command == "pane.close")
        .expect("legacy defaults must include cmd-w pane close");
    bindings.insert(cmd_w_index + 1, binding("ctrl-w", "pane.close"));
    for binding in &mut bindings {
        binding.context = None;
    }
    bindings
}

fn legacy_v2_default_bindings() -> Vec<Keybinding> {
    let mut bindings = legacy_v3_default_bindings();
    bindings.retain(|binding| binding.command != "file.find");
    for binding in &mut bindings {
        match (binding.keys.as_str(), binding.command.as_str()) {
            ("cmd-shift-p", "command_palette.open") => binding.keys = "cmd-p".to_string(),
            ("ctrl-shift-p", "command_palette.open") => binding.keys = "ctrl-p".to_string(),
            ("cmd-alt-p", "project.opened_palette") => binding.keys = "cmd-shift-p".to_string(),
            ("ctrl-alt-p", "project.opened_palette") => binding.keys = "ctrl-shift-p".to_string(),
            _ => {}
        }
    }
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
    atomic_write(path, source.as_bytes()).map_err(|source| KeybindingsLoadError::WriteConfig {
        path: path.to_path_buf(),
        source,
    })
}

pub fn default_keybindings() -> KeybindingsConfig {
    KeybindingsConfig {
        schema_version: KEYBINDINGS_SCHEMA_VERSION,
        leader: default_keybinding_leader(),
        bindings: vec![
            binding("cmd-q", "application.quit"),
            binding("cmd-o", "project.open"),
            binding("ctrl-o", "project.open"),
            binding("cmd-p", "file.find"),
            binding("ctrl-p", "file.find"),
            binding("cmd-shift-p", "command_palette.open"),
            binding("ctrl-shift-p", "command_palette.open"),
            binding("cmd-,", "settings.open"),
            binding("ctrl-,", "settings.open"),
            binding("cmd-s", "file.save"),
            binding("ctrl-s", "file.save"),
            binding("cmd-shift-e", "project_panel.toggle"),
            binding("ctrl-shift-e", "project_panel.toggle"),
            binding("cmd-shift-o", "project.palette"),
            binding("ctrl-shift-o", "project.palette"),
            binding("cmd-alt-p", "project.opened_palette"),
            binding("ctrl-alt-p", "project.opened_palette"),
            binding("cmd-j", "tab.palette"),
            binding("ctrl-j", "tab.palette"),
            binding("cmd-k", "pane.palette"),
            binding("ctrl-k", "pane.palette"),
            binding("cmd-t", "tab.new"),
            binding("ctrl-t", "tab.new"),
            binding("cmd-]", "tab.next"),
            binding("ctrl-tab", "tab.next"),
            binding("cmd-[", "tab.prev"),
            binding("ctrl-shift-tab", "tab.prev"),
            binding("cmd-d", "pane.split_vertical"),
            binding("ctrl-d", "pane.split_vertical"),
            binding("cmd-shift-d", "pane.split_horizontal"),
            binding("ctrl-shift-d", "pane.split_horizontal"),
            binding("cmd-w", "pane.close"),
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
    contextual_binding(keys, command, DEFAULT_KEYBINDING_CONTEXT)
}

fn contextual_binding(keys: &str, command: &str, context: &str) -> Keybinding {
    Keybinding {
        keys: keys.to_string(),
        command: command.to_string(),
        context: Some(context.to_string()),
        unbind: false,
    }
}

fn default_keybinding_leader() -> String {
    DEFAULT_KEYBINDING_LEADER.to_string()
}

pub fn resolve_keybinding_sequence(keys: &str, leader: &str) -> String {
    let keys = normalize_keys(keys);
    let leader = normalize_keys(leader);
    let mut resolved = String::with_capacity(keys.len().saturating_add(leader.len()));
    for key in keys.split_whitespace() {
        if !resolved.is_empty() {
            resolved.push(' ');
        }
        if key.eq_ignore_ascii_case("<leader>") {
            resolved.push_str(&leader);
        } else {
            resolved.push_str(key);
        }
    }
    resolved
}

fn normalize_keys(keys: &str) -> String {
    keys.trim().to_ascii_lowercase()
}

fn normalize_context(context: Option<&str>) -> Option<String> {
    context
        .map(str::trim)
        .filter(|context| !context.is_empty())
        .map(str::to_string)
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
    warnings.extend(
        config
            .invalid_bindings()
            .into_iter()
            .map(KeybindingLoadWarning::InvalidBinding),
    );
    warnings
}
