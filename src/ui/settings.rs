use gpui::{Pixels, px};

use crate::ui::i18n::{UiText, UiTextKey};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SettingsPanelStyle {
    pub width: Pixels,
    pub height: Pixels,
    pub max_width: Pixels,
    pub max_height: Pixels,
    pub sidebar_width: Pixels,
    pub row_min_height: Pixels,
    pub control_width: Pixels,
    pub compact_control_width: Pixels,
    pub control_height: Pixels,
    pub search_height: Pixels,
    pub select_menu_width: Pixels,
    pub border_width: Pixels,
}

pub fn settings_panel_style() -> SettingsPanelStyle {
    SettingsPanelStyle {
        width: px(900.0),
        height: px(560.0),
        max_width: px(940.0),
        max_height: px(600.0),
        sidebar_width: px(240.0),
        row_min_height: px(72.0),
        control_width: px(220.0),
        compact_control_width: px(128.0),
        control_height: px(32.0),
        search_height: px(36.0),
        select_menu_width: px(280.0),
        border_width: px(1.0),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsGroupId {
    General,
    Appearance,
    Languages,
    Editor,
    Terminal,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SettingsRowMeta {
    pub title: &'static str,
    pub description: &'static str,
}

pub fn settings_rows_for_group(group: SettingsGroupId, text: &UiText) -> Vec<SettingsRowMeta> {
    let row = |title, description| SettingsRowMeta {
        title: text.get(title),
        description: text.get(description),
    };

    match group {
        SettingsGroupId::General => vec![
            row(
                UiTextKey::SettingsLanguage,
                UiTextKey::SettingsLanguageDescription,
            ),
            row(
                UiTextKey::SettingsSystemNotifications,
                UiTextKey::SettingsSystemNotificationsDescription,
            ),
        ],
        SettingsGroupId::Appearance => vec![
            row(
                UiTextKey::SettingsUiTheme,
                UiTextKey::SettingsUiThemeDescription,
            ),
            row(
                UiTextKey::SettingsTerminalTheme,
                UiTextKey::SettingsTerminalThemeDescription,
            ),
            row(
                UiTextKey::SettingsEditSettingsToml,
                UiTextKey::SettingsEditSettingsTomlDescription,
            ),
            row(
                UiTextKey::SettingsThemesDirectory,
                UiTextKey::SettingsThemesDirectoryDescription,
            ),
        ],
        SettingsGroupId::Languages => vec![
            row(
                UiTextKey::SettingsLanguageDetection,
                UiTextKey::SettingsLanguageDetectionDescription,
            ),
            row(
                UiTextKey::SettingsDefaultCodeLanguage,
                UiTextKey::SettingsDefaultCodeLanguageDescription,
            ),
            row(
                UiTextKey::SettingsSupportedLanguages,
                UiTextKey::SettingsSupportedLanguagesDescription,
            ),
            row(
                UiTextKey::SettingsLanguageServer,
                UiTextKey::SettingsLanguageServerDescription,
            ),
            row(
                UiTextKey::SettingsLanguageServerCommand,
                UiTextKey::SettingsLanguageServerCommandDescription,
            ),
        ],
        SettingsGroupId::Editor => vec![
            row(
                UiTextKey::SettingsEditorFontFamily,
                UiTextKey::SettingsEditorFontFamilyDescription,
            ),
            row(
                UiTextKey::SettingsEditorFontSize,
                UiTextKey::SettingsEditorFontSizeDescription,
            ),
            row(
                UiTextKey::SettingsEditorLineHeight,
                UiTextKey::SettingsEditorLineHeightDescription,
            ),
            row(
                UiTextKey::SettingsEditorTabSize,
                UiTextKey::SettingsEditorTabSizeDescription,
            ),
            row(
                UiTextKey::SettingsEditorSoftWrap,
                UiTextKey::SettingsEditorSoftWrapDescription,
            ),
            row(
                UiTextKey::SettingsEditorLineNumbers,
                UiTextKey::SettingsEditorLineNumbersDescription,
            ),
            row(
                UiTextKey::SettingsEditorAutosave,
                UiTextKey::SettingsEditorAutosaveDescription,
            ),
            row(
                UiTextKey::SettingsEditorAutosaveDelay,
                UiTextKey::SettingsEditorAutosaveDelayDescription,
            ),
            row(
                UiTextKey::SettingsProjectPanelDefaultOpen,
                UiTextKey::SettingsProjectPanelDefaultOpenDescription,
            ),
            row(
                UiTextKey::SettingsProjectPanelShowHidden,
                UiTextKey::SettingsProjectPanelShowHiddenDescription,
            ),
            row(
                UiTextKey::SettingsProjectPanelWidth,
                UiTextKey::SettingsProjectPanelWidthDescription,
            ),
            row(
                UiTextKey::SettingsProjectSidebarWidth,
                UiTextKey::SettingsProjectSidebarWidthDescription,
            ),
        ],
        SettingsGroupId::Terminal => vec![
            row(
                UiTextKey::SettingsDefaultShell,
                UiTextKey::SettingsDefaultShellDescription,
            ),
            row(
                UiTextKey::SettingsFontFamily,
                UiTextKey::SettingsFontFamilyDescription,
            ),
            row(
                UiTextKey::SettingsFontSize,
                UiTextKey::SettingsFontSizeDescription,
            ),
            row(
                UiTextKey::SettingsLineHeight,
                UiTextKey::SettingsLineHeightDescription,
            ),
            row(
                UiTextKey::SettingsPadding,
                UiTextKey::SettingsPaddingDescription,
            ),
            row(
                UiTextKey::SettingsScrollback,
                UiTextKey::SettingsScrollbackDescription,
            ),
            row(
                UiTextKey::SettingsScrollbar,
                UiTextKey::SettingsScrollbarDescription,
            ),
            row(
                UiTextKey::SettingsClosePaneOnExit,
                UiTextKey::SettingsClosePaneOnExitDescription,
            ),
        ],
        SettingsGroupId::DefaultLayout => vec![
            row(
                UiTextKey::SettingsDefaultLayoutPath,
                UiTextKey::SettingsDefaultLayoutPathDescription,
            ),
            row(
                UiTextKey::SettingsEditDefaultLayout,
                UiTextKey::SettingsEditDefaultLayoutDescription,
            ),
            row(
                UiTextKey::SettingsReloadDefaultLayout,
                UiTextKey::SettingsReloadDefaultLayoutDescription,
            ),
            row(
                UiTextKey::SettingsResetDefaultLayout,
                UiTextKey::SettingsResetDefaultLayoutDescription,
            ),
        ],
        SettingsGroupId::Keybindings => vec![
            row(
                UiTextKey::SettingsEditKeybindingsToml,
                UiTextKey::SettingsEditKeybindingsTomlDescription,
            ),
            row(
                UiTextKey::SettingsKeybindingDiagnostics,
                UiTextKey::SettingsKeybindingDiagnosticsDescription,
            ),
        ],
    }
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
        let query = self.search_query.trim().to_lowercase();
        SettingsGroupId::ALL
            .iter()
            .copied()
            .filter(|group| {
                query.is_empty()
                    || group.title(text).to_lowercase().contains(&query)
                    || group.description(text).to_lowercase().contains(&query)
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
