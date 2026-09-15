use std::{collections::BTreeMap, mem, path::PathBuf};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::{atomic_write, paths::AppConfigPaths};

pub const MIN_BAR_MODULE_WIDTH: f32 = 24.0;
pub const MAX_BAR_MODULE_WIDTH: f32 = 640.0;

#[derive(Clone, Debug, PartialEq)]
pub struct LoadedBars {
    pub settings: ShellBarsSettings,
    pub warnings: Vec<BarsLoadWarning>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BarsLoadWarning {
    InvalidToml { path: PathBuf, message: String },
    InvalidValue { field: &'static str, value: String },
}

#[derive(Debug, thiserror::Error)]
pub enum BarsLoadError {
    #[error("Device preferences are not bound; cannot load bars")]
    DevicePreferencesUnbound,
    #[error("failed to create bars config directory {path}: {source}")]
    CreateConfigDirectory {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to read bars file at {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to serialize default bars at {path}: {source}")]
    SerializeDefaults {
        path: PathBuf,
        source: toml::ser::Error,
    },
    #[error("failed to write default bars at {path}: {source}")]
    WriteDefaults {
        path: PathBuf,
        source: std::io::Error,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum BarsSaveError {
    #[error("Device preferences are not bound; cannot save bars")]
    DevicePreferencesUnbound,
    #[error("failed to create bars config directory {path}: {source}")]
    CreateConfigDirectory {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to serialize bars at {path}: {source}")]
    Serialize {
        path: PathBuf,
        source: toml::ser::Error,
    },
    #[error("failed to write bars at {path}: {source}")]
    Write {
        path: PathBuf,
        source: std::io::Error,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
#[derive(Default)]
pub struct ShellBarsSettings {
    pub window: WindowBarSettings,
    pub status: StatusBarSettings,
}

impl ShellBarsSettings {
    pub fn contains(&self, module: &ShellBarModule) -> bool {
        self.window.layout.contains(module)
            || (self.status.enabled && self.status.layout.contains(module))
    }

    pub fn validate(&mut self) -> Vec<BarSettingsIssue> {
        self.window.layout.migrate_legacy_window_identity();
        self.status.layout.clear_legacy_region_markers();
        let mut issues = Vec::new();
        validate_layout(&mut self.window.layout, "window", &mut issues);
        validate_layout(&mut self.status.layout, "status", &mut issues);
        issues
    }
}

/// Load bars without creating a preferences file. Missing bars retain builtin defaults.
pub fn load_bars(paths: &AppConfigPaths) -> Result<LoadedBars, BarsLoadError> {
    let paths = device_bar_paths(paths).ok_or(BarsLoadError::DevicePreferencesUnbound)?;
    let path = paths.bars_file();
    let source = match crate::config::storage::read_to_string(&path) {
        Ok(source) => source,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
            return Ok(LoadedBars {
                settings: ShellBarsSettings::default(),
                warnings: Vec::new(),
            });
        }
        Err(source) => return Err(BarsLoadError::Read { path, source }),
    };
    let mut settings = match toml::from_str::<ShellBarsSettings>(&source) {
        Ok(settings) => settings,
        Err(error) => {
            return Ok(LoadedBars {
                settings: ShellBarsSettings::default(),
                warnings: vec![BarsLoadWarning::InvalidToml {
                    path,
                    message: error.to_string(),
                }],
            });
        }
    };
    let warnings = settings
        .validate()
        .into_iter()
        .map(|issue| BarsLoadWarning::InvalidValue {
            field: issue.field,
            value: issue.value,
        })
        .collect();
    Ok(LoadedBars { settings, warnings })
}

pub fn save_bars(
    paths: &AppConfigPaths,
    settings: &ShellBarsSettings,
) -> Result<PathBuf, BarsSaveError> {
    let paths = device_bar_paths(paths).ok_or(BarsSaveError::DevicePreferencesUnbound)?;
    let path = paths.bars_file();
    if let Some(parent) = path.parent() {
        crate::config::storage::create_dir_all(parent).map_err(|source| {
            BarsSaveError::CreateConfigDirectory {
                path: parent.to_path_buf(),
                source,
            }
        })?;
    }
    let source = toml::to_string_pretty(settings).map_err(|source| BarsSaveError::Serialize {
        path: path.clone(),
        source,
    })?;
    atomic_write(&path, source.as_bytes()).map_err(|source| BarsSaveError::Write {
        path: path.clone(),
        source,
    })?;
    Ok(path)
}

fn device_bar_paths(paths: &AppConfigPaths) -> Option<AppConfigPaths> {
    paths
        .is_test_fixture()
        .then(|| paths.clone())
        .or_else(crate::config::scope::device_preferences_config_paths)
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WindowBarSettings {
    #[serde(flatten)]
    pub layout: BarLayoutSettings,
}

impl Default for WindowBarSettings {
    fn default() -> Self {
        Self {
            layout: BarLayoutSettings {
                left: window_identity_template(),
                center: Vec::new(),
                right: vec![
                    ShellBarModule::ProjectsCount,
                    ShellBarModule::TerminalsCount,
                    ShellBarModule::TabsCount,
                    ShellBarModule::EditorsCount,
                    ShellBarModule::AppCpu,
                    ShellBarModule::AppMemory,
                    ShellBarModule::SystemCpu,
                    ShellBarModule::SystemMemory,
                    ShellBarModule::CommandPalette,
                    ShellBarModule::Settings,
                ],
                modules: BTreeMap::new(),
                legacy_left: false,
                legacy_center: false,
                legacy_right: false,
            },
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StatusBarSettings {
    pub enabled: bool,
    #[serde(flatten)]
    pub layout: BarLayoutSettings,
}

impl Default for StatusBarSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            layout: BarLayoutSettings {
                left: vec![
                    ShellBarModule::VimMode,
                    ShellBarModule::Surface,
                    ShellBarModule::VimDetail,
                    ShellBarModule::ActiveItem,
                ],
                center: vec![ShellBarModule::VimKeys],
                right: vec![
                    ShellBarModule::EditorLanguage,
                    ShellBarModule::EditorPosition,
                    ShellBarModule::EditorDirty,
                    ShellBarModule::EditorDiagnostics,
                    ShellBarModule::GitBranch,
                    ShellBarModule::GitChanges,
                    ShellBarModule::AgentState,
                    ShellBarModule::Ssh,
                    ShellBarModule::Update,
                ],
                modules: BTreeMap::new(),
                legacy_left: false,
                legacy_center: false,
                legacy_right: false,
            },
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(default)]
pub struct BarLayoutSettings {
    #[serde(serialize_with = "serialize_bar_template")]
    pub left: Vec<ShellBarModule>,
    #[serde(serialize_with = "serialize_bar_template")]
    pub center: Vec<ShellBarModule>,
    #[serde(serialize_with = "serialize_bar_template")]
    pub right: Vec<ShellBarModule>,
    pub modules: BTreeMap<String, BarModuleSettings>,
    #[serde(skip)]
    legacy_left: bool,
    #[serde(skip)]
    legacy_center: bool,
    #[serde(skip)]
    legacy_right: bool,
}

impl<'de> Deserialize<'de> for BarLayoutSettings {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Default, Deserialize)]
        #[serde(default)]
        struct RawBarLayoutSettings {
            left: Option<ParsedBarTemplate>,
            center: Option<ParsedBarTemplate>,
            right: Option<ParsedBarTemplate>,
            modules: BTreeMap<String, BarModuleSettings>,
        }

        let raw = RawBarLayoutSettings::deserialize(deserializer)?;
        let left = raw.left.unwrap_or_else(ParsedBarTemplate::legacy_empty);
        let center = raw.center.unwrap_or_else(ParsedBarTemplate::legacy_empty);
        let right = raw.right.unwrap_or_else(ParsedBarTemplate::legacy_empty);
        Ok(Self {
            left: left.modules,
            center: center.modules,
            right: right.modules,
            modules: raw.modules,
            legacy_left: left.legacy_array,
            legacy_center: center.legacy_array,
            legacy_right: right.legacy_array,
        })
    }
}

impl BarLayoutSettings {
    pub fn contains(&self, module: &ShellBarModule) -> bool {
        self.left
            .iter()
            .chain(&self.center)
            .chain(&self.right)
            .any(|candidate| candidate == module)
    }

    pub fn module_settings(&self, module: &ShellBarModule) -> BarModuleSettings {
        self.modules
            .get(module.as_str())
            .copied()
            .unwrap_or_default()
    }

    fn migrate_legacy_window_identity(&mut self) {
        if self.legacy_left {
            self.left.retain(|module| !is_window_identity(module));
            let mut migrated = window_identity_template();
            migrated.append(&mut self.left);
            self.left = migrated;
        }
        if self.legacy_center {
            self.center.retain(|module| !is_window_identity(module));
        }
        if self.legacy_right {
            self.right.retain(|module| !is_window_identity(module));
        }
        self.legacy_left = false;
        self.legacy_center = false;
        self.legacy_right = false;
    }

    fn clear_legacy_region_markers(&mut self) {
        self.legacy_left = false;
        self.legacy_center = false;
        self.legacy_right = false;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BarModuleSettings {
    pub max_width: Option<f32>,
    pub hide_when_empty: bool,
}

impl Default for BarModuleSettings {
    fn default() -> Self {
        Self {
            max_width: None,
            hide_when_empty: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShellBarModule {
    ProjectName,
    ProjectPath,
    ActiveItem,
    Surface,
    VimMode,
    VimDetail,
    VimKeys,
    EditorLanguage,
    EditorPosition,
    EditorDirty,
    EditorDiagnostics,
    TerminalTitle,
    TerminalState,
    GitBranch,
    GitChanges,
    AgentState,
    Ssh,
    Update,
    ProjectsCount,
    TerminalsCount,
    TabsCount,
    EditorsCount,
    AppCpu,
    AppMemory,
    SystemCpu,
    SystemMemory,
    CommandPalette,
    Settings,
    Space(u16),
    Text(String),
    Icon(String),
    Separator,
    Unknown(String),
}

impl ShellBarModule {
    pub fn from_name(name: impl Into<String>) -> Self {
        let name = name.into();
        let normalized = name.trim().to_ascii_lowercase().replace('_', "-");
        match normalized.as_str() {
            "project-name" => Self::ProjectName,
            "project-path" => Self::ProjectPath,
            "active-item" => Self::ActiveItem,
            "surface" => Self::Surface,
            "vim-mode" => Self::VimMode,
            "vim-detail" => Self::VimDetail,
            "vim-keys" => Self::VimKeys,
            "editor-language" => Self::EditorLanguage,
            "editor-position" => Self::EditorPosition,
            "editor-dirty" => Self::EditorDirty,
            "editor-diagnostics" => Self::EditorDiagnostics,
            "terminal-title" => Self::TerminalTitle,
            "terminal-state" => Self::TerminalState,
            "git-branch" => Self::GitBranch,
            "git-changes" => Self::GitChanges,
            "agent-state" => Self::AgentState,
            "ssh" => Self::Ssh,
            "update" => Self::Update,
            "projects-count" => Self::ProjectsCount,
            "terminals-count" => Self::TerminalsCount,
            "tabs-count" => Self::TabsCount,
            "editors-count" => Self::EditorsCount,
            "app-cpu" => Self::AppCpu,
            "app-memory" => Self::AppMemory,
            "system-cpu" => Self::SystemCpu,
            "system-memory" => Self::SystemMemory,
            "command-palette" => Self::CommandPalette,
            "settings" => Self::Settings,
            _ => Self::Unknown(name),
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            Self::ProjectName => "project-name",
            Self::ProjectPath => "project-path",
            Self::ActiveItem => "active-item",
            Self::Surface => "surface",
            Self::VimMode => "vim-mode",
            Self::VimDetail => "vim-detail",
            Self::VimKeys => "vim-keys",
            Self::EditorLanguage => "editor-language",
            Self::EditorPosition => "editor-position",
            Self::EditorDirty => "editor-dirty",
            Self::EditorDiagnostics => "editor-diagnostics",
            Self::TerminalTitle => "terminal-title",
            Self::TerminalState => "terminal-state",
            Self::GitBranch => "git-branch",
            Self::GitChanges => "git-changes",
            Self::AgentState => "agent-state",
            Self::Ssh => "ssh",
            Self::Update => "update",
            Self::ProjectsCount => "projects-count",
            Self::TerminalsCount => "terminals-count",
            Self::TabsCount => "tabs-count",
            Self::EditorsCount => "editors-count",
            Self::AppCpu => "app-cpu",
            Self::AppMemory => "app-memory",
            Self::SystemCpu => "system-cpu",
            Self::SystemMemory => "system-memory",
            Self::CommandPalette => "command-palette",
            Self::Settings => "settings",
            Self::Space(_) => "space",
            Self::Text(_) => "text",
            Self::Icon(_) => "icon",
            Self::Separator => "separator",
            Self::Unknown(name) => name,
        }
    }

    pub fn is_known(&self) -> bool {
        !matches!(self, Self::Unknown(_))
    }
}

impl Serialize for ShellBarModule {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for ShellBarModule {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        String::deserialize(deserializer).map(Self::from_name)
    }
}

const MAX_BAR_TEMPLATE_BYTES: usize = 16 * 1024;
const MAX_BAR_TEMPLATE_MODULES: usize = 256;
const MAX_BAR_TEMPLATE_TEXT_BYTES: usize = 4 * 1024;
const MAX_BAR_SPACE_COUNT: u16 = 256;

const BAR_ICON_NAMES: &[(&str, &str)] = &[
    ("settings", "icons/settings.svg"),
    ("info", "icons/info.svg"),
    ("cpu", "icons/cpu.svg"),
    ("memory-stick", "icons/memory-stick.svg"),
    ("search", "icons/search.svg"),
    ("palette", "icons/palette.svg"),
    ("folder", "icons/folder.svg"),
    ("folder-open", "icons/folder-open.svg"),
    ("file", "icons/file.svg"),
    ("square-terminal", "icons/square-terminal.svg"),
    ("github", "icons/github.svg"),
    ("network", "icons/network.svg"),
    ("globe", "icons/globe.svg"),
    ("user", "icons/user.svg"),
    ("bot", "icons/bot.svg"),
    ("bell", "icons/bell.svg"),
    ("calendar", "icons/calendar.svg"),
    ("chart-pie", "icons/chart-pie.svg"),
    ("hard-drive", "icons/hard-drive.svg"),
    ("battery", "icons/battery.svg"),
    ("triangle-alert", "icons/triangle-alert.svg"),
    ("circle-check", "icons/circle-check.svg"),
    ("circle-x", "icons/circle-x.svg"),
    ("play", "icons/play.svg"),
    ("pause", "icons/pause.svg"),
];

/// Returns the bundled asset path for an allowlisted bar icon.
pub fn bar_icon_path(icon: &str) -> Option<&'static str> {
    let icon = icon.trim();
    BAR_ICON_NAMES
        .iter()
        .find(|(name, _)| icon.eq_ignore_ascii_case(name))
        .map(|(_, path)| *path)
}

fn canonical_bar_icon_name(icon: &str) -> Option<&'static str> {
    let icon = icon.trim();
    BAR_ICON_NAMES
        .iter()
        .find(|(name, _)| icon.eq_ignore_ascii_case(name))
        .map(|(name, _)| *name)
}

/// Parses a bracket-delimited bar template into modules.
pub fn parse_bar_template(template: &str) -> Result<Vec<ShellBarModule>, String> {
    if template.len() > MAX_BAR_TEMPLATE_BYTES {
        return Err(template_error(
            MAX_BAR_TEMPLATE_BYTES,
            "template exceeds the 16 KiB limit",
        ));
    }

    let mut modules = Vec::new();
    let mut characters = template.char_indices();
    while let Some((position, character)) = characters.next() {
        if character.is_whitespace() {
            continue;
        }
        if character != '[' {
            return Err(template_error(position, "expected '[' or whitespace"));
        }

        let mut token = String::new();
        let mut closed = false;
        while let Some((token_position, character)) = characters.next() {
            match character {
                ']' => {
                    closed = true;
                    break;
                }
                '[' => {
                    return Err(template_error(
                        token_position,
                        "unescaped '[' inside a token",
                    ));
                }
                '\\' => match characters.next() {
                    Some((_, '[')) => token.push('['),
                    Some((_, ']')) => token.push(']'),
                    Some((_, '\\')) => token.push('\\'),
                    Some((escape_position, _)) => {
                        return Err(template_error(
                            escape_position,
                            "only '\\[', '\\]' and '\\\\' escapes are allowed",
                        ));
                    }
                    None => {
                        return Err(template_error(token_position, "unfinished escape"));
                    }
                },
                _ => token.push(character),
            }
        }
        if !closed {
            return Err(template_error(position, "unclosed '['"));
        }
        if modules.len() == MAX_BAR_TEMPLATE_MODULES {
            return Err(template_error(
                position,
                "template exceeds the 256-module limit",
            ));
        }
        modules.push(parse_bar_template_token(&token, position)?);
    }
    Ok(modules)
}

/// Formats modules as a canonical bracket-delimited bar template.
pub fn format_bar_template(modules: &[ShellBarModule]) -> String {
    let mut template = String::new();
    for (index, module) in modules.iter().enumerate() {
        if index > 0 {
            template.push(' ');
        }
        match module {
            ShellBarModule::Space(1) => template.push_str("[Space]"),
            ShellBarModule::Space(count) => {
                template.push_str("[Space*");
                template.push_str(&count.to_string());
                template.push(']');
            }
            ShellBarModule::Text(text) => {
                template.push_str("[text:");
                escape_bar_template_text(text, &mut template);
                template.push(']');
            }
            ShellBarModule::Icon(icon) => {
                template.push_str("[icon:");
                template.push_str(icon);
                template.push(']');
            }
            ShellBarModule::Separator => template.push_str("[|]"),
            module => {
                template.push('[');
                template.push_str(module.as_str());
                template.push(']');
            }
        }
    }
    template
}

fn serialize_bar_template<S>(modules: &[ShellBarModule], serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.serialize_str(&format_bar_template(modules))
}

#[derive(Debug)]
struct ParsedBarTemplate {
    modules: Vec<ShellBarModule>,
    legacy_array: bool,
}

impl ParsedBarTemplate {
    fn legacy_empty() -> Self {
        Self {
            modules: Vec::new(),
            legacy_array: true,
        }
    }
}

impl<'de> Deserialize<'de> for ParsedBarTemplate {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Representation {
            Template(String),
            Legacy(Vec<ShellBarModule>),
        }

        match Representation::deserialize(deserializer)? {
            Representation::Template(template) => parse_bar_template(&template)
                .map(|modules| Self {
                    modules,
                    legacy_array: false,
                })
                .map_err(serde::de::Error::custom),
            Representation::Legacy(modules) => Ok(Self {
                modules,
                legacy_array: true,
            }),
        }
    }
}

fn parse_bar_template_token(token: &str, position: usize) -> Result<ShellBarModule, String> {
    let trimmed = token.trim();
    if trimmed.is_empty() {
        return Err(template_error(position, "empty token"));
    }
    if trimmed == "|" {
        return Ok(ShellBarModule::Separator);
    }

    if let Some((keyword, value)) = token.split_once(':') {
        let keyword = keyword.trim();
        if keyword.eq_ignore_ascii_case("text") {
            if value.len() > MAX_BAR_TEMPLATE_TEXT_BYTES {
                return Err(template_error(position, "text exceeds the 4 KiB limit"));
            }
            return Ok(ShellBarModule::Text(value.to_string()));
        }
        if keyword.eq_ignore_ascii_case("icon") {
            let icon = canonical_bar_icon_name(value).ok_or_else(|| {
                template_error(position, format!("unknown icon {:?}", value.trim()))
            })?;
            return Ok(ShellBarModule::Icon(icon.to_string()));
        }
        return Err(template_error(
            position,
            format!("unknown token keyword {:?}", keyword),
        ));
    }

    if trimmed.eq_ignore_ascii_case("text") || trimmed.eq_ignore_ascii_case("icon") {
        return Err(template_error(position, "token requires ':'"));
    }
    if trimmed.eq_ignore_ascii_case("space") {
        return Ok(ShellBarModule::Space(1));
    }
    if let Some((keyword, count)) = trimmed.split_once('*')
        && keyword.trim().eq_ignore_ascii_case("space")
    {
        let count = count.trim().parse::<u16>().map_err(|_| {
            template_error(position, "space count must be an integer between 1 and 256")
        })?;
        if !(1..=MAX_BAR_SPACE_COUNT).contains(&count) {
            return Err(template_error(
                position,
                "space count must be between 1 and 256",
            ));
        }
        return Ok(ShellBarModule::Space(count));
    }

    let module = ShellBarModule::from_name(trimmed);
    if module.is_known() {
        Ok(module)
    } else {
        Err(template_error(
            position,
            format!("unknown token {:?}", trimmed),
        ))
    }
}

fn escape_bar_template_text(text: &str, template: &mut String) {
    for character in text.chars() {
        match character {
            '[' => template.push_str("\\["),
            ']' => template.push_str("\\]"),
            '\\' => template.push_str("\\\\"),
            _ => template.push(character),
        }
    }
}

fn template_error(position: usize, message: impl AsRef<str>) -> String {
    format!("bar template at byte {position}: {}", message.as_ref())
}

fn window_identity_template() -> Vec<ShellBarModule> {
    vec![
        ShellBarModule::ProjectName,
        ShellBarModule::Separator,
        ShellBarModule::ProjectPath,
        ShellBarModule::Separator,
        ShellBarModule::GitBranch,
        ShellBarModule::Separator,
        ShellBarModule::GitChanges,
    ]
}

fn is_window_identity(module: &ShellBarModule) -> bool {
    matches!(
        module,
        ShellBarModule::ProjectName
            | ShellBarModule::ProjectPath
            | ShellBarModule::GitBranch
            | ShellBarModule::GitChanges
    )
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BarSettingsIssue {
    pub field: &'static str,
    pub value: String,
}

fn validate_layout(
    layout: &mut BarLayoutSettings,
    host: &'static str,
    issues: &mut Vec<BarSettingsIssue>,
) {
    validate_section(&mut layout.left, host, "left", issues);
    validate_section(&mut layout.center, host, "center", issues);
    validate_section(&mut layout.right, host, "right", issues);

    let module_field = if host == "window" {
        "window.modules"
    } else {
        "status.modules"
    };
    let width_field = if host == "window" {
        "window.modules.max_width"
    } else {
        "status.modules.max_width"
    };
    let mut normalized = BTreeMap::new();
    for (name, mut settings) in mem::take(&mut layout.modules) {
        let module = ShellBarModule::from_name(name.clone());
        if !module.is_known() {
            issues.push(BarSettingsIssue {
                field: module_field,
                value: name,
            });
            continue;
        }
        if settings.max_width.is_some_and(|width| {
            !width.is_finite() || !(MIN_BAR_MODULE_WIDTH..=MAX_BAR_MODULE_WIDTH).contains(&width)
        }) {
            issues.push(BarSettingsIssue {
                field: width_field,
                value: module.as_str().to_string(),
            });
            settings.max_width = None;
        }
        normalized.insert(module.as_str().to_string(), settings);
    }
    layout.modules = normalized;
}

fn validate_section(
    modules: &mut Vec<ShellBarModule>,
    host: &'static str,
    section: &'static str,
    issues: &mut Vec<BarSettingsIssue>,
) {
    let field = match (host, section) {
        ("window", "left") => "window.left",
        ("window", "center") => "window.center",
        ("window", "right") => "window.right",
        ("status", "left") => "status.left",
        ("status", "center") => "status.center",
        ("status", "right") => "status.right",
        _ => "bars",
    };
    modules.retain(|module| {
        if !module.is_known() {
            issues.push(BarSettingsIssue {
                field,
                value: module.as_str().to_string(),
            });
            false
        } else {
            true
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_device_bars_use_builtins_without_creating_a_file() {
        let root = tempfile::tempdir().unwrap();
        let paths = AppConfigPaths::from_config_dir(root.path().join("device"));

        let loaded = load_bars(&paths).unwrap();

        assert_eq!(loaded.settings, ShellBarsSettings::default());
        assert!(loaded.warnings.is_empty());
        assert!(!paths.bars_file().exists());
    }

    #[test]
    fn malformed_device_bars_are_reported_without_rewriting_the_file() {
        let root = tempfile::tempdir().unwrap();
        let paths = AppConfigPaths::from_config_dir(root.path().join("device"));
        std::fs::create_dir_all(paths.config_dir()).unwrap();
        std::fs::write(paths.bars_file(), "[window\n").unwrap();

        let loaded = load_bars(&paths).unwrap();

        assert_eq!(loaded.settings, ShellBarsSettings::default());
        assert!(matches!(
            loaded.warnings.as_slice(),
            [BarsLoadWarning::InvalidToml { .. }]
        ));
        assert_eq!(
            std::fs::read_to_string(paths.bars_file()).unwrap(),
            "[window\n"
        );
    }
}
