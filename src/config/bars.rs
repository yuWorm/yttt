use std::{
    collections::{BTreeMap, HashSet},
    fs, mem,
    path::PathBuf,
};

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
pub struct ShellBarsSettings {
    pub window: WindowBarSettings,
    pub status: StatusBarSettings,
}

impl Default for ShellBarsSettings {
    fn default() -> Self {
        Self {
            window: WindowBarSettings::default(),
            status: StatusBarSettings::default(),
        }
    }
}

impl ShellBarsSettings {
    pub fn contains(&self, module: &ShellBarModule) -> bool {
        self.window.layout.contains(module)
            || (self.status.enabled && self.status.layout.contains(module))
    }

    pub fn validate(&mut self) -> Vec<BarSettingsIssue> {
        let mut issues = Vec::new();
        validate_layout(&mut self.window.layout, "window", &mut issues);
        validate_layout(&mut self.status.layout, "status", &mut issues);
        issues
    }
}

pub fn load_or_create_bars(paths: &AppConfigPaths) -> Result<LoadedBars, BarsLoadError> {
    let path = ensure_bars_file(paths)?;
    let source = fs::read_to_string(&path).map_err(|source| BarsLoadError::Read {
        path: path.clone(),
        source,
    })?;
    let mut warnings = Vec::new();
    let mut settings = match toml::from_str::<ShellBarsSettings>(&source) {
        Ok(settings) => settings,
        Err(error) => {
            warnings.push(BarsLoadWarning::InvalidToml {
                path,
                message: error.to_string(),
            });
            return Ok(LoadedBars {
                settings: ShellBarsSettings::default(),
                warnings,
            });
        }
    };
    warnings.extend(
        settings
            .validate()
            .into_iter()
            .map(|issue| BarsLoadWarning::InvalidValue {
                field: issue.field,
                value: issue.value,
            }),
    );
    Ok(LoadedBars { settings, warnings })
}

pub fn save_bars(
    paths: &AppConfigPaths,
    settings: &ShellBarsSettings,
) -> Result<PathBuf, BarsSaveError> {
    let path = paths.bars_file();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| BarsSaveError::CreateConfigDirectory {
            path: parent.to_path_buf(),
            source,
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

fn ensure_bars_file(paths: &AppConfigPaths) -> Result<PathBuf, BarsLoadError> {
    let path = paths.bars_file();
    if path.exists() {
        return Ok(path);
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| BarsLoadError::CreateConfigDirectory {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    let source = toml::to_string_pretty(&ShellBarsSettings::default()).map_err(|source| {
        BarsLoadError::SerializeDefaults {
            path: path.clone(),
            source,
        }
    })?;
    atomic_write(&path, source.as_bytes()).map_err(|source| BarsLoadError::WriteDefaults {
        path: path.clone(),
        source,
    })?;
    Ok(path)
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
                left: vec![ShellBarModule::ProjectName, ShellBarModule::ProjectPath],
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
                    ShellBarModule::GitBranch,
                    ShellBarModule::GitChanges,
                    ShellBarModule::CommandPalette,
                    ShellBarModule::Settings,
                ],
                modules: BTreeMap::new(),
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
            },
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BarLayoutSettings {
    pub left: Vec<ShellBarModule>,
    pub center: Vec<ShellBarModule>,
    pub right: Vec<ShellBarModule>,
    pub modules: BTreeMap<String, BarModuleSettings>,
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
    let mut seen = HashSet::new();
    validate_section(&mut layout.left, host, "left", &mut seen, issues);
    validate_section(&mut layout.center, host, "center", &mut seen, issues);
    validate_section(&mut layout.right, host, "right", &mut seen, issues);

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
        let canonical_name = module.as_str().to_string();
        if normalized
            .insert(canonical_name.clone(), settings)
            .is_some()
        {
            issues.push(BarSettingsIssue {
                field: module_field,
                value: canonical_name,
            });
        }
    }
    layout.modules = normalized;
}

fn validate_section(
    modules: &mut Vec<ShellBarModule>,
    host: &'static str,
    section: &'static str,
    seen: &mut HashSet<String>,
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
        if !module.is_known() || !seen.insert(module.as_str().to_string()) {
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
