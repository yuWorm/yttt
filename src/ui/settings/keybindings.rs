use std::collections::{HashMap, HashSet};

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
        vim::VIM_PROFILE_CONTEXT,
    },
};

use super::keybinding_display::display_keybindings_for_current_platform;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum KeybindingProfile {
    #[default]
    Base,
    Vim,
}

impl KeybindingProfile {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Base => "base",
            Self::Vim => "vim",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeybindingOrigin {
    Builtin,
    User,
    Inherited,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum KeybindingDiagnosticKind {
    Conflict,
    Shadowed,
    Prefix,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeybindingAssignment {
    pub keys: String,
    pub context: Option<String>,
    pub origin: KeybindingOrigin,
    pub shadowed: bool,
}

impl KeybindingAssignment {
    pub fn display_keys(&self) -> Vec<String> {
        display_keybindings_for_current_platform(std::slice::from_ref(&self.keys))
    }

    pub fn scope_label(&self) -> &'static str {
        let Some(context) = self.context.as_deref() else {
            return "Global";
        };
        if context.contains("yttt_vim_surface == settings") {
            "Vim · Settings"
        } else if context.contains("yttt_vim_surface == terminal") {
            "Vim · Terminal"
        } else if context.contains("yttt_vim_surface == tree") {
            "Vim · Project Tree"
        } else if context.contains("yttt_vim_surface == palette") {
            "Vim · Palette"
        } else if context.contains("vim_mode == insert")
            || context.contains("yttt_vim_mode == insert")
        {
            "Vim · Insert"
        } else if context.contains("vim_mode == normal")
            || context.contains("yttt_vim_mode == normal")
        {
            "Vim · Normal"
        } else if context.contains("VimControl") || context.contains("yttt_vim_control == true") {
            "Vim · Normal / Visual"
        } else if is_vim_context(Some(context)) {
            "Vim"
        } else if context.contains("Terminal") {
            "Terminal"
        } else if context.contains("Tree") {
            "Project Tree"
        } else if context.contains("Palette") {
            "Palette"
        } else if context.contains("Editor") {
            "Editor"
        } else {
            "Workspace"
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeybindingRow {
    pub command: BindableActionId,
    pub command_id: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    pub keys: Vec<String>,
    pub assignments: Vec<KeybindingAssignment>,
    pub diagnostics: Vec<KeybindingDiagnosticKind>,
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
        self.build_rows(None, text)
    }

    pub fn rows_for_profile(
        &self,
        profile: KeybindingProfile,
        text: &UiText,
    ) -> Vec<KeybindingRow> {
        self.build_rows(Some(profile), text)
    }

    fn build_rows(&self, profile: Option<KeybindingProfile>, text: &UiText) -> Vec<KeybindingRow> {
        let bindings = assigned_ui_keybinding_specs(&self.config, &self.registry);
        let user_assignments = user_binding_assignments(&self.config);
        let vim_unbinds = vim_inherited_unbinds(&self.config);
        let diagnostics = assignment_diagnostics(&bindings, profile);

        BindableActionId::all()
            .filter(|action| self.registry.contains_str(action.as_str()))
            .map(|action| {
                let assignments = assignments_for_action(
                    &bindings,
                    action,
                    profile,
                    &user_assignments,
                    &vim_unbinds,
                );
                let keys = normalize_key_list(
                    assignments
                        .iter()
                        .filter(|assignment| !assignment.shadowed)
                        .map(|assignment| assignment.keys.clone())
                        .collect(),
                );
                let diagnostics = diagnostics.get(&action).cloned().unwrap_or_default();
                let has_conflict = diagnostics.contains(&KeybindingDiagnosticKind::Conflict);
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
                    assignments,
                    diagnostics,
                    has_conflict,
                }
            })
            .collect()
    }

    pub fn action_keys(&self, action: BindableActionId) -> Vec<String> {
        let bindings = assigned_ui_keybinding_specs(&self.config, &self.registry);
        action_keys_from_bindings(&bindings, action)
    }

    pub fn action_keys_for_profile(
        &self,
        action: BindableActionId,
        profile: KeybindingProfile,
    ) -> Vec<String> {
        let bindings = assigned_ui_keybinding_specs(&self.config, &self.registry);
        let vim_unbinds = vim_inherited_unbinds(&self.config);
        action_keys_for_profile_from_bindings(&bindings, action, profile, &vim_unbinds)
    }

    pub fn conflicting_keys_for_profile(
        &self,
        action: BindableActionId,
        profile: KeybindingProfile,
        keys: Vec<String>,
    ) -> Vec<String> {
        let mut preview = self.clone();
        preview.set_action_keys_for_profile(action, profile, keys);
        let bindings = assigned_ui_keybinding_specs(&preview.config, &preview.registry);
        let mut conflicts = assignment_conflicts(&bindings)
            .into_iter()
            .filter(|conflict| conflict.actions.contains(&action))
            .map(|conflict| conflict.keys)
            .collect::<Vec<_>>();
        conflicts.sort();
        conflicts.dedup();
        conflicts
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

    pub fn set_action_keys_for_profile(
        &mut self,
        action: BindableActionId,
        profile: KeybindingProfile,
        keys: Vec<String>,
    ) {
        match profile {
            KeybindingProfile::Base => self.set_base_action_keys(action, keys),
            KeybindingProfile::Vim => self.set_vim_action_keys(action, keys),
        }
    }

    fn set_base_action_keys(&mut self, action: BindableActionId, keys: Vec<String>) {
        let keys = normalize_key_list(keys);
        let current_bindings = assigned_ui_keybinding_specs(&self.config, &self.registry);
        let current_keys = action_keys_for_profile_from_bindings(
            &current_bindings,
            action,
            KeybindingProfile::Base,
            &HashSet::new(),
        );
        if keys == current_keys {
            return;
        }

        let mut current_contexts =
            binding_contexts_for_action(&current_bindings, action, KeybindingProfile::Base);
        let defaults = default_bindings_for_action_with_leader(action, &self.config.leader)
            .into_iter()
            .filter(|binding| !is_vim_context(binding.context.as_deref()))
            .collect::<Vec<_>>();
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

        self.remove_action_overrides_for_profile(action, KeybindingProfile::Base);
        append_removed_default_unbinds(&mut self.config, action, &defaults, &desired);
        append_desired_bindings(&mut self.config, action, &defaults, desired);
    }

    fn set_vim_action_keys(&mut self, action: BindableActionId, keys: Vec<String>) {
        let keys = normalize_key_list(keys);
        let current_bindings = assigned_ui_keybinding_specs(&self.config, &self.registry);
        let vim_unbinds = vim_inherited_unbinds(&self.config);
        let current_keys = action_keys_for_profile_from_bindings(
            &current_bindings,
            action,
            KeybindingProfile::Vim,
            &vim_unbinds,
        );
        if keys == current_keys {
            return;
        }

        let mut current_contexts =
            binding_contexts_for_action(&current_bindings, action, KeybindingProfile::Vim);
        let defaults = default_bindings_for_action_with_leader(action, &self.config.leader)
            .into_iter()
            .filter(|binding| is_vim_context(binding.context.as_deref()))
            .collect::<Vec<_>>();
        let base_keys = current_bindings
            .iter()
            .filter(|binding| {
                binding.command == action && !is_vim_context(binding.context.as_deref())
            })
            .map(|binding| normalize_keys(binding.keys.as_ref()))
            .collect::<HashSet<_>>();
        let desired_keys = keys.iter().cloned().collect::<HashSet<_>>();

        self.remove_action_overrides_for_profile(action, KeybindingProfile::Vim);
        for binding in &defaults {
            let key = normalize_keys(binding.keys.as_ref());
            if !desired_keys.contains(&key) {
                self.config.bindings.push(Keybinding {
                    keys: key,
                    command: action.as_str().to_string(),
                    context: binding.context.as_deref().map(str::to_string),
                    unbind: true,
                });
            }
        }
        for key in base_keys.iter().filter(|key| !desired_keys.contains(*key)) {
            self.config.bindings.push(Keybinding {
                keys: key.clone(),
                command: action.as_str().to_string(),
                context: Some(VIM_PROFILE_CONTEXT.to_string()),
                unbind: true,
            });
        }

        for key in keys {
            let default_for_key = defaults
                .iter()
                .any(|binding| normalize_keys(binding.keys.as_ref()) == key);
            let inherited_without_shadow =
                base_keys.contains(&key) && !key_is_shadowed(&current_bindings, action, &key);
            if default_for_key || inherited_without_shadow {
                continue;
            }

            let contexts = current_contexts
                .remove(&key)
                .unwrap_or_else(|| vec![preferred_vim_context_for_action(action, &defaults)]);
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

    pub fn delete_action_keys_for_profile(
        &mut self,
        action: BindableActionId,
        profile: KeybindingProfile,
    ) {
        self.set_action_keys_for_profile(action, profile, Vec::new());
    }

    pub fn reset_action_keys_for_profile(
        &mut self,
        action: BindableActionId,
        profile: KeybindingProfile,
    ) {
        self.remove_action_overrides_for_profile(action, profile);
    }

    fn remove_action_overrides_for_profile(
        &mut self,
        action: BindableActionId,
        profile: KeybindingProfile,
    ) {
        self.config.bindings.retain(|binding| {
            binding.command != action.as_str()
                || profile_for_context(binding.context.as_deref()) != profile
        });
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

fn action_keys_for_profile_from_bindings(
    bindings: &[UiKeybindingSpec],
    action: BindableActionId,
    profile: KeybindingProfile,
    vim_unbinds: &HashSet<(BindableActionId, String)>,
) -> Vec<String> {
    match profile {
        KeybindingProfile::Base => normalize_key_list(
            bindings
                .iter()
                .filter(|binding| {
                    binding.command == action && !is_vim_context(binding.context.as_deref())
                })
                .map(|binding| binding.keys.to_string())
                .collect(),
        ),
        KeybindingProfile::Vim => {
            let explicit = bindings
                .iter()
                .filter(|binding| {
                    binding.command == action && is_vim_context(binding.context.as_deref())
                })
                .map(|binding| normalize_keys(binding.keys.as_ref()))
                .collect::<HashSet<_>>();
            let mut keys = explicit.iter().cloned().collect::<Vec<_>>();
            keys.extend(
                bindings
                    .iter()
                    .filter(|binding| {
                        binding.command == action
                            && !vim_unbinds
                                .contains(&(action, normalize_keys(binding.keys.as_ref())))
                            && (!key_is_shadowed(
                                bindings,
                                action,
                                &normalize_keys(binding.keys.as_ref()),
                            ) || explicit.contains(&normalize_keys(binding.keys.as_ref())))
                    })
                    .map(|binding| binding.keys.to_string()),
            );
            normalize_key_list(keys)
        }
    }
}

fn assignments_for_action(
    bindings: &[UiKeybindingSpec],
    action: BindableActionId,
    profile: Option<KeybindingProfile>,
    user_assignments: &HashSet<(BindableActionId, String, Option<String>)>,
    vim_unbinds: &HashSet<(BindableActionId, String)>,
) -> Vec<KeybindingAssignment> {
    let mut assignments = Vec::new();
    for binding in bindings.iter().filter(|binding| binding.command == action) {
        let vim_context = is_vim_context(binding.context.as_deref());
        let include = match profile {
            None => true,
            Some(KeybindingProfile::Base) => !vim_context,
            Some(KeybindingProfile::Vim) => true,
        };
        if !include {
            continue;
        }
        let keys = normalize_keys(binding.keys.as_ref());
        let context = normalize_context(binding.context.as_deref());
        let explicit_user = user_assignments.contains(&(action, keys.clone(), context.clone()));
        let inherited = profile == Some(KeybindingProfile::Vim) && !vim_context;
        assignments.push(KeybindingAssignment {
            shadowed: inherited
                && (vim_unbinds.contains(&(action, keys.clone()))
                    || key_is_shadowed(bindings, action, &keys)),
            keys,
            context,
            origin: if inherited {
                KeybindingOrigin::Inherited
            } else if explicit_user {
                KeybindingOrigin::User
            } else {
                KeybindingOrigin::Builtin
            },
        });
    }
    assignments.sort_by(|left, right| {
        left.shadowed
            .cmp(&right.shadowed)
            .then(left.keys.cmp(&right.keys))
            .then(left.context.cmp(&right.context))
    });
    assignments.dedup_by(|left, right| {
        left.keys == right.keys
            && left.context == right.context
            && left.origin == right.origin
            && left.shadowed == right.shadowed
    });
    assignments
}

fn user_binding_assignments(
    config: &KeybindingsConfig,
) -> HashSet<(BindableActionId, String, Option<String>)> {
    config
        .bindings
        .iter()
        .filter(|binding| !binding.unbind)
        .filter_map(|binding| {
            Some((
                BindableActionId::from_str_id(binding.command.trim())?,
                normalize_keys(&binding.keys),
                normalize_context(binding.context.as_deref()),
            ))
        })
        .collect()
}

fn vim_inherited_unbinds(config: &KeybindingsConfig) -> HashSet<(BindableActionId, String)> {
    config
        .bindings
        .iter()
        .filter(|binding| binding.unbind && is_vim_context(binding.context.as_deref()))
        .filter_map(|binding| {
            Some((
                BindableActionId::from_str_id(binding.command.trim())?,
                normalize_keys(&binding.keys),
            ))
        })
        .collect()
}

fn assignment_diagnostics(
    bindings: &[UiKeybindingSpec],
    profile: Option<KeybindingProfile>,
) -> HashMap<BindableActionId, Vec<KeybindingDiagnosticKind>> {
    let relevant = bindings
        .iter()
        .filter(|binding| match profile {
            None => true,
            Some(KeybindingProfile::Base) => !is_vim_context(binding.context.as_deref()),
            Some(KeybindingProfile::Vim) => true,
        })
        .collect::<Vec<_>>();
    let mut diagnostics = HashMap::<BindableActionId, HashSet<KeybindingDiagnosticKind>>::new();

    for conflict in assignment_conflicts(
        &relevant
            .iter()
            .map(|binding| (*binding).clone())
            .collect::<Vec<_>>(),
    ) {
        for action in conflict.actions {
            diagnostics
                .entry(action)
                .or_default()
                .insert(KeybindingDiagnosticKind::Conflict);
        }
    }

    if profile != Some(KeybindingProfile::Base) {
        for base in relevant
            .iter()
            .filter(|binding| !is_vim_context(binding.context.as_deref()))
        {
            for vim in relevant.iter().filter(|binding| {
                is_vim_context(binding.context.as_deref())
                    && normalize_keys(binding.keys.as_ref()) == normalize_keys(base.keys.as_ref())
                    && binding.command != base.command
            }) {
                diagnostics
                    .entry(base.command)
                    .or_default()
                    .insert(KeybindingDiagnosticKind::Shadowed);
                diagnostics
                    .entry(vim.command)
                    .or_default()
                    .insert(KeybindingDiagnosticKind::Shadowed);
            }
        }
    }

    for (index, left) in relevant.iter().enumerate() {
        for right in relevant.iter().skip(index + 1) {
            let left_keys = normalize_keys(left.keys.as_ref());
            let right_keys = normalize_keys(right.keys.as_ref());
            if !contexts_can_overlap(left, right, profile)
                || !(key_sequence_is_prefix(&left_keys, &right_keys)
                    || key_sequence_is_prefix(&right_keys, &left_keys))
            {
                continue;
            }
            diagnostics
                .entry(left.command)
                .or_default()
                .insert(KeybindingDiagnosticKind::Prefix);
            diagnostics
                .entry(right.command)
                .or_default()
                .insert(KeybindingDiagnosticKind::Prefix);
        }
    }

    diagnostics
        .into_iter()
        .map(|(action, diagnostics)| {
            let mut diagnostics = diagnostics.into_iter().collect::<Vec<_>>();
            diagnostics.sort_by_key(|diagnostic| match diagnostic {
                KeybindingDiagnosticKind::Conflict => 0,
                KeybindingDiagnosticKind::Shadowed => 1,
                KeybindingDiagnosticKind::Prefix => 2,
            });
            (action, diagnostics)
        })
        .collect()
}

fn contexts_can_overlap(
    left: &UiKeybindingSpec,
    right: &UiKeybindingSpec,
    profile: Option<KeybindingProfile>,
) -> bool {
    let left_context = normalize_context(left.context.as_deref());
    let right_context = normalize_context(right.context.as_deref());
    left_context == right_context
        || (profile == Some(KeybindingProfile::Vim)
            && is_vim_context(left.context.as_deref()) != is_vim_context(right.context.as_deref()))
}

fn key_sequence_is_prefix(prefix: &str, sequence: &str) -> bool {
    prefix != sequence
        && sequence
            .strip_prefix(prefix)
            .is_some_and(|suffix| suffix.starts_with(' '))
}

fn key_is_shadowed(bindings: &[UiKeybindingSpec], action: BindableActionId, keys: &str) -> bool {
    bindings.iter().any(|binding| {
        binding.command != action
            && is_vim_context(binding.context.as_deref())
            && normalize_keys(binding.keys.as_ref()) == keys
    })
}

fn binding_contexts_for_action(
    bindings: &[UiKeybindingSpec],
    action: BindableActionId,
    profile: KeybindingProfile,
) -> HashMap<String, Vec<Option<String>>> {
    let mut contexts = HashMap::<String, Vec<Option<String>>>::new();
    for binding in bindings.iter().filter(|binding| {
        binding.command == action && profile_for_context(binding.context.as_deref()) == profile
    }) {
        let contexts_for_key = contexts
            .entry(normalize_keys(binding.keys.as_ref()))
            .or_default();
        let context = normalize_context(binding.context.as_deref());
        if !contexts_for_key.contains(&context) {
            contexts_for_key.push(context);
        }
    }
    contexts
}

fn preferred_vim_context_for_action(
    action: BindableActionId,
    defaults: &[UiKeybindingSpec],
) -> Option<String> {
    if action.command().is_some() {
        Some(VIM_PROFILE_CONTEXT.to_string())
    } else {
        defaults
            .iter()
            .find_map(|binding| normalize_context(binding.context.as_deref()))
            .or_else(|| Some(VIM_PROFILE_CONTEXT.to_string()))
    }
}

fn append_removed_default_unbinds(
    config: &mut KeybindingsConfig,
    action: BindableActionId,
    defaults: &[UiKeybindingSpec],
    desired: &[(String, Vec<Option<String>>)],
) {
    for binding in defaults {
        let binding_context = normalize_context(binding.context.as_deref());
        let preserves_default = desired.iter().any(|(key, contexts)| {
            normalize_keys(binding.keys.as_ref()) == *key && contexts.contains(&binding_context)
        });
        if !preserves_default {
            config.bindings.push(Keybinding {
                keys: binding.keys.to_string(),
                command: action.as_str().to_string(),
                context: binding_context,
                unbind: true,
            });
        }
    }
}

fn append_desired_bindings(
    config: &mut KeybindingsConfig,
    action: BindableActionId,
    defaults: &[UiKeybindingSpec],
    desired: Vec<(String, Vec<Option<String>>)>,
) {
    for (key, contexts) in desired {
        for context in contexts {
            if defaults.iter().any(|binding| {
                normalize_keys(binding.keys.as_ref()) == key
                    && binding.context.as_deref() == context.as_deref()
            }) {
                continue;
            }
            config.bindings.push(Keybinding {
                keys: key.clone(),
                command: action.as_str().to_string(),
                context,
                unbind: false,
            });
        }
    }
}

fn profile_for_context(context: Option<&str>) -> KeybindingProfile {
    if is_vim_context(context) {
        KeybindingProfile::Vim
    } else {
        KeybindingProfile::Base
    }
}

fn is_vim_context(context: Option<&str>) -> bool {
    context.is_some_and(|context| {
        context.contains("YtttVim")
            || context.contains("WorkspaceVim")
            || context.contains("VimEditor")
            || context.contains("VimControl")
            || context.contains("vim_mode")
            || context.contains("vim_operator")
    })
}

fn normalize_context(context: Option<&str>) -> Option<String> {
    context
        .map(str::trim)
        .filter(|context| !context.is_empty())
        .map(str::to_string)
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
