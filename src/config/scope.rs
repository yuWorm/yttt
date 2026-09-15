use std::{
    fs,
    io::{self, Write as _},
    path::{Path, PathBuf},
    sync::LazyLock,
};

use fs2::FileExt as _;
use parking_lot::RwLock;

use super::{
    bars::{BarsLoadError, BarsLoadWarning, BarsSaveError, load_bars, save_bars},
    paths::AppConfigPaths,
    profile::AppProfile,
    settings::AppSettings,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsScope {
    Device,
    Host,
    Project,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingApply {
    Immediate,
    NewSession,
    ReopenFile,
}

const SETTING_KEYS: &[&str] = &[
    "general.language",
    "general.ui_font_family",
    "general.ui_font_size",
    "general.ui_line_height",
    "general.onboarding_completed",
    "general.performance_metrics_enabled",
    "general.system_performance_metrics_enabled",
    "general.auto_check_updates",
    "general.restore_last_session",
    "general.new_tab_command_picker_enabled",
    "general.new_tab_commands",
    "window.effect",
    "window.opacity",
    "theme.name",
    "theme.ui_style",
    "theme.terminal",
    "theme.icon_theme",
    "notifications.system",
    "agent.primary",
    "agent.sessions_enabled",
    "agent.additional_session_agents",
    "terminal.shell",
    "terminal.custom_shells",
    "terminal.environment",
    "terminal.font_family",
    "terminal.font_size",
    "terminal.line_height",
    "terminal.padding",
    "terminal.scrollback",
    "terminal.show_scrollbar",
    "terminal.cursor_shape",
    "terminal.cursor_blinking",
    "terminal.cursor_blink_interval_ms",
    "terminal.cursor_blink_timeout_secs",
    "terminal.cursor_unfocused_hollow",
    "terminal.cursor_thickness",
    "terminal.hide_mouse_when_typing",
    "terminal.copy_on_select",
    "terminal.semantic_escape_chars",
    "terminal.osc52_policy",
    "terminal.kitty_keyboard",
    "terminal.hint_alphabet",
    "terminal.hints",
    "editor.font_family",
    "editor.font_size",
    "editor.line_height",
    "editor.tab_size",
    "editor.soft_wrap",
    "editor.line_numbers",
    "editor.autosave",
    "editor.autosave_delay_ms",
    "editor.auto_detect_language",
    "editor.default_language",
    "editor.lsp.enabled",
    "editor.lsp.command",
    "vim.mode",
    "bars",
    "project_panel.default_open",
    "project_panel.show_hidden",
    "project_panel.width",
    "project_panel.project_sidebar_width",
    "project_panel.collapsed_agent_projects",
];

pub fn setting_keys() -> &'static [&'static str] {
    SETTING_KEYS
}

pub fn setting_scope(key: &str) -> SettingsScope {
    match key {
        "general.new_tab_commands"
        | "terminal.shell"
        | "terminal.custom_shells"
        | "terminal.environment"
        | "terminal.scrollback"
        | "terminal.kitty_keyboard"
        | "editor.tab_size"
        | "editor.auto_detect_language"
        | "editor.default_language"
        | "editor.lsp" => SettingsScope::Host,
        _ if key.starts_with("agent.")
            || key.starts_with("terminal.environment.")
            || key.starts_with("default_layout.")
            || key.starts_with("editor.lsp.") =>
        {
            SettingsScope::Host
        }
        _ => SettingsScope::Device,
    }
}

/// Project settings may override these Host defaults for the opened project only.
pub fn supports_project_override(key: &str) -> bool {
    matches!(
        key,
        "editor.tab_size" | "editor.auto_detect_language" | "editor.default_language"
    )
}

pub fn setting_apply(key: &str) -> SettingApply {
    match key {
        "terminal.shell"
        | "terminal.custom_shells"
        | "terminal.environment"
        | "terminal.scrollback"
        | "terminal.kitty_keyboard" => SettingApply::NewSession,
        "editor.tab_size"
        | "editor.auto_detect_language"
        | "editor.default_language"
        | "editor.lsp"
        | "editor.lsp.enabled"
        | "editor.lsp.command" => SettingApply::ReopenFile,
        _ if key.starts_with("agent.") || key.starts_with("terminal.environment.") => {
            SettingApply::NewSession
        }
        _ => SettingApply::Immediate,
    }
}

#[derive(Clone)]
struct DeviceProfile {
    profile: AppProfile,
    preferences_root: PathBuf,
    state_dir: PathBuf,
}

static DEVICE_PROFILE: LazyLock<RwLock<Option<DeviceProfile>>> =
    LazyLock::new(|| RwLock::new(None));

/// Bind the local profile before any Host connection. This is the only source from which legacy
/// local preferences may be migrated; a connected Host is never considered a migration source.
pub fn bind_device_profile(profile: &AppProfile) -> io::Result<()> {
    let binding = DeviceProfile {
        profile: profile.clone(),
        preferences_root: profile.paths().config.join("device"),
        state_dir: profile.paths().state.join("device"),
    };

    {
        let current = DEVICE_PROFILE.read();
        if let Some(current) = current.as_ref() {
            if current.preferences_root != binding.preferences_root {
                return Err(io::Error::other(
                    "a Client process cannot switch device preference profiles",
                ));
            }
            return Ok(());
        }
    }

    migrate_legacy_device_preferences(profile.paths().config.as_path(), &binding.preferences_root)?;
    super::storage::bind_device_preferences_root(binding.preferences_root.clone())?;
    *DEVICE_PROFILE.write() = Some(binding);
    Ok(())
}

pub fn device_preferences_root() -> Option<PathBuf> {
    DEVICE_PROFILE
        .read()
        .as_ref()
        .map(|binding| binding.preferences_root.clone())
}

pub fn device_state_dir() -> Option<PathBuf> {
    DEVICE_PROFILE
        .read()
        .as_ref()
        .map(|binding| binding.state_dir.clone())
}

pub fn device_preferences_config_paths() -> Option<AppConfigPaths> {
    DEVICE_PROFILE.read().as_ref().map(|binding| {
        AppConfigPaths::from_device_preferences(&binding.profile, binding.preferences_root.clone())
    })
}

pub fn device_settings_file() -> Option<PathBuf> {
    device_preferences_config_paths().map(|paths| paths.settings_file())
}

pub fn device_themes_dir() -> Option<PathBuf> {
    device_preferences_config_paths().map(|paths| paths.themes_dir())
}

pub fn device_icon_themes_dir() -> Option<PathBuf> {
    device_preferences_config_paths().map(|paths| paths.icon_themes_dir())
}

pub fn device_keybindings_file() -> Option<PathBuf> {
    device_preferences_config_paths().map(|paths| paths.keybindings_file())
}

pub fn device_bars_file() -> Option<PathBuf> {
    device_preferences_config_paths().map(|paths| paths.bars_file())
}

#[derive(Debug, thiserror::Error)]
pub enum DevicePreferencesLoadError {
    #[error("Device preferences are not bound for this Client profile")]
    DevicePreferencesUnbound,
    #[error("failed to read Device preferences at {path}: {source}")]
    Read { path: PathBuf, source: io::Error },
    #[error("failed to parse Device preferences at {path}: {source}")]
    Parse {
        path: PathBuf,
        source: toml::de::Error,
    },
    #[error("failed to load Device bar preferences: {0}")]
    Bars(#[from] BarsLoadError),
}

pub fn merge_device_preferences(
    paths: &AppConfigPaths,
    settings: &mut AppSettings,
) -> Result<(), DevicePreferencesLoadError> {
    merge_device_preferences_with_warnings(paths, settings).map(|_| ())
}

/// Merge Device-owned settings and return non-fatal Device bar validation warnings without
/// requiring a second filesystem read.
pub fn merge_device_preferences_with_warnings(
    paths: &AppConfigPaths,
    settings: &mut AppSettings,
) -> Result<Vec<BarsLoadWarning>, DevicePreferencesLoadError> {
    if paths.is_test_fixture() {
        // A fixture has one asset root, but Host and Device settings still need distinct files.
        let path = paths.config_dir().join("device/settings.toml");
        if let Some(device) = read_optional_device_settings(&path)? {
            merge_device_settings(settings, &device);
        }
        return match fs::metadata(paths.bars_file()) {
            Ok(_) => {
                let loaded = load_bars(paths)?;
                settings.bars = loaded.settings;
                Ok(loaded.warnings)
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(source) => Err(DevicePreferencesLoadError::Read {
                path: paths.bars_file(),
                source,
            }),
        };
    }
    let Some(device_paths) = device_preferences_config_paths() else {
        return Err(DevicePreferencesLoadError::DevicePreferencesUnbound);
    };
    let path = device_paths.settings_file();
    let device = read_device_settings(&path)?;
    merge_device_settings(settings, &device);

    let loaded_bars = load_bars(&device_paths)?;
    settings.bars = loaded_bars.settings;
    Ok(loaded_bars.warnings)
}

fn read_device_settings(path: &Path) -> Result<AppSettings, DevicePreferencesLoadError> {
    Ok(read_optional_device_settings(path)?.unwrap_or_default())
}

fn read_optional_device_settings(
    path: &Path,
) -> Result<Option<AppSettings>, DevicePreferencesLoadError> {
    match fs::read_to_string(path) {
        Ok(source) => toml::from_str::<AppSettings>(&source)
            .map(Some)
            .map_err(|source| DevicePreferencesLoadError::Parse {
                path: path.to_path_buf(),
                source,
            }),
        Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(DevicePreferencesLoadError::Read {
            path: path.to_path_buf(),
            source,
        }),
    }
}

fn merge_device_settings(settings: &mut AppSettings, device: &AppSettings) {
    settings.general.language = device.general.language;
    settings
        .general
        .ui_font_family
        .clone_from(&device.general.ui_font_family);
    settings.general.ui_font_size = device.general.ui_font_size;
    settings.general.ui_line_height = device.general.ui_line_height;
    settings.general.onboarding_completed = device.general.onboarding_completed;
    settings.general.performance_metrics_enabled = device.general.performance_metrics_enabled;
    settings.general.system_performance_metrics_enabled =
        device.general.system_performance_metrics_enabled;
    settings.general.auto_check_updates = device.general.auto_check_updates;
    settings.general.restore_last_session = device.general.restore_last_session;
    settings.general.new_tab_command_picker_enabled = device.general.new_tab_command_picker_enabled;

    settings.window = device.window;
    settings.theme.clone_from(&device.theme);
    settings.notifications.clone_from(&device.notifications);

    settings
        .terminal
        .font_family
        .clone_from(&device.terminal.font_family);
    settings.terminal.font_size = device.terminal.font_size;
    settings.terminal.line_height = device.terminal.line_height;
    settings.terminal.padding = device.terminal.padding;
    settings.terminal.show_scrollbar = device.terminal.show_scrollbar;
    settings.terminal.cursor_shape = device.terminal.cursor_shape;
    settings.terminal.cursor_blinking = device.terminal.cursor_blinking;
    settings.terminal.cursor_blink_interval_ms = device.terminal.cursor_blink_interval_ms;
    settings.terminal.cursor_blink_timeout_secs = device.terminal.cursor_blink_timeout_secs;
    settings.terminal.cursor_unfocused_hollow = device.terminal.cursor_unfocused_hollow;
    settings.terminal.cursor_thickness = device.terminal.cursor_thickness;
    settings.terminal.hide_mouse_when_typing = device.terminal.hide_mouse_when_typing;
    settings.terminal.copy_on_select = device.terminal.copy_on_select;
    settings
        .terminal
        .semantic_escape_chars
        .clone_from(&device.terminal.semantic_escape_chars);
    settings.terminal.osc52_policy = device.terminal.osc52_policy;
    settings
        .terminal
        .hint_alphabet
        .clone_from(&device.terminal.hint_alphabet);
    settings.terminal.hints.clone_from(&device.terminal.hints);

    settings
        .editor
        .font_family
        .clone_from(&device.editor.font_family);
    settings.editor.font_size = device.editor.font_size;
    settings.editor.line_height = device.editor.line_height;
    settings.editor.soft_wrap = device.editor.soft_wrap;
    settings.editor.line_numbers = device.editor.line_numbers;
    settings.editor.autosave = device.editor.autosave;
    settings.editor.autosave_delay_ms = device.editor.autosave_delay_ms;

    settings.vim.clone_from(&device.vim);
    settings.project_panel.clone_from(&device.project_panel);
}

#[derive(Debug, thiserror::Error)]
pub enum ScopedSettingsSaveError {
    #[error("Device preferences are not bound; cannot save Device settings")]
    DevicePreferencesUnbound,
    #[error("a single save cannot mix Device and Host settings")]
    MixedScopeChange,
    #[error("Host settings changed but this Client does not have Host write control: {path}")]
    HostWriteDenied { path: PathBuf },
    #[error("Host settings changed since this draft was loaded: {path}")]
    HostConflict { path: PathBuf },
    #[error("Device settings changed since this draft was loaded: {path}")]
    DeviceConflict { path: PathBuf },
    #[error("failed to reload Device preferences: {0}")]
    LoadDevice(#[from] DevicePreferencesLoadError),
    #[error("failed to reload Host settings at {path}: {source}")]
    LoadHost {
        path: PathBuf,
        source: super::settings::SettingsLoadError,
    },
    #[error("failed to create {scope:?} settings directory {path}: {source}")]
    CreateDirectory {
        scope: SettingsScope,
        path: PathBuf,
        source: io::Error,
    },
    #[error("failed to serialize {scope:?} settings for {path}: {source}")]
    Serialize {
        scope: SettingsScope,
        path: PathBuf,
        source: toml::ser::Error,
    },
    #[error("failed to write {scope:?} settings at {path}: {source}")]
    Write {
        scope: SettingsScope,
        path: PathBuf,
        source: io::Error,
    },
    #[error("failed to save Device bar preferences: {0}")]
    Bars(#[from] BarsSaveError),
}

/// Persist settings for precisely one target. Mixed Device and Host changes are rejected before
/// either backend is touched, so callers never report a partially saved effective view.
pub fn save_scoped_settings(
    paths: &AppConfigPaths,
    candidate: &AppSettings,
    confirmed: &AppSettings,
    can_write_host: bool,
) -> Result<(), ScopedSettingsSaveError> {
    let device_paths = paths
        .is_test_fixture()
        .then(|| paths.clone())
        .or_else(device_preferences_config_paths)
        .ok_or(ScopedSettingsSaveError::DevicePreferencesUnbound)?;
    save_scoped_settings_to_paths(paths, &device_paths, candidate, confirmed, can_write_host)
}

/// Whether a candidate changes a Host-owned setting.
pub fn host_settings_changed(candidate: &AppSettings, confirmed: &AppSettings) -> bool {
    candidate.general.new_tab_commands != confirmed.general.new_tab_commands
        || candidate.agent != confirmed.agent
        || candidate.terminal.shell != confirmed.terminal.shell
        || candidate.terminal.custom_shells != confirmed.terminal.custom_shells
        || candidate.terminal.environment != confirmed.terminal.environment
        || candidate.terminal.scrollback != confirmed.terminal.scrollback
        || candidate.terminal.kitty_keyboard != confirmed.terminal.kitty_keyboard
        || candidate.editor.tab_size != confirmed.editor.tab_size
        || candidate.editor.auto_detect_language != confirmed.editor.auto_detect_language
        || candidate.editor.default_language != confirmed.editor.default_language
        || candidate.editor.lsp != confirmed.editor.lsp
}

fn device_settings_changed(candidate: &AppSettings, confirmed: &AppSettings) -> bool {
    candidate.general.language != confirmed.general.language
        || candidate.general.ui_font_family != confirmed.general.ui_font_family
        || candidate.general.ui_font_size != confirmed.general.ui_font_size
        || candidate.general.ui_line_height != confirmed.general.ui_line_height
        || candidate.general.onboarding_completed != confirmed.general.onboarding_completed
        || candidate.general.performance_metrics_enabled
            != confirmed.general.performance_metrics_enabled
        || candidate.general.system_performance_metrics_enabled
            != confirmed.general.system_performance_metrics_enabled
        || candidate.general.auto_check_updates != confirmed.general.auto_check_updates
        || candidate.general.restore_last_session != confirmed.general.restore_last_session
        || candidate.general.new_tab_command_picker_enabled
            != confirmed.general.new_tab_command_picker_enabled
        || candidate.window != confirmed.window
        || candidate.theme != confirmed.theme
        || candidate.notifications != confirmed.notifications
        || candidate.terminal.font_family != confirmed.terminal.font_family
        || candidate.terminal.font_size != confirmed.terminal.font_size
        || candidate.terminal.line_height != confirmed.terminal.line_height
        || candidate.terminal.padding != confirmed.terminal.padding
        || candidate.terminal.show_scrollbar != confirmed.terminal.show_scrollbar
        || candidate.terminal.cursor_shape != confirmed.terminal.cursor_shape
        || candidate.terminal.cursor_blinking != confirmed.terminal.cursor_blinking
        || candidate.terminal.cursor_blink_interval_ms
            != confirmed.terminal.cursor_blink_interval_ms
        || candidate.terminal.cursor_blink_timeout_secs
            != confirmed.terminal.cursor_blink_timeout_secs
        || candidate.terminal.cursor_unfocused_hollow != confirmed.terminal.cursor_unfocused_hollow
        || candidate.terminal.cursor_thickness != confirmed.terminal.cursor_thickness
        || candidate.terminal.hide_mouse_when_typing != confirmed.terminal.hide_mouse_when_typing
        || candidate.terminal.copy_on_select != confirmed.terminal.copy_on_select
        || candidate.terminal.semantic_escape_chars != confirmed.terminal.semantic_escape_chars
        || candidate.terminal.osc52_policy != confirmed.terminal.osc52_policy
        || candidate.terminal.hint_alphabet != confirmed.terminal.hint_alphabet
        || candidate.terminal.hints != confirmed.terminal.hints
        || candidate.editor.font_family != confirmed.editor.font_family
        || candidate.editor.font_size != confirmed.editor.font_size
        || candidate.editor.line_height != confirmed.editor.line_height
        || candidate.editor.soft_wrap != confirmed.editor.soft_wrap
        || candidate.editor.line_numbers != confirmed.editor.line_numbers
        || candidate.editor.autosave != confirmed.editor.autosave
        || candidate.editor.autosave_delay_ms != confirmed.editor.autosave_delay_ms
        || candidate.vim != confirmed.vim
        || candidate.project_panel != confirmed.project_panel
}

fn save_scoped_settings_to_paths(
    host_paths: &AppConfigPaths,
    device_paths: &AppConfigPaths,
    candidate: &AppSettings,
    confirmed: &AppSettings,
    can_write_host: bool,
) -> Result<(), ScopedSettingsSaveError> {
    let device_changed = device_settings_changed(candidate, confirmed);
    let bars_changed = candidate.bars != confirmed.bars;
    let host_changed = host_settings_changed(candidate, confirmed);

    if (device_changed || bars_changed) && host_changed {
        return Err(ScopedSettingsSaveError::MixedScopeChange);
    }
    if host_changed && !can_write_host {
        return Err(ScopedSettingsSaveError::HostWriteDenied {
            path: host_paths.settings_file(),
        });
    }
    if host_changed {
        let current = super::settings::load_settings(host_paths).map_err(|source| {
            ScopedSettingsSaveError::LoadHost {
                path: host_paths.settings_file(),
                source,
            }
        })?;
        if host_settings_changed(&current.settings, confirmed) {
            return Err(ScopedSettingsSaveError::HostConflict {
                path: host_paths.settings_file(),
            });
        }
    }
    if device_changed {
        let device_file = if device_paths.is_test_fixture() && device_paths == host_paths {
            device_paths.config_dir().join("device/settings.toml")
        } else {
            device_paths.settings_file()
        };
        // Lock a stable sibling: locking settings.toml itself would not protect its replacement.
        let _device_lock = lock_device_settings(&device_file).map_err(|source| {
            ScopedSettingsSaveError::Write {
                scope: SettingsScope::Device,
                path: device_file.clone(),
                source,
            }
        })?;
        let current = match read_optional_device_settings(&device_file)? {
            Some(settings) => super::settings::validate_settings(settings, &mut Vec::new()),
            None if device_paths.is_test_fixture() && device_paths == host_paths => {
                // Single-root fixtures may still load their initial Device values from legacy TOML.
                super::settings::load_settings(host_paths)
                    .map_err(|source| ScopedSettingsSaveError::LoadHost {
                        path: host_paths.settings_file(),
                        source,
                    })?
                    .settings
            }
            None => AppSettings::default(),
        };
        if device_settings_changed(&current, confirmed) {
            return Err(ScopedSettingsSaveError::DeviceConflict { path: device_file });
        }
        write_scoped_settings(&device_file, candidate, SettingsScope::Device)?;
    }
    if bars_changed {
        save_bars(device_paths, &candidate.bars)?;
    }
    if host_changed {
        write_scoped_settings(&host_paths.settings_file(), candidate, SettingsScope::Host)?;
    }
    Ok(())
}

fn lock_device_settings(path: &Path) -> io::Result<fs::File> {
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "settings path has no parent")
    })?;
    fs::create_dir_all(parent)?;
    let mut options = fs::OpenOptions::new();
    options.create(true).read(true).write(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let lock = options.open(path.with_extension("toml.lock"))?;
    lock.lock_exclusive()?;
    Ok(lock)
}

fn write_scoped_settings(
    path: &Path,
    settings: &AppSettings,
    scope: SettingsScope,
) -> Result<(), ScopedSettingsSaveError> {
    if let Some(parent) = path.parent() {
        super::storage::create_dir_all(parent).map_err(|source| {
            ScopedSettingsSaveError::CreateDirectory {
                scope,
                path: parent.to_path_buf(),
                source,
            }
        })?;
    }
    let source = scoped_settings_source(settings, scope).map_err(|source| {
        ScopedSettingsSaveError::Serialize {
            scope,
            path: path.to_path_buf(),
            source,
        }
    })?;
    super::atomic_write(path, source.as_bytes()).map_err(|source| ScopedSettingsSaveError::Write {
        scope,
        path: path.to_path_buf(),
        source,
    })
}

fn scoped_settings_source(
    settings: &AppSettings,
    scope: SettingsScope,
) -> Result<String, toml::ser::Error> {
    let value = toml::Value::try_from(settings)?;
    toml::to_string_pretty(&filter_scope(value, "", scope))
}

fn filter_scope(value: toml::Value, prefix: &str, scope: SettingsScope) -> toml::Value {
    let toml::Value::Table(table) = value else {
        return value;
    };

    let filtered = table
        .into_iter()
        .filter_map(|(key, value)| {
            let path = if prefix.is_empty() {
                key.clone()
            } else {
                format!("{prefix}.{key}")
            };
            if is_concrete_setting(&path) {
                return (setting_scope(&path) == scope).then_some((key, value));
            }
            let value = filter_scope(value, &path, scope);
            match &value {
                toml::Value::Table(entries) if entries.is_empty() => None,
                _ => Some((key, value)),
            }
        })
        .collect();
    toml::Value::Table(filtered)
}

fn is_concrete_setting(path: &str) -> bool {
    SETTING_KEYS.contains(&path) || matches!(path, "editor.lsp")
}

fn migrate_legacy_device_preferences(legacy_root: &Path, device_root: &Path) -> io::Result<()> {
    let marker = device_root.join("migration-v1.complete");
    if marker.try_exists()? {
        return Ok(());
    }
    for file in ["keybindings.toml", "bars.toml"] {
        copy_file_durably_if_missing(&legacy_root.join(file), &device_root.join(file))?;
    }
    migrate_legacy_settings(legacy_root, device_root)?;
    copy_directory_durably_if_missing(&legacy_root.join("themes"), &device_root.join("themes"))?;
    atomic_local_write(&marker, b"1\n")
}

fn migrate_legacy_settings(legacy_root: &Path, device_root: &Path) -> io::Result<()> {
    let source_path = legacy_root.join("settings.toml");
    let destination = device_root.join("settings.toml");
    let bars_destination = device_root.join("bars.toml");
    if (destination.try_exists()? && bars_destination.try_exists()?) || !source_path.try_exists()? {
        return Ok(());
    }

    let source = fs::read_to_string(&source_path)?;
    toml::from_str::<toml::Value>(&source).map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "failed to parse legacy Device preferences at {}: {error}",
                source_path.display()
            ),
        )
    })?;
    let (settings, _, legacy_bars) =
        super::settings::parse_settings_source(&source, &source_path, &mut Vec::new());
    if !destination.try_exists()? {
        let source = scoped_settings_source(&settings, SettingsScope::Device).map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "failed to serialize migrated Device preferences at {}: {error}",
                    destination.display()
                ),
            )
        })?;
        atomic_local_write(&destination, source.as_bytes())?;
    }
    if !bars_destination.try_exists()?
        && let Some(mut bars) = legacy_bars
    {
        bars.validate();
        let source = toml::to_string_pretty(&bars).map_err(io::Error::other)?;
        atomic_local_write(&bars_destination, source.as_bytes())?;
    }
    Ok(())
}

fn copy_file_durably_if_missing(source: &Path, destination: &Path) -> io::Result<()> {
    if destination.try_exists()? || !source.try_exists()? {
        return Ok(());
    }
    atomic_local_write(destination, &fs::read(source)?)
}

fn copy_directory_durably_if_missing(source: &Path, destination: &Path) -> io::Result<()> {
    if destination.try_exists()? || !source.try_exists()? {
        return Ok(());
    }
    if !fs::metadata(source)?.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotADirectory,
            format!("{} is not a theme directory", source.display()),
        ));
    }
    let parent = destination.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "Device theme directory has no parent: {}",
                destination.display()
            ),
        )
    })?;
    fs::create_dir_all(parent)?;
    let temporary = tempfile::Builder::new()
        .prefix(".yttt-device-themes-")
        .tempdir_in(parent)?;
    copy_directory_contents(source, temporary.path())?;
    sync_directory_tree(temporary.path())?;
    fs::rename(temporary.path(), destination)?;
    sync_directory(parent)
}

fn copy_directory_contents(source: &Path, destination: &Path) -> io::Result<()> {
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let source = entry.path();
        let destination = destination.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_directory_contents(&source, &destination)?;
        } else if entry.file_type()?.is_file() {
            atomic_local_write(&destination, &fs::read(source)?)?;
        }
    }
    Ok(())
}

fn sync_directory_tree(path: &Path) -> io::Result<()> {
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            sync_directory_tree(&entry.path())?;
        }
    }
    sync_directory(path)
}

fn atomic_local_write(path: &Path, source: &[u8]) -> io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("Device preference file has no parent: {}", path.display()),
        )
    })?;
    fs::create_dir_all(parent)?;
    let mut temporary = tempfile::Builder::new()
        .prefix(".yttt-device-")
        .suffix(".tmp")
        .tempfile_in(parent)?;
    temporary.write_all(source)?;
    temporary.as_file_mut().sync_all()?;
    temporary.persist(path).map_err(|error| error.error)?;
    sync_directory(parent)
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> io::Result<()> {
    fs::File::open(path)?.sync_all()
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_metadata_marks_the_complete_host_execution_surface() {
        for key in [
            "general.new_tab_commands",
            "agent.primary",
            "agent.sessions_enabled",
            "agent.additional_session_agents",
            "terminal.shell",
            "terminal.custom_shells",
            "terminal.environment",
            "terminal.environment.PATH",
            "terminal.scrollback",
            "terminal.kitty_keyboard",
            "editor.tab_size",
            "editor.auto_detect_language",
            "editor.default_language",
            "editor.lsp.enabled",
            "editor.lsp.command",
        ] {
            assert_eq!(setting_scope(key), SettingsScope::Host, "{key}");
        }
        for key in setting_keys() {
            assert_ne!(setting_scope(key), SettingsScope::Project, "{key}");
        }
        assert!(supports_project_override("editor.tab_size"));
        assert!(supports_project_override("editor.auto_detect_language"));
        assert!(supports_project_override("editor.default_language"));
        assert!(!supports_project_override("editor.lsp.enabled"));
    }

    #[test]
    fn device_merge_does_not_overwrite_host_execution_settings() {
        let mut host = AppSettings::default();
        host.general.new_tab_commands = vec!["host-command".into()];
        host.agent.sessions_enabled = false;
        host.terminal.shell = "bash".into();
        host.terminal.scrollback = 42;
        host.editor.tab_size = 8;
        host.editor.lsp.enabled = true;

        let mut device = AppSettings::default();
        device.general.ui_font_size = 18.0;
        device.terminal.font_size = 15.0;
        device.editor.font_size = 16.0;
        merge_device_settings(&mut host, &device);

        assert_eq!(host.general.ui_font_size, 18.0);
        assert_eq!(host.terminal.font_size, 15.0);
        assert_eq!(host.editor.font_size, 16.0);
        assert_eq!(host.general.new_tab_commands, ["host-command"]);
        assert!(!host.agent.sessions_enabled);
        assert_eq!(host.terminal.shell, "bash");
        assert_eq!(host.terminal.scrollback, 42);
        assert_eq!(host.editor.tab_size, 8);
        assert!(host.editor.lsp.enabled);
    }

    #[test]
    fn missing_device_preferences_reset_remote_appearance_to_device_builtins() {
        let defaults = AppSettings::default();
        let mut remote_host = AppSettings::default();
        remote_host.general.ui_font_size = 22.0;
        remote_host.window.opacity = 0.2;
        remote_host.theme.name = "remote-theme".into();
        remote_host.terminal.font_size = 19.0;
        remote_host.editor.font_size = 20.0;
        remote_host.terminal.shell = "remote-shell".into();
        remote_host.editor.tab_size = 8;

        let root = tempfile::tempdir().unwrap();
        let device_file = root.path().join("device").join("settings.toml");
        fs::create_dir_all(device_file.parent().unwrap()).unwrap();
        fs::write(&device_file, "[general]\nui_font_size = 17.0\n").unwrap();
        fs::remove_file(&device_file).unwrap();

        let missing_device = read_device_settings(&device_file).unwrap();
        merge_device_settings(&mut remote_host, &missing_device);

        assert_eq!(
            remote_host.general.ui_font_size,
            defaults.general.ui_font_size
        );
        assert_eq!(remote_host.window, defaults.window);
        assert_eq!(remote_host.theme, defaults.theme);
        assert_eq!(remote_host.terminal.font_size, defaults.terminal.font_size);
        assert_eq!(remote_host.editor.font_size, defaults.editor.font_size);
        assert_eq!(remote_host.terminal.shell, "remote-shell");
        assert_eq!(remote_host.editor.tab_size, 8);
    }

    #[test]
    fn observer_save_persists_device_without_host_write() {
        let root = tempfile::tempdir().unwrap();
        let host = AppConfigPaths::from_config_dir(root.path().join("host"));
        let device = AppConfigPaths::from_config_dir(root.path().join("device"));
        let confirmed = AppSettings::default();
        let mut candidate = confirmed.clone();
        candidate.general.ui_font_size = 18.0;

        save_scoped_settings_to_paths(&host, &device, &candidate, &confirmed, false).unwrap();

        assert_eq!(
            read_device_settings(&device.settings_file())
                .unwrap()
                .general
                .ui_font_size,
            18.0
        );
        assert!(!host.settings_file().exists());
    }

    #[test]
    fn stale_device_save_cannot_overwrite_another_clients_preferences() {
        let root = tempfile::tempdir().unwrap();
        let host = AppConfigPaths::from_config_dir(root.path().join("host"));
        let device = AppConfigPaths::from_config_dir(root.path().join("device"));
        let confirmed = AppSettings::default();
        let mut first = confirmed.clone();
        first.general.ui_font_size = 18.0;
        let mut stale = confirmed.clone();
        stale.editor.font_size = 20.0;

        save_scoped_settings_to_paths(&host, &device, &first, &confirmed, false).unwrap();
        let second = save_scoped_settings_to_paths(&host, &device, &stale, &confirmed, false);
        let saved = read_device_settings(&device.settings_file()).unwrap();
        assert_eq!(saved.general.ui_font_size, 18.0);
        assert_eq!(saved.editor.font_size, confirmed.editor.font_size);
        assert!(matches!(
            second,
            Err(ScopedSettingsSaveError::DeviceConflict { .. })
        ));

        let mut refreshed = saved;
        refreshed.terminal.shell = "another-host-shell".into();
        let mut retried = refreshed.clone();
        retried.editor.font_size = 20.0;
        save_scoped_settings_to_paths(&host, &device, &retried, &refreshed, false).unwrap();
        let saved = read_device_settings(&device.settings_file()).unwrap();
        assert_eq!(saved.general.ui_font_size, 18.0);
        assert_eq!(saved.editor.font_size, 20.0);
        assert!(!host.settings_file().exists());
    }

    #[test]
    fn simultaneous_device_saves_accept_only_one_shared_baseline() {
        let root = tempfile::tempdir().unwrap();
        let host = AppConfigPaths::from_config_dir(root.path().join("host"));
        let device = AppConfigPaths::from_config_dir(root.path().join("device"));
        let confirmed = AppSettings::default();
        let barrier = std::sync::Barrier::new(8);
        let results = std::thread::scope(|scope| {
            let handles = (0..8)
                .map(|index| {
                    let host = &host;
                    let device = &device;
                    let confirmed = &confirmed;
                    let barrier = &barrier;
                    scope.spawn(move || {
                        let mut candidate = confirmed.clone();
                        candidate.general.ui_font_size = 18.0 + index as f32;
                        barrier.wait();
                        let result = save_scoped_settings_to_paths(
                            host, device, &candidate, confirmed, false,
                        );
                        (candidate.general.ui_font_size, result)
                    })
                })
                .collect::<Vec<_>>();
            handles
                .into_iter()
                .map(|handle| handle.join().unwrap())
                .collect::<Vec<_>>()
        });
        let accepted = results
            .iter()
            .filter(|(_, result)| result.is_ok())
            .collect::<Vec<_>>();
        assert_eq!(
            accepted.len(),
            1,
            "only one writer may commit against the original baseline"
        );
        assert_eq!(
            read_device_settings(&device.settings_file())
                .unwrap()
                .general
                .ui_font_size,
            accepted[0].0,
        );
        assert!(results.iter().all(|(_, result)| {
            result.is_ok() || matches!(result, Err(ScopedSettingsSaveError::DeviceConflict { .. }))
        }));
    }

    #[test]
    fn device_save_compares_the_validated_values_shown_to_the_user() {
        let root = tempfile::tempdir().unwrap();
        let paths = AppConfigPaths::from_config_dir(root.path());
        let device_file = root.path().join("device/settings.toml");
        fs::create_dir_all(device_file.parent().unwrap()).unwrap();
        fs::write(&device_file, "[general]\nui_font_size = 1000\n").unwrap();
        let confirmed = super::super::settings::load_settings(&paths)
            .unwrap()
            .settings;
        let mut candidate = confirmed.clone();
        candidate.editor.font_size = 20.0;
        save_scoped_settings(&paths, &candidate, &confirmed, false).unwrap();
        let loaded = super::super::settings::load_settings(&paths)
            .unwrap()
            .settings;
        assert_eq!(loaded.general.ui_font_size, confirmed.general.ui_font_size);
        assert_eq!(loaded.editor.font_size, 20.0);
    }

    #[test]
    fn unreadable_device_preferences_are_not_replaced_by_a_stale_draft() {
        let root = tempfile::tempdir().unwrap();
        let host = AppConfigPaths::from_config_dir(root.path().join("host"));
        let device = AppConfigPaths::from_config_dir(root.path().join("device"));
        fs::create_dir_all(device.config_dir()).unwrap();
        fs::write(
            device.settings_file(),
            "[general]\nui_font_size = \"broken\"\n",
        )
        .unwrap();
        let bytes = fs::read(device.settings_file()).unwrap();
        let confirmed = AppSettings::default();
        let mut candidate = confirmed.clone();
        candidate.editor.font_size = 20.0;
        assert!(matches!(
            save_scoped_settings_to_paths(&host, &device, &candidate, &confirmed, false),
            Err(ScopedSettingsSaveError::LoadDevice(_)),
        ));
        assert_eq!(fs::read(device.settings_file()).unwrap(), bytes);
    }

    #[test]
    fn observer_save_denies_host_changes_without_writing_any_target() {
        let root = tempfile::tempdir().unwrap();
        let host = AppConfigPaths::from_config_dir(root.path().join("host"));
        let device = AppConfigPaths::from_config_dir(root.path().join("device"));
        let confirmed = AppSettings::default();
        let mut candidate = confirmed.clone();
        candidate.terminal.shell = "zsh".into();

        let error = save_scoped_settings_to_paths(&host, &device, &candidate, &confirmed, false)
            .unwrap_err();

        assert!(matches!(
            error,
            ScopedSettingsSaveError::HostWriteDenied { .. }
        ));
        assert!(!device.settings_file().exists());
        assert!(!host.settings_file().exists());
    }

    #[test]
    fn mixed_observer_save_is_rejected_before_either_target_is_written() {
        let root = tempfile::tempdir().unwrap();
        let host = AppConfigPaths::from_config_dir(root.path().join("host"));
        let device = AppConfigPaths::from_config_dir(root.path().join("device"));
        let confirmed = AppSettings::default();
        let mut candidate = confirmed.clone();
        candidate.general.ui_font_size = 18.0;
        candidate.terminal.shell = "zsh".into();

        let error = save_scoped_settings_to_paths(&host, &device, &candidate, &confirmed, false)
            .unwrap_err();

        assert!(matches!(error, ScopedSettingsSaveError::MixedScopeChange));
        assert!(!device.settings_file().exists());
        assert!(!host.settings_file().exists());
    }

    #[test]
    fn device_preferences_survive_host_switch_without_leaking_host_fields() {
        let root = tempfile::tempdir().unwrap();
        let host_a = AppConfigPaths::from_config_dir(root.path().join("host-a"));
        let host_b = AppConfigPaths::from_config_dir(root.path().join("host-b"));
        let device = AppConfigPaths::from_config_dir(root.path().join("device"));
        let confirmed = AppSettings::default();

        let mut device_candidate = confirmed.clone();
        device_candidate.general.ui_font_size = 18.0;
        save_scoped_settings_to_paths(&host_a, &device, &device_candidate, &confirmed, true)
            .unwrap();

        let mut host_a_candidate = confirmed.clone();
        host_a_candidate.terminal.shell = "bash".into();
        save_scoped_settings_to_paths(&host_a, &device, &host_a_candidate, &confirmed, true)
            .unwrap();

        let mut host_b_effective = AppSettings::default();
        host_b_effective.terminal.shell = "zsh".into();
        let device_source = fs::read_to_string(device.settings_file()).unwrap();
        let device_settings = toml::from_str::<AppSettings>(&device_source).unwrap();
        merge_device_settings(&mut host_b_effective, &device_settings);

        assert_eq!(host_b_effective.general.ui_font_size, 18.0);
        assert_eq!(host_b_effective.terminal.shell, "zsh");
        assert!(
            !fs::read_to_string(device.settings_file())
                .unwrap()
                .contains("shell")
        );
        assert!(
            !fs::read_to_string(host_a.settings_file())
                .unwrap()
                .contains("ui_font_size")
        );
        assert!(!host_b.settings_file().exists());
    }

    #[test]
    fn legacy_migration_keeps_host_execution_fields_out_of_device_preferences() {
        let root = tempfile::tempdir().unwrap();
        let legacy = root.path().join("legacy");
        let device = root.path().join("device");
        fs::create_dir_all(&legacy).unwrap();
        fs::write(
            legacy.join("settings.toml"),
            "[general]\nui_font_size = 18.0\nnew_tab_commands = [\"host\"]\n\n[terminal]\nshell = \"zsh\"\nfont_size = 15.0\n",
        )
        .unwrap();
        fs::write(legacy.join("keybindings.toml"), "leader = \"space\"\n").unwrap();
        fs::write(legacy.join("bars.toml"), "[status]\nenabled = false\n").unwrap();
        let legacy_theme = legacy.join("themes").join("private.toml");
        fs::create_dir_all(legacy_theme.parent().unwrap()).unwrap();
        fs::write(&legacy_theme, "name = \"private\"\n").unwrap();

        migrate_legacy_device_preferences(&legacy, &device).unwrap();
        let migrated = fs::read_to_string(device.join("settings.toml")).unwrap();

        assert!(migrated.contains("ui_font_size = 18.0"));
        assert!(migrated.contains("font_size = 15.0"));
        assert!(!migrated.contains("new_tab_commands"));
        assert!(!migrated.contains("shell = \"zsh\""));
        assert_eq!(
            fs::read_to_string(device.join("keybindings.toml")).unwrap(),
            "leader = \"space\"\n"
        );
        assert_eq!(
            fs::read_to_string(device.join("bars.toml")).unwrap(),
            "[status]\nenabled = false\n"
        );
        assert_eq!(
            fs::read_to_string(device.join("themes").join("private.toml")).unwrap(),
            "name = \"private\"\n"
        );
        assert!(legacy.join("settings.toml").exists());
        assert!(legacy.join("keybindings.toml").exists());
        assert!(legacy.join("bars.toml").exists());
        assert!(legacy_theme.exists());
    }

    #[test]
    fn legacy_migration_normalizes_vim_and_does_not_recreate_deleted_device_preferences() {
        let root = tempfile::tempdir().unwrap();
        let legacy = root.path().join("legacy");
        let device = root.path().join("device");
        fs::create_dir_all(&legacy).unwrap();
        let source = "[general]\nworkspace_vim_navigation = true\n";
        fs::write(legacy.join("settings.toml"), source).unwrap();
        fs::write(legacy.join("keybindings.toml"), "leader = \"space\"\n").unwrap();
        migrate_legacy_device_preferences(&legacy, &device).unwrap();
        assert_eq!(
            read_device_settings(&device.join("settings.toml"))
                .unwrap()
                .vim
                .mode,
            super::super::settings::VimModeSetting::Global,
        );
        fs::remove_file(device.join("keybindings.toml")).unwrap();
        migrate_legacy_device_preferences(&legacy, &device).unwrap();
        assert!(!device.join("keybindings.toml").exists());
        assert_eq!(
            fs::read_to_string(legacy.join("settings.toml")).unwrap(),
            source
        );
    }

    #[test]
    fn stale_host_settings_cannot_overwrite_another_clients_confirmed_change() {
        let root = tempfile::tempdir().unwrap();
        let host = AppConfigPaths::from_config_dir(root.path().join("host"));
        let device = AppConfigPaths::from_config_dir(root.path().join("device"));
        let baseline = AppSettings::default();
        let mut first = baseline.clone();
        first.editor.tab_size = 8;
        save_scoped_settings_to_paths(&host, &device, &first, &baseline, true).unwrap();
        let saved = fs::read(host.settings_file()).unwrap();
        let mut stale = baseline.clone();
        stale.terminal.shell = "/bin/bash".into();
        assert!(matches!(
            save_scoped_settings_to_paths(&host, &device, &stale, &baseline, true),
            Err(ScopedSettingsSaveError::HostConflict { .. }),
        ));
        assert_eq!(fs::read(host.settings_file()).unwrap(), saved);
    }

    #[test]
    fn malformed_legacy_device_preferences_fail_binding_migration() {
        let root = tempfile::tempdir().unwrap();
        let legacy = root.path().join("legacy");
        let device = root.path().join("device");
        fs::create_dir_all(&legacy).unwrap();
        fs::write(legacy.join("settings.toml"), "[terminal\n").unwrap();

        let error = migrate_legacy_device_preferences(&legacy, &device).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(!device.join("settings.toml").exists());
        assert!(legacy.join("settings.toml").exists());
    }
}
