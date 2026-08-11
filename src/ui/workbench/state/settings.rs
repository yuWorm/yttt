use std::{collections::HashMap, rc::Rc};

use gpui::{Entity, Subscription};
use gpui_component::{VirtualListScrollHandle, input::InputState};

use crate::ui::app::platform::{self, PermissionKind, PermissionStatus};
use crate::ui::settings::{
    SettingsPageState,
    keybindings::{KeybindingProfile, KeybindingRow, KeybindingsEditorState},
};
use crate::ui::theme::zed::{ZedThemeDetection, ZedThemeImportConflictPolicy};

use super::super::{SettingsFontFamilySelectState, SettingsNumberField, SettingsStringSelectState};

#[derive(Clone)]
pub(in super::super) struct ZedThemeImportDialogState {
    pub(in super::super) detection: ZedThemeDetection,
    pub(in super::super) conflict_policy: ZedThemeImportConflictPolicy,
}

pub(in super::super) struct SettingsControllerState {
    pub(in super::super) keybinding_warning_lines: Vec<String>,
    pub(in super::super) keybinding_load_error: Option<String>,
    pub(in super::super) keybindings_editor: KeybindingsEditorState,
    pub(in super::super) keybinding_rows_cache: Option<Rc<Vec<KeybindingRow>>>,
    pub(in super::super) keybinding_scroll_handle: VirtualListScrollHandle,
    pub(in super::super) keybinding_profile: KeybindingProfile,
    pub(in super::super) keybinding_interceptor_subscription: Option<Subscription>,
    pub(in super::super) settings_search_input: Option<Entity<InputState>>,
    pub(in super::super) settings_search_input_subscription: Option<Subscription>,
    pub(in super::super) settings_search_input_needs_focus: bool,
    pub(in super::super) settings_language_select: Option<Entity<SettingsStringSelectState>>,
    pub(in super::super) settings_language_select_subscription: Option<Subscription>,
    pub(in super::super) settings_vim_mode_select: Option<Entity<SettingsStringSelectState>>,
    pub(in super::super) settings_vim_mode_select_subscription: Option<Subscription>,
    pub(in super::super) settings_shell_select: Option<Entity<SettingsStringSelectState>>,
    pub(in super::super) settings_shell_select_subscription: Option<Subscription>,
    pub(in super::super) settings_custom_shell_input: Option<Entity<InputState>>,
    pub(in super::super) settings_environment_name_input: Option<Entity<InputState>>,
    pub(in super::super) settings_environment_value_input: Option<Entity<InputState>>,
    pub(in super::super) settings_new_tab_command_input: Option<Entity<InputState>>,
    pub(in super::super) settings_window_effect_select: Option<Entity<SettingsStringSelectState>>,
    pub(in super::super) settings_window_effect_select_subscription: Option<Subscription>,
    pub(in super::super) settings_ui_theme_select: Option<Entity<SettingsStringSelectState>>,
    pub(in super::super) settings_ui_theme_select_subscription: Option<Subscription>,
    pub(in super::super) settings_ui_style_select: Option<Entity<SettingsStringSelectState>>,
    pub(in super::super) settings_ui_style_select_subscription: Option<Subscription>,
    pub(in super::super) settings_ui_font_family_select:
        Option<Entity<SettingsFontFamilySelectState>>,
    pub(in super::super) settings_ui_font_family_select_subscription: Option<Subscription>,
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
    pub(in super::super) zed_theme_import_dialog: Option<ZedThemeImportDialogState>,
    pub(in super::super) permission_statuses: [PermissionStatus; PermissionKind::COUNT],
    pub(in super::super) permission_request_attempted: [bool; PermissionKind::COUNT],
    pub(in super::super) permission_refreshing: bool,
    pub(in super::super) permission_statuses_loaded: bool,
    pub(in super::super) permission_refresh_generation: u64,
    pub(in super::super) permission_requesting: Option<PermissionKind>,
}

impl SettingsControllerState {
    pub(in super::super) fn new(
        keybinding_warning_lines: Vec<String>,
        keybindings_editor: KeybindingsEditorState,
        keybinding_load_error: Option<String>,
    ) -> Self {
        Self {
            keybinding_warning_lines,
            keybindings_editor,
            keybinding_load_error,
            keybinding_rows_cache: None,
            keybinding_scroll_handle: VirtualListScrollHandle::new(),
            keybinding_profile: KeybindingProfile::default(),
            keybinding_interceptor_subscription: None,
            settings_search_input: None,
            settings_search_input_subscription: None,
            settings_search_input_needs_focus: false,
            settings_language_select: None,
            settings_language_select_subscription: None,
            settings_vim_mode_select: None,
            settings_vim_mode_select_subscription: None,
            settings_shell_select: None,
            settings_shell_select_subscription: None,
            settings_custom_shell_input: None,
            settings_environment_name_input: None,
            settings_environment_value_input: None,
            settings_new_tab_command_input: None,
            settings_window_effect_select: None,
            settings_window_effect_select_subscription: None,
            settings_ui_theme_select: None,
            settings_ui_theme_select_subscription: None,
            settings_ui_style_select: None,
            settings_ui_style_select_subscription: None,
            settings_ui_font_family_select: None,
            settings_ui_font_family_select_subscription: None,
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
            permission_statuses: std::array::from_fn(|index| {
                platform::initial_permission_status(PermissionKind::ALL[index])
            }),
            permission_request_attempted: [false; PermissionKind::COUNT],
            permission_refreshing: false,
            permission_statuses_loaded: false,
            permission_refresh_generation: 0,
            permission_requesting: None,
            zed_theme_import_dialog: None,
        }
    }
}
