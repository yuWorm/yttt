pub mod font_options;
pub mod keybinding_display;
pub mod keybindings;

use crate::{
    config::scope::{
        SettingApply, SettingsScope, setting_apply, setting_scope, supports_project_override,
    },
    ui::i18n::{UiText, UiTextKey},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsGroupId {
    General,
    Appearance,
    Languages,
    Editor,
    Terminal,
    Agent,
    Permissions,
    ProjectLayout,
    DefaultLayout,
    Keybindings,
}

impl SettingsGroupId {
    pub const ALL: &'static [Self] = &[
        Self::General,
        Self::Appearance,
        Self::Languages,
        Self::Editor,
        Self::Terminal,
        Self::Agent,
        Self::Permissions,
        Self::ProjectLayout,
        Self::DefaultLayout,
        Self::Keybindings,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::General => "general",
            Self::Appearance => "appearance",
            Self::Languages => "languages",
            Self::Editor => "editor",
            Self::Terminal => "terminal",
            Self::Agent => "agent",
            Self::Permissions => "permissions",
            Self::ProjectLayout => "project-layout",
            Self::DefaultLayout => "default-layout",
            Self::Keybindings => "keybindings",
        }
    }

    pub fn title_key(self) -> UiTextKey {
        match self {
            Self::General => UiTextKey::SettingsGroupGeneral,
            Self::Appearance => UiTextKey::SettingsGroupAppearance,
            Self::Languages => UiTextKey::SettingsGroupLanguages,
            Self::Editor => UiTextKey::SettingsGroupEditor,
            Self::Terminal => UiTextKey::SettingsGroupTerminal,
            Self::Agent => UiTextKey::SettingsGroupAgent,
            Self::Permissions => UiTextKey::SettingsGroupPermissions,
            Self::ProjectLayout => UiTextKey::SettingsGroupProjectLayout,
            Self::DefaultLayout => UiTextKey::SettingsGroupDefaultLayout,
            Self::Keybindings => UiTextKey::SettingsGroupKeybindings,
        }
    }

    pub fn description_key(self) -> UiTextKey {
        match self {
            Self::General => UiTextKey::SettingsGroupGeneralDescription,
            Self::Appearance => UiTextKey::SettingsGroupAppearanceDescription,
            Self::Languages => UiTextKey::SettingsGroupLanguagesDescription,
            Self::Editor => UiTextKey::SettingsGroupEditorDescription,
            Self::Terminal => UiTextKey::SettingsGroupTerminalDescription,
            Self::Agent => UiTextKey::SettingsGroupAgentDescription,
            Self::Permissions => UiTextKey::SettingsGroupPermissionsDescription,
            Self::ProjectLayout => UiTextKey::SettingsGroupProjectLayoutDescription,
            Self::DefaultLayout => UiTextKey::SettingsGroupDefaultLayoutDescription,
            Self::Keybindings => UiTextKey::SettingsGroupKeybindingsDescription,
        }
    }

    pub fn title(self, text: &UiText) -> &'static str {
        text.get(self.title_key())
    }

    pub fn description(self, text: &UiText) -> &'static str {
        text.get(self.description_key())
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|group| group.as_str() == id)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct SettingsRowMeta {
    pub key: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    pub scope: SettingsScope,
    pub apply: SettingApply,
    pub project_override: bool,
}

impl SettingsRowMeta {
    pub fn is_visible_for(self, scope: SettingsScope) -> bool {
        self.scope == scope || (scope == SettingsScope::Project && self.project_override)
    }
}

pub fn settings_rows_for_group(group: SettingsGroupId, text: &UiText) -> Vec<SettingsRowMeta> {
    let row = |key, title, description| SettingsRowMeta {
        key,
        title: text.get(title),
        description: text.get(description),
        scope: setting_scope(key),
        apply: setting_apply(key),
        project_override: supports_project_override(key),
    };

    match group {
        SettingsGroupId::General => vec![
            row(
                "general.language",
                UiTextKey::SettingsLanguage,
                UiTextKey::SettingsLanguageDescription,
            ),
            row(
                "vim.mode",
                UiTextKey::SettingsVimMode,
                UiTextKey::SettingsVimModeDescription,
            ),
            row(
                "notifications.system",
                UiTextKey::SettingsSystemNotifications,
                UiTextKey::SettingsSystemNotificationsDescription,
            ),
            row(
                "updates.check",
                UiTextKey::SettingsUpdates,
                UiTextKey::SettingsUpdatesDescription,
            ),
            row(
                "updates.auto_check",
                UiTextKey::SettingsAutoCheckUpdates,
                UiTextKey::SettingsAutoCheckUpdatesDescription,
            ),
            row(
                "general.restore_last_session",
                UiTextKey::SettingsRestoreLastSession,
                UiTextKey::SettingsRestoreLastSessionDescription,
            ),
            row(
                "general.new_tab_command_picker_enabled",
                UiTextKey::SettingsNewTabCommandPicker,
                UiTextKey::SettingsNewTabCommandPickerDescription,
            ),
            row(
                "general.new_tab_commands",
                UiTextKey::SettingsNewTabCommands,
                UiTextKey::SettingsNewTabCommandsDescription,
            ),
            row(
                "connections.remote_services",
                UiTextKey::RemoteServices,
                UiTextKey::RemoteServicesDescription,
            ),
        ],
        SettingsGroupId::Appearance => vec![
            row(
                "window.effect",
                UiTextKey::SettingsWindowEffect,
                UiTextKey::SettingsWindowEffectDescription,
            ),
            row(
                "window.opacity",
                UiTextKey::SettingsWindowOpacity,
                UiTextKey::SettingsWindowOpacityDescription,
            ),
            row(
                "general.ui_font_family",
                UiTextKey::SettingsUiFontFamily,
                UiTextKey::SettingsUiFontFamilyDescription,
            ),
            row(
                "general.ui_font_size",
                UiTextKey::SettingsUiFontSize,
                UiTextKey::SettingsUiFontSizeDescription,
            ),
            row(
                "general.ui_line_height",
                UiTextKey::SettingsUiLineHeight,
                UiTextKey::SettingsUiLineHeightDescription,
            ),
            row(
                "theme.name",
                UiTextKey::SettingsUiTheme,
                UiTextKey::SettingsUiThemeDescription,
            ),
            row(
                "theme.ui_style",
                UiTextKey::SettingsUiStyle,
                UiTextKey::SettingsUiStyleDescription,
            ),
            row(
                "theme.icon_theme",
                UiTextKey::SettingsIconTheme,
                UiTextKey::SettingsIconThemeDescription,
            ),
            row(
                "theme.terminal",
                UiTextKey::SettingsTerminalTheme,
                UiTextKey::SettingsTerminalThemeDescription,
            ),
            row(
                "settings.toml",
                UiTextKey::SettingsEditSettingsToml,
                UiTextKey::SettingsEditSettingsTomlDescription,
            ),
            row(
                "themes.directory",
                UiTextKey::SettingsThemesDirectory,
                UiTextKey::SettingsThemesDirectoryDescription,
            ),
            row(
                "themes.import_zed",
                UiTextKey::SettingsImportZedThemes,
                UiTextKey::SettingsImportZedThemesDescription,
            ),
        ],
        SettingsGroupId::Languages => vec![
            row(
                "editor.auto_detect_language",
                UiTextKey::SettingsLanguageDetection,
                UiTextKey::SettingsLanguageDetectionDescription,
            ),
            row(
                "editor.default_language",
                UiTextKey::SettingsDefaultCodeLanguage,
                UiTextKey::SettingsDefaultCodeLanguageDescription,
            ),
            row(
                "editor.supported_languages",
                UiTextKey::SettingsSupportedLanguages,
                UiTextKey::SettingsSupportedLanguagesDescription,
            ),
            row(
                "editor.lsp.enabled",
                UiTextKey::SettingsLanguageServer,
                UiTextKey::SettingsLanguageServerDescription,
            ),
            row(
                "editor.lsp.command",
                UiTextKey::SettingsLanguageServerCommand,
                UiTextKey::SettingsLanguageServerCommandDescription,
            ),
        ],
        SettingsGroupId::Editor => vec![
            row(
                "editor.font_family",
                UiTextKey::SettingsEditorFontFamily,
                UiTextKey::SettingsEditorFontFamilyDescription,
            ),
            row(
                "editor.font_size",
                UiTextKey::SettingsEditorFontSize,
                UiTextKey::SettingsEditorFontSizeDescription,
            ),
            row(
                "editor.line_height",
                UiTextKey::SettingsEditorLineHeight,
                UiTextKey::SettingsEditorLineHeightDescription,
            ),
            row(
                "editor.tab_size",
                UiTextKey::SettingsEditorTabSize,
                UiTextKey::SettingsEditorTabSizeDescription,
            ),
            row(
                "editor.soft_wrap",
                UiTextKey::SettingsEditorSoftWrap,
                UiTextKey::SettingsEditorSoftWrapDescription,
            ),
            row(
                "editor.line_numbers",
                UiTextKey::SettingsEditorLineNumbers,
                UiTextKey::SettingsEditorLineNumbersDescription,
            ),
            row(
                "editor.autosave",
                UiTextKey::SettingsEditorAutosave,
                UiTextKey::SettingsEditorAutosaveDescription,
            ),
            row(
                "editor.autosave_delay_ms",
                UiTextKey::SettingsEditorAutosaveDelay,
                UiTextKey::SettingsEditorAutosaveDelayDescription,
            ),
            row(
                "project_panel.default_open",
                UiTextKey::SettingsProjectPanelDefaultOpen,
                UiTextKey::SettingsProjectPanelDefaultOpenDescription,
            ),
            row(
                "project_panel.show_hidden",
                UiTextKey::SettingsProjectPanelShowHidden,
                UiTextKey::SettingsProjectPanelShowHiddenDescription,
            ),
            row(
                "project_panel.width",
                UiTextKey::SettingsProjectPanelWidth,
                UiTextKey::SettingsProjectPanelWidthDescription,
            ),
            row(
                "project_panel.project_sidebar_width",
                UiTextKey::SettingsProjectSidebarWidth,
                UiTextKey::SettingsProjectSidebarWidthDescription,
            ),
        ],
        SettingsGroupId::Terminal => vec![
            row(
                "terminal.shell",
                UiTextKey::SettingsDefaultShell,
                UiTextKey::SettingsDefaultShellDescription,
            ),
            row(
                "terminal.custom_shells",
                UiTextKey::SettingsCustomShell,
                UiTextKey::SettingsCustomShellDescription,
            ),
            row(
                "terminal.environment",
                UiTextKey::SettingsEnvironmentVariables,
                UiTextKey::SettingsEnvironmentVariablesDescription,
            ),
            row(
                "terminal.font_family",
                UiTextKey::SettingsFontFamily,
                UiTextKey::SettingsFontFamilyDescription,
            ),
            row(
                "terminal.font_size",
                UiTextKey::SettingsFontSize,
                UiTextKey::SettingsFontSizeDescription,
            ),
            row(
                "terminal.line_height",
                UiTextKey::SettingsLineHeight,
                UiTextKey::SettingsLineHeightDescription,
            ),
            row(
                "terminal.padding",
                UiTextKey::SettingsPadding,
                UiTextKey::SettingsPaddingDescription,
            ),
            row(
                "terminal.scrollback",
                UiTextKey::SettingsScrollback,
                UiTextKey::SettingsScrollbackDescription,
            ),
            row(
                "terminal.show_scrollbar",
                UiTextKey::SettingsScrollbar,
                UiTextKey::SettingsScrollbarDescription,
            ),
            row(
                "terminal.cursor_shape",
                UiTextKey::SettingsTerminalCursorShape,
                UiTextKey::SettingsTerminalCursorShapeDescription,
            ),
            row(
                "terminal.cursor_blinking",
                UiTextKey::SettingsTerminalCursorBlinking,
                UiTextKey::SettingsTerminalCursorBlinkingDescription,
            ),
            row(
                "terminal.hide_mouse_when_typing",
                UiTextKey::SettingsTerminalHideMouseWhenTyping,
                UiTextKey::SettingsTerminalHideMouseWhenTypingDescription,
            ),
            row(
                "terminal.copy_on_select",
                UiTextKey::SettingsTerminalCopyOnSelect,
                UiTextKey::SettingsTerminalCopyOnSelectDescription,
            ),
            row(
                "terminal.osc52_policy",
                UiTextKey::SettingsTerminalOsc52Policy,
                UiTextKey::SettingsTerminalOsc52PolicyDescription,
            ),
            row(
                "terminal.kitty_keyboard",
                UiTextKey::SettingsTerminalKittyKeyboard,
                UiTextKey::SettingsTerminalKittyKeyboardDescription,
            ),
        ],
        SettingsGroupId::Agent => vec![
            row(
                "agent.primary",
                UiTextKey::SettingsAgentPrimary,
                UiTextKey::SettingsAgentPrimaryDescription,
            ),
            row(
                "agent.sessions",
                UiTextKey::SettingsAgentSessions,
                UiTextKey::SettingsAgentSessionsDescription,
            ),
        ],
        SettingsGroupId::Permissions => vec![
            row(
                "permissions.login_startup",
                UiTextKey::SettingsLoginStartup,
                UiTextKey::SettingsLoginStartupDescription,
            ),
            row(
                "permissions.status",
                UiTextKey::SettingsPermissionStatus,
                UiTextKey::SettingsPermissionStatusDescription,
            ),
            row(
                "permissions.notifications",
                UiTextKey::SettingsPermissionNotifications,
                UiTextKey::SettingsPermissionNotificationsDescription,
            ),
            row(
                "permissions.file_system",
                UiTextKey::SettingsPermissionFileSystem,
                UiTextKey::SettingsPermissionFileSystemDescription,
            ),
            row(
                "permissions.developer_tools",
                UiTextKey::SettingsPermissionDeveloperTools,
                UiTextKey::SettingsPermissionDeveloperToolsDescription,
            ),
            row(
                "permissions.accessibility",
                UiTextKey::SettingsPermissionAccessibility,
                UiTextKey::SettingsPermissionAccessibilityDescription,
            ),
            row(
                "permissions.screen_capture",
                UiTextKey::SettingsPermissionScreenCapture,
                UiTextKey::SettingsPermissionScreenCaptureDescription,
            ),
        ],
        SettingsGroupId::ProjectLayout => vec![
            SettingsRowMeta {
                key: "project_layout.edit",
                title: text.get(UiTextKey::SettingsEditLayoutToml),
                description: text.get(UiTextKey::SettingsEditLayoutTomlDescription),
                scope: SettingsScope::Project,
                apply: SettingApply::Immediate,
                project_override: false,
            },
            SettingsRowMeta {
                key: "project_layout.save",
                title: text.get(UiTextKey::SettingsSaveCurrentLayout),
                description: text.get(UiTextKey::SettingsSaveCurrentLayoutDescription),
                scope: SettingsScope::Project,
                apply: SettingApply::Immediate,
                project_override: false,
            },
            SettingsRowMeta {
                key: "project_layout.export",
                title: text.get(UiTextKey::SettingsExportProjectLayout),
                description: text.get(UiTextKey::SettingsExportProjectLayoutDescription),
                scope: SettingsScope::Project,
                apply: SettingApply::Immediate,
                project_override: false,
            },
        ],
        SettingsGroupId::DefaultLayout => vec![
            row(
                "default_layout.path",
                UiTextKey::SettingsDefaultLayoutPath,
                UiTextKey::SettingsDefaultLayoutPathDescription,
            ),
            row(
                "default_layout.edit",
                UiTextKey::SettingsEditDefaultLayout,
                UiTextKey::SettingsEditDefaultLayoutDescription,
            ),
            row(
                "default_layout.reload",
                UiTextKey::SettingsReloadDefaultLayout,
                UiTextKey::SettingsReloadDefaultLayoutDescription,
            ),
            row(
                "default_layout.reset",
                UiTextKey::SettingsResetDefaultLayout,
                UiTextKey::SettingsResetDefaultLayoutDescription,
            ),
        ],
        SettingsGroupId::Keybindings => vec![
            row(
                "keybindings.edit",
                UiTextKey::SettingsEditKeybindingsToml,
                UiTextKey::SettingsEditKeybindingsTomlDescription,
            ),
            row(
                "keybindings.vim_quick_start",
                UiTextKey::SettingsVimQuickStart,
                UiTextKey::SettingsVimQuickStartDescription,
            ),
            row(
                "keybindings.vim_leader",
                UiTextKey::SettingsVimLeader,
                UiTextKey::SettingsVimLeaderDescription,
            ),
            row(
                "keybindings.diagnostics",
                UiTextKey::SettingsKeybindingDiagnostics,
                UiTextKey::SettingsKeybindingDiagnosticsDescription,
            ),
        ],
    }
}

pub fn settings_rows_for_scope(
    group: SettingsGroupId,
    text: &UiText,
    scope: SettingsScope,
) -> Vec<SettingsRowMeta> {
    settings_rows_for_group(group, text)
        .into_iter()
        .filter(|row| row.is_visible_for(scope))
        .collect()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SettingsGroupItem {
    pub id: SettingsGroupId,
    pub title: &'static str,
    pub description: &'static str,
    pub selected: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SettingsPageState {
    pub is_open: bool,
    pub selected_group: SettingsGroupId,
    pub search_query: String,
}

impl Default for SettingsPageState {
    fn default() -> Self {
        Self {
            is_open: false,
            selected_group: SettingsGroupId::General,
            search_query: String::new(),
        }
    }
}

impl SettingsPageState {
    pub fn visible_groups(&self, text: &UiText) -> Vec<SettingsGroupItem> {
        self.visible_groups_for_scope(text, None)
    }

    pub fn visible_groups_for_settings_scope(
        &self,
        text: &UiText,
        scope: SettingsScope,
    ) -> Vec<SettingsGroupItem> {
        self.visible_groups_for_scope(text, Some(scope))
    }

    fn visible_groups_for_scope(
        &self,
        text: &UiText,
        scope: Option<SettingsScope>,
    ) -> Vec<SettingsGroupItem> {
        let query = self.search_query.trim().to_lowercase();
        SettingsGroupId::ALL
            .iter()
            .copied()
            .filter(|group| {
                let rows = match scope {
                    Some(scope) => settings_rows_for_scope(*group, text, scope),
                    None => settings_rows_for_group(*group, text),
                };
                !rows.is_empty()
                    && (query.is_empty()
                        || group.title(text).to_lowercase().contains(&query)
                        || group.description(text).to_lowercase().contains(&query)
                        || rows.iter().any(|row| {
                            row.title.to_lowercase().contains(&query)
                                || row.description.to_lowercase().contains(&query)
                        }))
            })
            .map(|group| SettingsGroupItem {
                id: group,
                title: group.title(text),
                description: group.description(text),
                selected: group == self.selected_group,
            })
            .collect()
    }
}
