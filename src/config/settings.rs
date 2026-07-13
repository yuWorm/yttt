use std::{
    fs,
    path::{Path, PathBuf},
};

use crate::config::paths::AppConfigPaths;
use crate::ui::theme::DEFAULT_THEME_NAME;

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct AppSettings {
    pub general: GeneralSettings,
    pub theme: ThemeSettings,
    pub notifications: NotificationSettings,
    pub terminal: TerminalSettings,
    pub editor: EditorSettings,
    pub project_panel: ProjectPanelSettings,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            general: GeneralSettings::default(),
            theme: ThemeSettings::default(),
            notifications: NotificationSettings::default(),
            terminal: TerminalSettings::default(),
            editor: EditorSettings::default(),
            project_panel: ProjectPanelSettings::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LanguageSetting {
    #[serde(rename = "system")]
    System,
    #[serde(rename = "en")]
    English,
    #[serde(rename = "zh-CN")]
    Chinese,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct GeneralSettings {
    pub language: LanguageSetting,
    pub onboarding_completed: bool,
}

impl Default for GeneralSettings {
    fn default() -> Self {
        Self {
            language: LanguageSetting::System,
            onboarding_completed: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct ThemeSettings {
    pub name: String,
    pub terminal: Option<String>,
    pub icon_theme: Option<String>,
}

impl Default for ThemeSettings {
    fn default() -> Self {
        Self {
            name: DEFAULT_THEME_NAME.to_string(),
            terminal: None,
            icon_theme: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct NotificationSettings {
    pub system: bool,
}

impl Default for NotificationSettings {
    fn default() -> Self {
        Self { system: false }
    }
}

pub const AUTO_SHELL: &str = "auto";

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct TerminalSettings {
    pub shell: String,
    pub custom_shells: Vec<String>,
    pub font_family: String,
    pub font_size: f32,
    pub line_height: f32,
    pub padding: f32,
    pub scrollback: usize,
    pub show_scrollbar: bool,
}

impl Default for TerminalSettings {
    fn default() -> Self {
        Self {
            shell: AUTO_SHELL.to_string(),
            custom_shells: Vec::new(),
            font_family: String::new(),
            font_size: 13.0,
            line_height: 1.15,
            padding: 6.0,
            scrollback: 10000,
            show_scrollbar: true,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EditorAutosave {
    #[default]
    Off,
    OnFocusChange,
    AfterDelay,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct EditorSettings {
    pub font_family: String,
    pub font_size: f32,
    pub line_height: f32,
    pub tab_size: usize,
    pub soft_wrap: bool,
    pub line_numbers: bool,
    pub autosave: EditorAutosave,
    pub autosave_delay_ms: u64,
    pub auto_detect_language: bool,
    pub default_language: String,
    pub lsp: EditorLspSettings,
}

impl Default for EditorSettings {
    fn default() -> Self {
        Self {
            font_family: String::new(),
            font_size: 14.0,
            line_height: 1.4,
            tab_size: 4,
            soft_wrap: false,
            line_numbers: true,
            autosave: EditorAutosave::Off,
            autosave_delay_ms: 1000,
            auto_detect_language: true,
            default_language: "plain_text".to_string(),
            lsp: EditorLspSettings::default(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct EditorLspSettings {
    pub enabled: bool,
    pub command: String,
}

impl Default for EditorLspSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            command: String::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct ProjectPanelSettings {
    pub default_open: bool,
    pub show_hidden: bool,
    pub width: f32,
    pub project_sidebar_width: f32,
}

impl Default for ProjectPanelSettings {
    fn default() -> Self {
        Self {
            default_open: true,
            show_hidden: false,
            width: 280.0,
            project_sidebar_width: 216.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct LoadedSettings {
    pub settings: AppSettings,
    pub warnings: Vec<SettingsLoadWarning>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum SettingsLoadWarning {
    InvalidToml { path: PathBuf, message: String },
    InvalidGeneralValue { field: &'static str },
    InvalidTerminalValue { field: &'static str },
    InvalidEditorValue { field: &'static str },
    InvalidProjectPanelValue { field: &'static str },
}

#[derive(Debug, thiserror::Error)]
pub enum SettingsLoadError {
    #[error("failed to create settings config directory {path}: {source}")]
    CreateConfigDirectory {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to read settings file at {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to serialize default settings at {path}: {source}")]
    SerializeDefaults {
        path: PathBuf,
        source: toml::ser::Error,
    },
    #[error("failed to write default settings at {path}: {source}")]
    WriteDefaults {
        path: PathBuf,
        source: std::io::Error,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum SettingsSaveError {
    #[error("failed to create settings config directory {path}: {source}")]
    CreateConfigDirectory {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to serialize settings at {path}: {source}")]
    Serialize {
        path: PathBuf,
        source: toml::ser::Error,
    },
    #[error("failed to write settings at {path}: {source}")]
    Write {
        path: PathBuf,
        source: std::io::Error,
    },
}

pub fn load_or_create_settings(
    paths: &AppConfigPaths,
) -> Result<LoadedSettings, SettingsLoadError> {
    let path = ensure_settings_file(paths)?;
    let source = fs::read_to_string(&path).map_err(|source| SettingsLoadError::Read {
        path: path.clone(),
        source,
    })?;

    let mut warnings = Vec::new();
    let settings = parse_settings_source(&source, &path, &mut warnings);
    let settings = validate_settings(settings, &mut warnings);

    Ok(LoadedSettings { settings, warnings })
}

fn parse_settings_source(
    source: &str,
    path: &Path,
    warnings: &mut Vec<SettingsLoadWarning>,
) -> AppSettings {
    let mut value = match toml::from_str::<toml::Value>(source) {
        Ok(value) => value,
        Err(error) => {
            warnings.push(SettingsLoadWarning::InvalidToml {
                path: path.to_path_buf(),
                message: error.to_string(),
            });
            return AppSettings::default();
        }
    };

    normalize_general_settings(&mut value, warnings);
    normalize_editor_settings(&mut value, warnings);

    match value.try_into::<AppSettings>() {
        Ok(settings) => settings,
        Err(error) => {
            warnings.push(SettingsLoadWarning::InvalidToml {
                path: path.to_path_buf(),
                message: error.to_string(),
            });
            AppSettings::default()
        }
    }
}

fn normalize_general_settings(value: &mut toml::Value, warnings: &mut Vec<SettingsLoadWarning>) {
    let Some(general) = value.get_mut("general").and_then(toml::Value::as_table_mut) else {
        return;
    };
    let Some(language) = general.get("language") else {
        return;
    };

    if matches!(language.as_str(), Some("system" | "en" | "zh-CN")) {
        return;
    }

    general.insert(
        "language".to_string(),
        toml::Value::String("system".to_string()),
    );
    warnings.push(SettingsLoadWarning::InvalidGeneralValue { field: "language" });
}

fn normalize_editor_settings(value: &mut toml::Value, warnings: &mut Vec<SettingsLoadWarning>) {
    let Some(editor) = value.get_mut("editor").and_then(toml::Value::as_table_mut) else {
        return;
    };
    let Some(autosave) = editor.get("autosave") else {
        return;
    };

    if matches!(
        autosave.as_str(),
        Some("off" | "on_focus_change" | "after_delay")
    ) {
        return;
    }

    editor.insert(
        "autosave".to_string(),
        toml::Value::String("off".to_string()),
    );
    warnings.push(SettingsLoadWarning::InvalidEditorValue { field: "autosave" });
}

pub fn save_settings(
    paths: &AppConfigPaths,
    settings: &AppSettings,
) -> Result<PathBuf, SettingsSaveError> {
    let path = paths.settings_file();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| SettingsSaveError::CreateConfigDirectory {
            path: parent.to_path_buf(),
            source,
        })?;
    }

    let source =
        toml::to_string_pretty(settings).map_err(|source| SettingsSaveError::Serialize {
            path: path.clone(),
            source,
        })?;
    fs::write(&path, source).map_err(|source| SettingsSaveError::Write {
        path: path.clone(),
        source,
    })?;

    Ok(path)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShellPlatform {
    Windows,
    MacOs,
    Linux,
}

impl ShellPlatform {
    fn current() -> Self {
        if cfg!(target_os = "windows") {
            Self::Windows
        } else if cfg!(target_os = "macos") {
            Self::MacOs
        } else {
            Self::Linux
        }
    }
}

pub fn detect_shell_candidates() -> Vec<String> {
    let shell_env = std::env::var("SHELL").ok();
    let comspec_env = std::env::var("COMSPEC").ok();
    let path_entries = std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).collect::<Vec<_>>())
        .unwrap_or_default();
    detect_shell_candidates_with(
        ShellPlatform::current(),
        shell_env.as_deref(),
        comspec_env.as_deref(),
        &path_entries,
        Path::exists,
    )
}

pub fn detect_shell_candidates_with(
    platform: ShellPlatform,
    shell_env: Option<&str>,
    comspec_env: Option<&str>,
    path_entries: &[PathBuf],
    exists: impl Fn(&Path) -> bool,
) -> Vec<String> {
    let mut candidates = Vec::new();

    match platform {
        ShellPlatform::Windows => {
            push_existing_shell(&mut candidates, comspec_env, &exists);
            push_path_shells(
                &mut candidates,
                path_entries,
                &["pwsh.exe", "powershell.exe", "cmd.exe", "bash.exe"],
                &exists,
            );
            if candidates.is_empty() {
                push_unique(&mut candidates, "cmd.exe");
            }
        }
        ShellPlatform::MacOs => {
            push_existing_shell(&mut candidates, shell_env, &exists);
            for shell in [
                "/bin/zsh",
                "/bin/bash",
                "/bin/sh",
                "/opt/homebrew/bin/fish",
                "/usr/local/bin/fish",
            ] {
                push_existing_shell(&mut candidates, Some(shell), &exists);
            }
            push_path_shells(
                &mut candidates,
                path_entries,
                &["zsh", "bash", "fish", "sh"],
                &exists,
            );
            if candidates.is_empty() {
                push_unique(&mut candidates, "sh");
            }
        }
        ShellPlatform::Linux => {
            push_existing_shell(&mut candidates, shell_env, &exists);
            for shell in [
                "/bin/bash",
                "/usr/bin/bash",
                "/bin/zsh",
                "/usr/bin/zsh",
                "/usr/bin/fish",
                "/bin/fish",
                "/bin/sh",
            ] {
                push_existing_shell(&mut candidates, Some(shell), &exists);
            }
            push_path_shells(
                &mut candidates,
                path_entries,
                &["bash", "zsh", "fish", "sh"],
                &exists,
            );
            if candidates.is_empty() {
                push_unique(&mut candidates, "sh");
            }
        }
    }

    candidates
}

fn push_existing_shell(
    candidates: &mut Vec<String>,
    shell: Option<&str>,
    exists: &impl Fn(&Path) -> bool,
) {
    let Some(shell) = shell.map(str::trim).filter(|shell| !shell.is_empty()) else {
        return;
    };
    if exists(Path::new(shell)) {
        push_unique(candidates, shell);
    }
}

fn push_path_shells(
    candidates: &mut Vec<String>,
    path_entries: &[PathBuf],
    executable_names: &[&str],
    exists: &impl Fn(&Path) -> bool,
) {
    for executable_name in executable_names {
        if let Some(path) = path_entries
            .iter()
            .map(|directory| directory.join(executable_name))
            .find(|path| exists(path))
        {
            push_unique(candidates, &path.to_string_lossy());
        }
    }
}

pub fn resolve_default_shell(shell: &str, candidates: &[String]) -> String {
    let shell = shell.trim();
    if !shell.is_empty() && shell != AUTO_SHELL {
        return shell.to_string();
    }

    candidates
        .first()
        .cloned()
        .unwrap_or_else(|| "sh".to_string())
}

fn ensure_settings_file(paths: &AppConfigPaths) -> Result<PathBuf, SettingsLoadError> {
    let path = paths.settings_file();
    if path.exists() {
        return Ok(path);
    }

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| SettingsLoadError::CreateConfigDirectory {
            path: parent.to_path_buf(),
            source,
        })?;
    }

    let source = toml::to_string_pretty(&AppSettings::default()).map_err(|source| {
        SettingsLoadError::SerializeDefaults {
            path: path.clone(),
            source,
        }
    })?;
    fs::write(&path, source).map_err(|source| SettingsLoadError::WriteDefaults {
        path: path.clone(),
        source,
    })?;

    Ok(path)
}

fn validate_settings(
    mut settings: AppSettings,
    warnings: &mut Vec<SettingsLoadWarning>,
) -> AppSettings {
    let defaults = TerminalSettings::default();

    if settings.terminal.font_size <= 0.0 {
        settings.terminal.font_size = defaults.font_size;
        warnings.push(SettingsLoadWarning::InvalidTerminalValue { field: "font_size" });
    }
    if settings.terminal.line_height <= 0.0 {
        settings.terminal.line_height = defaults.line_height;
        warnings.push(SettingsLoadWarning::InvalidTerminalValue {
            field: "line_height",
        });
    }
    if settings.terminal.padding < 0.0 {
        settings.terminal.padding = defaults.padding;
        warnings.push(SettingsLoadWarning::InvalidTerminalValue { field: "padding" });
    }
    if settings.terminal.scrollback == 0 {
        settings.terminal.scrollback = defaults.scrollback;
        warnings.push(SettingsLoadWarning::InvalidTerminalValue {
            field: "scrollback",
        });
    }
    if settings.terminal.shell.trim().is_empty() {
        settings.terminal.shell = defaults.shell;
        warnings.push(SettingsLoadWarning::InvalidTerminalValue { field: "shell" });
    }
    let custom_shells = std::mem::take(&mut settings.terminal.custom_shells);
    for shell in custom_shells {
        let shell = shell.trim();
        if !shell.is_empty() {
            push_unique(&mut settings.terminal.custom_shells, shell);
        }
    }

    let editor_defaults = EditorSettings::default();
    settings.editor.font_family = settings.editor.font_family.trim().to_string();
    if !settings.editor.font_size.is_finite() || !(6.0..=72.0).contains(&settings.editor.font_size)
    {
        settings.editor.font_size = editor_defaults.font_size;
        warnings.push(SettingsLoadWarning::InvalidEditorValue { field: "font_size" });
    }
    if !settings.editor.line_height.is_finite() || settings.editor.line_height < 1.0 {
        settings.editor.line_height = editor_defaults.line_height;
        warnings.push(SettingsLoadWarning::InvalidEditorValue {
            field: "line_height",
        });
    }
    if !(1..=16).contains(&settings.editor.tab_size) {
        settings.editor.tab_size = editor_defaults.tab_size;
        warnings.push(SettingsLoadWarning::InvalidEditorValue { field: "tab_size" });
    }
    if settings.editor.autosave_delay_ms < 50 {
        settings.editor.autosave_delay_ms = editor_defaults.autosave_delay_ms;
        warnings.push(SettingsLoadWarning::InvalidEditorValue {
            field: "autosave_delay_ms",
        });
    }
    if settings.editor.default_language.trim().is_empty() {
        settings.editor.default_language = editor_defaults.default_language;
        warnings.push(SettingsLoadWarning::InvalidEditorValue {
            field: "default_language",
        });
    }
    settings.editor.default_language = settings.editor.default_language.trim().to_string();
    settings.editor.lsp.command = settings.editor.lsp.command.trim().to_string();

    let project_panel_defaults = ProjectPanelSettings::default();
    if !settings.project_panel.width.is_finite() {
        settings.project_panel.width = project_panel_defaults.width;
        warnings.push(SettingsLoadWarning::InvalidProjectPanelValue { field: "width" });
    } else if !(200.0..=520.0).contains(&settings.project_panel.width) {
        settings.project_panel.width = settings.project_panel.width.clamp(200.0, 520.0);
        warnings.push(SettingsLoadWarning::InvalidProjectPanelValue { field: "width" });
    }
    if !settings.project_panel.project_sidebar_width.is_finite() {
        settings.project_panel.project_sidebar_width = project_panel_defaults.project_sidebar_width;
        warnings.push(SettingsLoadWarning::InvalidProjectPanelValue {
            field: "project_sidebar_width",
        });
    } else if !(160.0..=420.0).contains(&settings.project_panel.project_sidebar_width) {
        settings.project_panel.project_sidebar_width = settings
            .project_panel
            .project_sidebar_width
            .clamp(160.0, 420.0);
        warnings.push(SettingsLoadWarning::InvalidProjectPanelValue {
            field: "project_sidebar_width",
        });
    }

    settings
}

fn push_unique(values: &mut Vec<String>, value: &str) {
    if values.iter().all(|existing| existing != value) {
        values.push(value.to_string());
    }
}
