use std::path::Path;

use tempfile::tempdir;
use yttt::config::{
    paths::AppConfigPaths,
    settings::{
        AUTO_SHELL, AppSettings, LanguageSetting, SettingsLoadWarning,
        detect_shell_candidates_with, load_or_create_settings, resolve_default_shell,
        save_settings,
    },
};

#[test]
fn app_config_paths_expose_settings_and_theme_dir() {
    let paths = AppConfigPaths::from_config_dir("/tmp/yttt-config");

    assert_eq!(
        paths.settings_file(),
        Path::new("/tmp/yttt-config/settings.toml")
    );
    assert_eq!(paths.themes_dir(), Path::new("/tmp/yttt-config/themes"));
}

#[test]
fn missing_settings_file_writes_defaults() {
    let dir = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(dir.path());

    let loaded = load_or_create_settings(&paths).unwrap();

    assert_eq!(loaded.settings.general.language, LanguageSetting::System);
    assert_eq!(loaded.settings.theme.name, "yttt-dark");
    assert_eq!(loaded.settings.theme.terminal, None);
    assert!(!loaded.settings.notifications.system);
    assert_eq!(loaded.settings.terminal.font_family, "");
    assert_eq!(loaded.settings.terminal.shell, AUTO_SHELL);
    assert_eq!(loaded.settings.terminal.font_size, 13.0);
    assert_eq!(loaded.settings.terminal.line_height, 1.15);
    assert_eq!(loaded.settings.terminal.padding, 6.0);
    assert_eq!(loaded.settings.terminal.scrollback, 10000);
    assert!(loaded.settings.terminal.close_on_exit);
    assert!(loaded.settings.terminal.show_scrollbar);
    assert!(paths.settings_file().exists());
    assert!(loaded.warnings.is_empty());
}

#[test]
fn settings_default_language_is_system() {
    let settings = AppSettings::default();

    assert_eq!(settings.general.language, LanguageSetting::System);
}

#[test]
fn terminal_settings_reject_invalid_numeric_values() {
    let dir = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(dir.path());
    std::fs::create_dir_all(paths.config_dir()).unwrap();
    std::fs::write(
        paths.settings_file(),
        r#"
[terminal]
font_size = 0
line_height = -1
padding = -2
scrollback = 0
"#,
    )
    .unwrap();

    let loaded = load_or_create_settings(&paths).unwrap();

    assert_eq!(loaded.settings.terminal.font_size, 13.0);
    assert_eq!(loaded.settings.terminal.line_height, 1.15);
    assert_eq!(loaded.settings.terminal.padding, 6.0);
    assert_eq!(loaded.settings.terminal.scrollback, 10000);
    assert_eq!(loaded.warnings.len(), 4);
}

#[test]
fn invalid_language_falls_back_to_system() {
    let dir = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(dir.path());
    std::fs::create_dir_all(paths.config_dir()).unwrap();
    std::fs::write(
        paths.settings_file(),
        r#"
[general]
language = "xx"
"#,
    )
    .unwrap();

    let loaded = load_or_create_settings(&paths).unwrap();

    assert_eq!(loaded.settings.general.language, LanguageSetting::System);
    assert_eq!(
        loaded.warnings,
        vec![SettingsLoadWarning::InvalidGeneralValue { field: "language" }]
    );
}

#[test]
fn settings_persist_notification_and_terminal_shell_choices() {
    let dir = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(dir.path());
    let mut settings = AppSettings::default();
    settings.notifications.system = true;
    settings.terminal.shell = "/bin/zsh".to_string();
    settings.terminal.font_size = 15.0;

    save_settings(&paths, &settings).unwrap();
    let loaded = load_or_create_settings(&paths).unwrap();

    assert!(loaded.settings.notifications.system);
    assert_eq!(loaded.settings.terminal.shell, "/bin/zsh");
    assert_eq!(loaded.settings.terminal.font_size, 15.0);
}

#[test]
fn settings_persist_language_and_close_on_exit() {
    let dir = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(dir.path());
    let mut settings = AppSettings::default();
    settings.general.language = LanguageSetting::Chinese;
    settings.terminal.close_on_exit = false;
    settings.terminal.show_scrollbar = false;

    save_settings(&paths, &settings).unwrap();
    let loaded = load_or_create_settings(&paths).unwrap();

    assert_eq!(loaded.settings.general.language, LanguageSetting::Chinese);
    assert!(!loaded.settings.terminal.close_on_exit);
    assert!(!loaded.settings.terminal.show_scrollbar);
}

#[test]
fn shell_detection_prioritizes_shell_env_then_common_system_shells() {
    let candidates = detect_shell_candidates_with(Some("/opt/homebrew/bin/fish"), |path| {
        matches!(path, "/opt/homebrew/bin/fish" | "/bin/zsh" | "/bin/bash")
    });

    assert_eq!(
        candidates,
        vec![
            "/opt/homebrew/bin/fish".to_string(),
            "/bin/zsh".to_string(),
            "/bin/bash".to_string(),
            "sh".to_string()
        ]
    );
}

#[test]
fn shell_detection_skips_missing_shell_env_value() {
    let candidates = detect_shell_candidates_with(Some("/tmp/not-a-shell"), |path| {
        matches!(path, "/bin/zsh" | "/bin/bash")
    });

    assert_eq!(
        candidates,
        vec![
            "/bin/zsh".to_string(),
            "/bin/bash".to_string(),
            "sh".to_string()
        ]
    );
}

#[test]
fn resolve_default_shell_uses_auto_or_manual_choice() {
    let candidates = vec!["/bin/zsh".to_string(), "/bin/bash".to_string()];

    assert_eq!(resolve_default_shell(AUTO_SHELL, &candidates), "/bin/zsh");
    assert_eq!(resolve_default_shell("", &candidates), "/bin/zsh");
    assert_eq!(
        resolve_default_shell("/usr/local/bin/fish", &candidates),
        "/usr/local/bin/fish"
    );
    assert_eq!(resolve_default_shell(AUTO_SHELL, &[]), "sh");
}
