use std::collections::HashMap;

use gpui::{Entity, Subscription};
use gpui_component::input::InputState;

use crate::ui::settings::{SettingsPageState, keybindings::KeybindingsEditorState};

use super::super::{SettingsFontFamilySelectState, SettingsNumberField, SettingsStringSelectState};

pub(in super::super) struct SettingsControllerState {
    pub(in super::super) keybinding_warning_lines: Vec<String>,
    pub(in super::super) keybindings_editor: KeybindingsEditorState,
    pub(in super::super) keybinding_interceptor_subscription: Option<Subscription>,
    pub(in super::super) settings_search_input: Option<Entity<InputState>>,
    pub(in super::super) settings_search_input_subscription: Option<Subscription>,
    pub(in super::super) settings_search_input_needs_focus: bool,
    pub(in super::super) settings_language_select: Option<Entity<SettingsStringSelectState>>,
    pub(in super::super) settings_language_select_subscription: Option<Subscription>,
    pub(in super::super) settings_shell_select: Option<Entity<SettingsStringSelectState>>,
    pub(in super::super) settings_shell_select_subscription: Option<Subscription>,
    pub(in super::super) settings_custom_shell_input: Option<Entity<InputState>>,
    pub(in super::super) settings_ui_theme_select: Option<Entity<SettingsStringSelectState>>,
    pub(in super::super) settings_ui_theme_select_subscription: Option<Subscription>,
    pub(in super::super) settings_icon_theme_select: Option<Entity<SettingsStringSelectState>>,
    pub(in super::super) settings_icon_theme_select_subscription: Option<Subscription>,
    pub(in super::super) settings_terminal_theme_select: Option<Entity<SettingsStringSelectState>>,
    pub(in super::super) settings_terminal_theme_select_subscription: Option<Subscription>,
    pub(in super::super) settings_terminal_cursor_shape_select:
        Option<Entity<SettingsStringSelectState>>,
    pub(in super::super) settings_terminal_cursor_shape_select_subscription: Option<Subscription>,
    pub(in super::super) settings_terminal_osc52_policy_select:
        Option<Entity<SettingsStringSelectState>>,
    pub(in super::super) settings_terminal_osc52_policy_select_subscription: Option<Subscription>,
    pub(in super::super) settings_editor_language_select: Option<Entity<SettingsStringSelectState>>,
    pub(in super::super) settings_editor_language_select_subscription: Option<Subscription>,
    pub(in super::super) settings_font_family_select: Option<Entity<SettingsFontFamilySelectState>>,
    pub(in super::super) settings_font_family_select_subscription: Option<Subscription>,
    pub(in super::super) settings_editor_font_family_select:
        Option<Entity<SettingsFontFamilySelectState>>,
    pub(in super::super) settings_editor_font_family_select_subscription: Option<Subscription>,
    pub(in super::super) settings_editor_autosave_select: Option<Entity<SettingsStringSelectState>>,
    pub(in super::super) settings_editor_autosave_select_subscription: Option<Subscription>,
    pub(in super::super) settings_number_inputs: HashMap<SettingsNumberField, Entity<InputState>>,
    pub(in super::super) settings_number_input_subscriptions:
        HashMap<SettingsNumberField, Vec<Subscription>>,
    pub(in super::super) settings_page: SettingsPageState,
}

impl SettingsControllerState {
    pub(in super::super) fn new(
        keybinding_warning_lines: Vec<String>,
        keybindings_editor: KeybindingsEditorState,
    ) -> Self {
        Self {
            keybinding_warning_lines,
            keybindings_editor,
            keybinding_interceptor_subscription: None,
            settings_search_input: None,
            settings_search_input_subscription: None,
            settings_search_input_needs_focus: false,
            settings_language_select: None,
            settings_language_select_subscription: None,
            settings_shell_select: None,
            settings_shell_select_subscription: None,
            settings_custom_shell_input: None,
            settings_ui_theme_select: None,
            settings_ui_theme_select_subscription: None,
            settings_icon_theme_select: None,
            settings_icon_theme_select_subscription: None,
            settings_terminal_theme_select: None,
            settings_terminal_theme_select_subscription: None,
            settings_terminal_cursor_shape_select: None,
            settings_terminal_cursor_shape_select_subscription: None,
            settings_terminal_osc52_policy_select: None,
            settings_terminal_osc52_policy_select_subscription: None,
            settings_editor_language_select: None,
            settings_editor_language_select_subscription: None,
            settings_font_family_select: None,
            settings_font_family_select_subscription: None,
            settings_editor_font_family_select: None,
            settings_editor_font_family_select_subscription: None,
            settings_editor_autosave_select: None,
            settings_editor_autosave_select_subscription: None,
            settings_number_inputs: HashMap::new(),
            settings_number_input_subscriptions: HashMap::new(),
            settings_page: SettingsPageState::default(),
        }
    }
}
