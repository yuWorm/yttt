use std::collections::HashMap;

use crate::{
    commands::{CommandId, CommandRegistry},
    config::{
        keybindings::{Keybinding, KeybindingsConfig, KeybindingsSaveError, save_keybindings},
        paths::AppConfigPaths,
    },
    palette::{command_description_with_text, command_title_with_text},
    ui::{
        i18n::UiText,
        interaction::actions::{
            BindableActionId, UiKeybindingSpec, assigned_ui_keybinding_specs,
            default_bindings_for_action_with_leader, preferred_context_for_action,
        },
    },
};

use super::keybinding_display::display_keybindings_for_current_platform;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeybindingRow {
    pub command: BindableActionId,
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
    source_load_error: Option<String>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum KeybindingEditError {
    #[error("conflicting keybindings: {0:?}")]
    ConflictingBindings(Vec<String>),
    #[error("invalid command ids: {0:?}")]
    InvalidCommands(Vec<String>),
    #[error("invalid keybindings: {0:?}")]
    InvalidBindings(Vec<String>),
    #[error("refusing to overwrite invalid keybindings file: {0}")]
    InvalidSource(String),
    #[error("{0}")]
    Save(String),
}

struct AssignmentConflict {
    keys: String,
    actions: Vec<BindableActionId>,
}

impl From<KeybindingsSaveError> for KeybindingEditError {
    fn from(error: KeybindingsSaveError) -> Self {
        Self::Save(error.to_string())
    }
}

impl KeybindingsEditorState {
    pub fn new(config: KeybindingsConfig, registry: CommandRegistry) -> Self {
        Self {
            config,
            registry,
            source_load_error: None,
        }
    }

    pub fn with_load_error(
        config: KeybindingsConfig,
        registry: CommandRegistry,
        error: impl Into<String>,
    ) -> Self {
        Self {
            config,
            registry,
            source_load_error: Some(error.into()),
        }
    }

    pub fn source_load_error(&self) -> Option<&str> {
        self.source_load_error.as_deref()
    }

    pub fn mark_source_invalid(&mut self, error: impl Into<String>) {
        self.source_load_error = Some(error.into());
    }

    pub fn config(&self) -> &KeybindingsConfig {
        &self.config
    }

    pub fn rows(&self) -> Vec<KeybindingRow> {
        self.rows_with_text(&UiText::english())
    }

    pub fn rows_with_text(&self, text: &UiText) -> Vec<KeybindingRow> {
        let bindings = assigned_ui_keybinding_specs(&self.config, &self.registry);
        let conflicts = assignment_conflicts(&bindings);

        BindableActionId::all()
            .filter(|action| self.registry.contains_str(action.as_str()))
            .map(|action| {
                let keys = action_keys_from_bindings(&bindings, action);
                let has_conflict = conflicts
                    .iter()
                    .any(|conflict| conflict.actions.contains(&action));
                let (title, description) = match action.command() {
                    Some(command) => (
                        command_title_with_text(command, text),
                        command_description_with_text(command, text),
                    ),
                    None => (
                        action.title().unwrap_or(action.as_str()),
                        action.description().unwrap_or(action.as_str()),
                    ),
                };

                KeybindingRow {
                    command: action,
                    command_id: action.as_str(),
                    title,
                    description,
                    keys,
                    has_conflict,
                }
            })
            .collect()
    }

    pub fn action_keys(&self, action: BindableActionId) -> Vec<String> {
        let bindings = assigned_ui_keybinding_specs(&self.config, &self.registry);
        action_keys_from_bindings(&bindings, action)
    }

    pub fn command_keys(&self, command: CommandId) -> Vec<String> {
        self.action_keys(BindableActionId::Command(command))
    }

    pub fn set_action_keys(&mut self, action: BindableActionId, keys: Vec<String>) {
        let keys = normalize_key_list(keys);
        let current_bindings = assigned_ui_keybinding_specs(&self.config, &self.registry);
        let current_keys = normalize_key_list(action_keys_from_bindings(&current_bindings, action));
        if keys == current_keys {
            return;
        }

        let mut current_contexts = HashMap::<String, Vec<Option<String>>>::new();
        for binding in current_bindings
            .iter()
            .filter(|binding| binding.command == action)
        {
            let contexts = current_contexts
                .entry(normalize_keys(binding.keys.as_ref()))
                .or_default();
            let context = binding.context.as_deref().map(str::to_string);
            if !contexts.contains(&context) {
                contexts.push(context);
            }
        }

        let defaults = default_bindings_for_action_with_leader(action, &self.config.leader);
        let mut default_contexts = defaults
            .iter()
            .map(|binding| binding.context.as_deref().map(str::to_string))
            .collect::<Vec<_>>();
        if default_contexts.is_empty() {
            default_contexts.push(preferred_context_for_action(action));
        }
        default_contexts.sort();
        default_contexts.dedup();

        let desired = keys
            .into_iter()
            .map(|key| {
                let contexts = current_contexts
                    .remove(&key)
                    .unwrap_or_else(|| default_contexts.clone());
                (key, contexts)
            })
            .collect::<Vec<_>>();

        self.remove_action_overrides(action);
        for binding in &defaults {
            let binding_context = binding.context.as_deref().map(str::to_string);
            let preserves_default = desired.iter().any(|(key, contexts)| {
                normalize_keys(binding.keys.as_ref()) == *key && contexts.contains(&binding_context)
            });
            if !preserves_default {
                self.config.bindings.push(Keybinding {
                    keys: binding.keys.to_string(),
                    command: action.as_str().to_string(),
                    context: binding_context,
                    unbind: true,
                });
            }
        }

        for (key, contexts) in desired {
            for context in contexts {
                if defaults.iter().any(|binding| {
                    normalize_keys(binding.keys.as_ref()) == key
                        && binding.context.as_deref() == context.as_deref()
                }) {
                    continue;
                }
                self.config.bindings.push(Keybinding {
                    keys: key.clone(),
                    command: action.as_str().to_string(),
                    context,
                    unbind: false,
                });
            }
        }
    }

    pub fn set_command_keys(&mut self, command: CommandId, keys: Vec<String>) {
        self.set_action_keys(BindableActionId::Command(command), keys);
    }

    pub fn delete_action_keys(&mut self, action: BindableActionId) {
        self.set_action_keys(action, Vec::new());
    }

    pub fn delete_command_keys(&mut self, command: CommandId) {
        self.delete_action_keys(BindableActionId::Command(command));
    }

    pub fn reset_action_keys(&mut self, action: BindableActionId) {
        self.remove_action_overrides(action);
    }

    pub fn reset_command_keys(&mut self, command: CommandId) {
        self.reset_action_keys(BindableActionId::Command(command));
    }

    fn remove_action_overrides(&mut self, action: BindableActionId) {
        self.config
            .bindings
            .retain(|binding| binding.command != action.as_str());
    }

    pub fn save(&self, paths: &AppConfigPaths) -> Result<(), KeybindingEditError> {
        if let Some(error) = &self.source_load_error {
            return Err(KeybindingEditError::InvalidSource(error.clone()));
        }
        let mut conflicts: Vec<_> =
            assignment_conflicts(&assigned_ui_keybinding_specs(&self.config, &self.registry))
                .into_iter()
                .map(|conflict| conflict.keys)
                .collect();
        conflicts.sort();
        conflicts.dedup();
        if !conflicts.is_empty() {
            return Err(KeybindingEditError::ConflictingBindings(conflicts));
        }

        let invalid = self.config.invalid_commands(&self.registry);
        if !invalid.is_empty() {
            return Err(KeybindingEditError::InvalidCommands(invalid));
        }

        let invalid_bindings = self.config.invalid_bindings();
        if !invalid_bindings.is_empty() {
            return Err(KeybindingEditError::InvalidBindings(invalid_bindings));
        }

        save_keybindings(paths, &self.config)?;
        Ok(())
    }
}

fn action_keys_from_bindings(
    bindings: &[UiKeybindingSpec],
    action: BindableActionId,
) -> Vec<String> {
    let mut keys: Vec<_> = bindings
        .iter()
        .filter(|binding| binding.command == action)
        .map(|binding| binding.keys.to_string())
        .collect();
    keys.sort();
    keys.dedup();
    keys
}

fn assignment_conflicts(bindings: &[UiKeybindingSpec]) -> Vec<AssignmentConflict> {
    let mut by_binding = HashMap::<(Option<String>, String), Vec<BindableActionId>>::new();
    for binding in bindings {
        let actions = by_binding
            .entry((
                binding.context.as_deref().map(str::to_string),
                normalize_keys(binding.keys.as_ref()),
            ))
            .or_default();
        if !actions.contains(&binding.command) {
            actions.push(binding.command);
        }
    }

    let mut assignments: Vec<_> = by_binding.into_iter().collect();
    assignments.sort_by(|left, right| left.0.cmp(&right.0));
    assignments
        .into_iter()
        .filter_map(|((_context, keys), actions)| {
            (actions.len() > 1).then_some(AssignmentConflict { keys, actions })
        })
        .collect()
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
