use gpui::{Pixels, px};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SettingsPanelStyle {
    pub width: Pixels,
    pub max_width: Pixels,
    pub max_height: Pixels,
    pub sidebar_width: Pixels,
    pub row_min_height: Pixels,
    pub border_width: Pixels,
}

pub fn settings_panel_style() -> SettingsPanelStyle {
    SettingsPanelStyle {
        width: px(980.0),
        max_width: px(1180.0),
        max_height: px(680.0),
        sidebar_width: px(260.0),
        row_min_height: px(68.0),
        border_width: px(1.0),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsGroupId {
    General,
    Appearance,
    Terminal,
    ProjectLayout,
    Keybindings,
}

impl SettingsGroupId {
    pub const ALL: &'static [Self] = &[
        Self::General,
        Self::Appearance,
        Self::Terminal,
        Self::ProjectLayout,
        Self::Keybindings,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::General => "general",
            Self::Appearance => "appearance",
            Self::Terminal => "terminal",
            Self::ProjectLayout => "project-layout",
            Self::Keybindings => "keybindings",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::General => "General",
            Self::Appearance => "Appearance",
            Self::Terminal => "Terminal",
            Self::ProjectLayout => "Project Layout",
            Self::Keybindings => "Keybindings",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::General => "Application behavior and notifications",
            Self::Appearance => "UI and terminal themes",
            Self::Terminal => "Shell, font, and terminal runtime defaults",
            Self::ProjectLayout => "Project layout files and TOML editing",
            Self::Keybindings => "Keyboard shortcuts and conflict diagnostics",
        }
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

pub fn settings_rows_for_group(group: SettingsGroupId) -> Vec<SettingsRowMeta> {
    match group {
        SettingsGroupId::General => vec![SettingsRowMeta {
            title: "System notifications",
            description: "Notify when agent terminal tasks complete or fail.",
        }],
        SettingsGroupId::Appearance => vec![
            SettingsRowMeta {
                title: "UI theme",
                description: "Theme used for YTTT chrome, panels, and controls.",
            },
            SettingsRowMeta {
                title: "Terminal theme",
                description: "Optional terminal colors override.",
            },
            SettingsRowMeta {
                title: "Edit settings TOML",
                description: "Open the app settings file for advanced edits.",
            },
            SettingsRowMeta {
                title: "Themes directory",
                description: "Open the folder containing user theme TOML files.",
            },
        ],
        SettingsGroupId::Terminal => vec![
            SettingsRowMeta {
                title: "Default shell",
                description: "Shell command used when creating new terminal tabs.",
            },
            SettingsRowMeta {
                title: "Font family",
                description: "Terminal font family.",
            },
            SettingsRowMeta {
                title: "Font size",
                description: "Terminal font size in pixels.",
            },
            SettingsRowMeta {
                title: "Line height",
                description: "Terminal line height multiplier.",
            },
            SettingsRowMeta {
                title: "Padding",
                description: "Terminal pane inner padding.",
            },
            SettingsRowMeta {
                title: "Scrollback",
                description: "Number of terminal lines kept in memory.",
            },
        ],
        SettingsGroupId::ProjectLayout => vec![
            SettingsRowMeta {
                title: "Layout source",
                description: "Current project layout source.",
            },
            SettingsRowMeta {
                title: "Save current layout",
                description: "Save current layout as an app-local override.",
            },
            SettingsRowMeta {
                title: "Export project layout",
                description: "Write current layout into the project config.",
            },
            SettingsRowMeta {
                title: "Edit layout TOML",
                description: "Edit the selected project layout file.",
            },
        ],
        SettingsGroupId::Keybindings => vec![
            SettingsRowMeta {
                title: "Edit keybindings TOML",
                description: "Open the user keybindings file.",
            },
            SettingsRowMeta {
                title: "Keybinding diagnostics",
                description: "Show invalid commands and shortcut conflicts.",
            },
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
    pub fn visible_groups(&self) -> Vec<SettingsGroupItem> {
        let query = self.search_query.trim().to_ascii_lowercase();
        SettingsGroupId::ALL
            .iter()
            .copied()
            .filter(|group| {
                query.is_empty()
                    || group.title().to_ascii_lowercase().contains(&query)
                    || group.description().to_ascii_lowercase().contains(&query)
            })
            .map(|group| SettingsGroupItem {
                id: group,
                title: group.title(),
                description: group.description(),
                selected: group == self.selected_group,
            })
            .collect()
    }
}
