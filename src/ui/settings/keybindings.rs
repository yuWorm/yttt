use crate::{
    commands::{CommandId, CommandRegistry},
    config::{
        keybindings::{
            Keybinding, KeybindingsConfig, KeybindingsSaveError, default_keybindings,
            save_keybindings,
        },
        paths::AppConfigPaths,
    },
    palette::{command_description_with_text, command_title_with_text},
    ui::i18n::UiText,
};

use super::keybinding_display::display_keybindings_for_current_platform;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeybindingRow {
    pub command: CommandId,
    pub command_id: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    pub keys: Vec<String>,
    pub has_conflict: bool,
}

impl KeybindingRow {
    pub fn display_keys(&self) -> Vec<String> {
        display_keybindings_for_current_platform(&self.keys)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeybindingsEditorState {
    config: KeybindingsConfig,
    registry: CommandRegistry,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum KeybindingEditError {
    #[error("conflicting keybindings: {0:?}")]
    ConflictingBindings(Vec<String>),
    #[error("invalid command ids: {0:?}")]
    InvalidCommands(Vec<String>),
    #[error("{0}")]
    Save(String),
}

impl From<KeybindingsSaveError> for KeybindingEditError {
    fn from(error: KeybindingsSaveError) -> Self {
        Self::Save(error.to_string())
    }
}

impl KeybindingsEditorState {
    pub fn new(config: KeybindingsConfig, registry: CommandRegistry) -> Self {
        Self { config, registry }
    }

    pub fn config(&self) -> &KeybindingsConfig {
        &self.config
    }

    pub fn rows(&self) -> Vec<KeybindingRow> {
        self.rows_with_text(&UiText::english())
    }

    pub fn rows_with_text(&self, text: &UiText) -> Vec<KeybindingRow> {
        let conflicting_keys: Vec<_> = self
            .config
            .conflicts()
            .into_iter()
            .map(|conflict| conflict.keys)
            .collect();

        CommandId::ALL
            .iter()
            .copied()
            .filter(|command| self.registry.contains(*command))
            .map(|command| {
                let keys = self.command_keys(command);
                let has_conflict = keys.iter().any(|key| {
                    conflicting_keys
                        .iter()
                        .any(|conflict| conflict == &normalize_keys(key))
                });

                KeybindingRow {
                    command,
                    command_id: command.as_str(),
                    title: command_title_with_text(command, text),
                    description: command_description_with_text(command, text),
                    keys,
                    has_conflict,
                }
            })
            .collect()
    }

    pub fn command_keys(&self, command: CommandId) -> Vec<String> {
        self.config
            .bindings
            .iter()
            .filter(|binding| binding.command == command.as_str())
            .map(|binding| binding.keys.clone())
            .collect()
    }

    pub fn set_command_keys(&mut self, command: CommandId, keys: Vec<String>) {
        self.delete_command_keys(command);
        for key in normalize_key_list(keys) {
            self.config.bindings.push(Keybinding {
                keys: key,
                command: command.as_str().to_string(),
            });
        }
    }

    pub fn delete_command_keys(&mut self, command: CommandId) {
        self.config
            .bindings
            .retain(|binding| binding.command != command.as_str());
    }

    pub fn reset_command_keys(&mut self, command: CommandId) {
        self.delete_command_keys(command);
        self.config.bindings.extend(
            default_keybindings()
                .bindings
                .into_iter()
                .filter(|binding| binding.command == command.as_str()),
        );
    }

    pub fn save(&self, paths: &AppConfigPaths) -> Result<(), KeybindingEditError> {
        let conflicts: Vec<_> = self
            .config
            .conflicts()
            .into_iter()
            .map(|conflict| conflict.keys)
            .collect();
        if !conflicts.is_empty() {
            return Err(KeybindingEditError::ConflictingBindings(conflicts));
        }

        let invalid = self.config.invalid_commands(&self.registry);
        if !invalid.is_empty() {
            return Err(KeybindingEditError::InvalidCommands(invalid));
        }

        save_keybindings(paths, &self.config)?;
        Ok(())
    }
}

fn normalize_key_list(keys: Vec<String>) -> Vec<String> {
    let mut keys: Vec<_> = keys
        .into_iter()
        .map(|key| normalize_keys(&key))
        .filter(|key| !key.is_empty())
        .collect();
    keys.sort();
    keys.dedup();
    keys
}

fn normalize_keys(keys: &str) -> String {
    keys.trim().to_ascii_lowercase()
}
