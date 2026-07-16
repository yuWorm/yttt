use std::path::{Path, PathBuf};

use tempfile::tempdir;
use yttt::config::{
    paths::AppConfigPaths,
    settings::{
        AUTO_SHELL, AppSettings, EditorAutosave, LanguageSetting, SettingsLoadWarning,
        ShellPlatform, WindowBackgroundEffect, detect_shell_candidates_with,
        language_setting_for_locale, load_or_create_settings, resolve_default_shell, save_settings,
    },
};
use yttt_terminal::{TerminalCursorShape, TerminalOsc52Policy};

#[test]
fn system_locale_maps_supported_chinese_variants() {
    for locale in ["zh-CN", "zh_Hans_CN.UTF-8", "zh-Hant-TW", "ZH_cn"] {
        assert_eq!(
            language_setting_for_locale(Some(locale)),
            LanguageSetting::Chinese,
            "{locale}"
        );
    }
}

#[test]
fn system_locale_defaults_other_or_missing_locales_to_english() {
    for locale in [Some("en-US"), Some("de_DE.UTF-8"), Some("ja-JP"), None] {
        assert_eq!(
            language_setting_for_locale(locale),
            LanguageSetting::English,
            "{locale:?}"
        );
    }
}

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
    assert_eq!(loaded.settings.general.ui_font_family, "");
    assert_eq!(loaded.settings.general.ui_font_size, 16.0);
    assert_eq!(loaded.settings.general.ui_line_height, 1.618_034);
    assert!(!loaded.settings.general.onboarding_completed);
    assert!(loaded.settings.general.performance_metrics_enabled);
    assert!(!loaded.settings.general.system_performance_metrics_enabled);
    assert!(!loaded.settings.general.restore_last_session);
    assert!(!loaded.settings.general.new_tab_command_picker_enabled);
    assert_eq!(
        loaded.settings.general.new_tab_commands,
        vec!["lazygit", "nvim", "codex"]
    );
    assert_eq!(
        loaded.settings.window.effect,
        WindowBackgroundEffect::Blurred
    );
    assert_eq!(loaded.settings.window.opacity, 0.72);
    assert_eq!(loaded.settings.theme.name, "one-dark-theme");
    assert_eq!(loaded.settings.theme.terminal, None);
    assert!(!loaded.settings.notifications.system);
    assert_eq!(loaded.settings.terminal.font_family, "");
    assert_eq!(loaded.settings.terminal.shell, AUTO_SHELL);
    assert!(loaded.settings.terminal.custom_shells.is_empty());
    assert_eq!(loaded.settings.terminal.font_size, 13.0);
    assert_eq!(loaded.settings.terminal.line_height, 1.15);
    assert_eq!(loaded.settings.terminal.padding, 6.0);
    assert_eq!(loaded.settings.terminal.scrollback, 10000);
    assert!(loaded.settings.terminal.show_scrollbar);
    assert_eq!(
        loaded.settings.terminal.cursor_shape,
        TerminalCursorShape::Block
    );
    assert!(!loaded.settings.terminal.cursor_blinking);
    assert_eq!(loaded.settings.terminal.cursor_blink_interval_ms, 750);
    assert_eq!(loaded.settings.terminal.cursor_blink_timeout_secs, 5);
    assert_eq!(loaded.settings.terminal.cursor_thickness, 0.15);
    assert!(loaded.settings.terminal.cursor_unfocused_hollow);
    assert!(!loaded.settings.terminal.hide_mouse_when_typing);
    assert!(!loaded.settings.terminal.copy_on_select);
    assert_eq!(
        loaded.settings.terminal.osc52_policy,
        TerminalOsc52Policy::CopyOnly
    );
    assert!(!loaded.settings.terminal.kitty_keyboard);
    assert_eq!(
        loaded.settings.terminal.semantic_escape_chars,
        ",│`|:\"' ()[]{}<>\t"
    );
    assert_eq!(loaded.settings.terminal.hint_alphabet, "jfkdls;ahgurieowpq");
    assert_eq!(loaded.settings.terminal.hints.len(), 1);
    assert!(loaded.settings.editor.auto_detect_language);
    assert_eq!(loaded.settings.editor.default_language, "plain_text");
    assert!(!loaded.settings.editor.lsp.enabled);
    assert_eq!(loaded.settings.editor.lsp.command, "");
    assert!(paths.settings_file().exists());
    assert!(loaded.warnings.is_empty());
}

#[test]
fn settings_default_language_is_system() {
    let settings = AppSettings::default();

    assert_eq!(settings.general.language, LanguageSetting::System);
}

#[test]
fn window_background_settings_load_without_touching_other_defaults() {
    let dir = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(dir.path());
    std::fs::write(
        paths.settings_file(),
        r#"
[general]
onboarding_completed = true

[window]
effect = "none"
opacity = 0.42
"#,
    )
    .unwrap();

    let loaded = load_or_create_settings(&paths).unwrap();

    assert_eq!(loaded.settings.window.effect, WindowBackgroundEffect::None);
    assert_eq!(loaded.settings.window.opacity, 0.42);
    assert!(loaded.settings.general.onboarding_completed);
    assert_eq!(loaded.settings.theme.name, "one-dark-theme");
    assert!(loaded.warnings.is_empty());
}

#[test]
fn editor_and_project_panel_defaults_match_the_design() {
    let settings = AppSettings::default();

    assert_eq!(settings.editor.font_family, "");
    assert_eq!(settings.editor.font_size, 14.0);
    assert_eq!(settings.editor.line_height, 1.4);
    assert_eq!(settings.editor.tab_size, 4);
    assert!(!settings.editor.soft_wrap);
    assert!(settings.editor.line_numbers);
    assert_eq!(settings.editor.autosave, EditorAutosave::Off);
    assert_eq!(settings.editor.autosave_delay_ms, 1000);
    assert!(settings.project_panel.default_open);
    assert!(!settings.project_panel.show_hidden);
    assert_eq!(settings.project_panel.width, 280.0);
    assert_eq!(settings.project_panel.project_sidebar_width, 216.0);
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
fn terminal_settings_reject_invalid_protocol_values() {
    let dir = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(dir.path());
    std::fs::create_dir_all(paths.config_dir()).unwrap();
    std::fs::write(
        paths.settings_file(),
        r#"
[terminal]
cursor_blink_interval_ms = 1
cursor_blink_timeout_secs = 256
cursor_thickness = 2.0
hint_alphabet = "界"
"#,
    )
    .unwrap();

    let loaded = load_or_create_settings(&paths).unwrap();
    let terminal = loaded.settings.terminal;
    assert_eq!(terminal.cursor_blink_interval_ms, 750);
    assert_eq!(terminal.cursor_blink_timeout_secs, 5);
    assert_eq!(terminal.cursor_thickness, 0.15);
    assert_eq!(terminal.hint_alphabet, "jfkdls;ahgurieowpq");
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
    settings.terminal.custom_shells =
        vec!["/opt/homebrew/bin/fish".to_string(), "/bin/zsh".to_string()];
    settings.terminal.font_size = 15.0;

    save_settings(&paths, &settings).unwrap();
    let loaded = load_or_create_settings(&paths).unwrap();

    assert!(loaded.settings.notifications.system);
    assert_eq!(loaded.settings.terminal.shell, "/bin/zsh");
    assert_eq!(
        loaded.settings.terminal.custom_shells,
        vec!["/opt/homebrew/bin/fish", "/bin/zsh"]
    );
    assert_eq!(loaded.settings.terminal.font_size, 15.0);
}

#[test]
fn settings_persist_language_and_terminal_scrollbar() {
    let dir = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(dir.path());
    let mut settings = AppSettings::default();
    settings.general.language = LanguageSetting::Chinese;
    settings.general.ui_font_family = "  Menlo  ".to_string();
    settings.general.ui_font_size = 20.0;
    settings.general.ui_line_height = 1.75;
    settings.general.onboarding_completed = true;
    settings.general.performance_metrics_enabled = false;
    settings.general.system_performance_metrics_enabled = true;
    settings.general.restore_last_session = true;
    settings.general.new_tab_command_picker_enabled = true;
    settings.general.new_tab_commands = vec!["nvim .".to_string(), "codex --resume".to_string()];
    settings.terminal.show_scrollbar = false;

    save_settings(&paths, &settings).unwrap();
    let loaded = load_or_create_settings(&paths).unwrap();

    assert_eq!(loaded.settings.general.language, LanguageSetting::Chinese);
    assert_eq!(loaded.settings.general.ui_font_family, "Menlo");
    assert_eq!(loaded.settings.general.ui_font_size, 20.0);
    assert_eq!(loaded.settings.general.ui_line_height, 1.75);
    assert!(loaded.settings.general.onboarding_completed);
    assert!(!loaded.settings.general.performance_metrics_enabled);
    assert!(loaded.settings.general.system_performance_metrics_enabled);
    assert!(loaded.settings.general.restore_last_session);
    assert!(loaded.settings.general.new_tab_command_picker_enabled);
    assert_eq!(
        loaded.settings.general.new_tab_commands,
        vec!["nvim .", "codex --resume"]
    );
    assert!(!loaded.settings.terminal.show_scrollbar);
}

#[test]
fn settings_persist_editor_language_and_lsp_choices() {
    let dir = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(dir.path());
    let mut settings = AppSettings::default();
    settings.editor.auto_detect_language = false;
    settings.editor.default_language = "toml".to_string();
    settings.editor.lsp.enabled = true;
    settings.editor.lsp.command = "taplo lsp stdio".to_string();

    save_settings(&paths, &settings).unwrap();
    let loaded = load_or_create_settings(&paths).unwrap();

    assert!(!loaded.settings.editor.auto_detect_language);
    assert_eq!(loaded.settings.editor.default_language, "toml");
    assert!(loaded.settings.editor.lsp.enabled);
    assert_eq!(loaded.settings.editor.lsp.command, "taplo lsp stdio");
}

#[test]
fn settings_persist_editor_and_project_panel_choices() {
    let dir = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(dir.path());

    for autosave in [
        EditorAutosave::Off,
        EditorAutosave::OnFocusChange,
        EditorAutosave::AfterDelay,
    ] {
        let mut settings = AppSettings::default();
        settings.editor.font_family = "JetBrains Mono".to_string();
        settings.editor.font_size = 16.0;
        settings.editor.line_height = 1.6;
        settings.editor.tab_size = 2;
        settings.editor.soft_wrap = true;
        settings.editor.line_numbers = false;
        settings.editor.autosave = autosave;
        settings.editor.autosave_delay_ms = 750;
        settings.project_panel.default_open = false;
        settings.project_panel.show_hidden = true;
        settings.project_panel.width = 320.0;
        settings.project_panel.project_sidebar_width = 240.0;

        save_settings(&paths, &settings).unwrap();
        let loaded = load_or_create_settings(&paths).unwrap();

        assert_eq!(loaded.settings, settings);
        assert!(loaded.warnings.is_empty());
    }
}

#[test]
fn invalid_general_window_editor_and_project_panel_values_are_normalized() {
    let dir = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(dir.path());
    std::fs::create_dir_all(paths.config_dir()).unwrap();
    std::fs::write(
        paths.settings_file(),
        r#"
[general]
language = "zh-CN"
ui_font_size = nan
ui_line_height = 0.5

[window]
effect = "glass"
opacity = 2.0

[terminal]
font_size = 15.0

[editor]
font_family = "  JetBrains Mono  "
font_size = nan
line_height = 0.5
tab_size = 17
autosave = "sometimes"
autosave_delay_ms = 0

[project_panel]
width = 10000.0
project_sidebar_width = 1.0
"#,
    )
    .unwrap();

    let loaded = load_or_create_settings(&paths).unwrap();

    assert_eq!(loaded.settings.general.language, LanguageSetting::Chinese);
    assert_eq!(loaded.settings.general.ui_font_size, 16.0);
    assert_eq!(loaded.settings.general.ui_line_height, 1.618_034);
    assert_eq!(
        loaded.settings.window.effect,
        WindowBackgroundEffect::Blurred
    );
    assert_eq!(loaded.settings.window.opacity, 0.72);
    assert_eq!(loaded.settings.terminal.font_size, 15.0);
    assert_eq!(loaded.settings.editor.font_family, "JetBrains Mono");
    assert_eq!(loaded.settings.editor.font_size, 14.0);
    assert_eq!(loaded.settings.editor.line_height, 1.4);
    assert_eq!(loaded.settings.editor.tab_size, 4);
    assert_eq!(loaded.settings.editor.autosave, EditorAutosave::Off);
    assert_eq!(loaded.settings.editor.autosave_delay_ms, 1000);
    assert_eq!(loaded.settings.project_panel.width, 520.0);
    assert_eq!(loaded.settings.project_panel.project_sidebar_width, 160.0);

    for warning in [
        SettingsLoadWarning::InvalidGeneralValue {
            field: "ui_font_size",
        },
        SettingsLoadWarning::InvalidGeneralValue {
            field: "ui_line_height",
        },
        SettingsLoadWarning::InvalidWindowValue { field: "effect" },
        SettingsLoadWarning::InvalidWindowValue { field: "opacity" },
        SettingsLoadWarning::InvalidEditorValue { field: "autosave" },
        SettingsLoadWarning::InvalidEditorValue { field: "font_size" },
        SettingsLoadWarning::InvalidEditorValue {
            field: "line_height",
        },
        SettingsLoadWarning::InvalidEditorValue { field: "tab_size" },
        SettingsLoadWarning::InvalidEditorValue {
            field: "autosave_delay_ms",
        },
        SettingsLoadWarning::InvalidProjectPanelValue { field: "width" },
        SettingsLoadWarning::InvalidProjectPanelValue {
            field: "project_sidebar_width",
        },
    ] {
        assert!(
            loaded.warnings.contains(&warning),
            "missing warning: {warning:?}"
        );
    }
}

#[test]
fn settings_allow_lsp_enabled_without_command_for_reserved_slot() {
    let dir = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(dir.path());
    let mut settings = AppSettings::default();
    settings.editor.lsp.enabled = true;

    save_settings(&paths, &settings).unwrap();
    let loaded = load_or_create_settings(&paths).unwrap();

    assert!(loaded.settings.editor.lsp.enabled);
    assert_eq!(loaded.settings.editor.lsp.command, "");
    assert!(loaded.warnings.is_empty());
}

#[test]
fn macos_shell_detection_prioritizes_shell_env_then_system_shells() {
    let candidates = detect_shell_candidates_with(
        ShellPlatform::MacOs,
        Some("/opt/homebrew/bin/fish"),
        None,
        &[],
        |path| {
            matches!(
                path.to_str(),
                Some("/opt/homebrew/bin/fish" | "/bin/zsh" | "/bin/bash" | "/bin/sh")
            )
        },
    );

    assert_eq!(
        candidates,
        vec![
            "/opt/homebrew/bin/fish".to_string(),
            "/bin/zsh".to_string(),
            "/bin/bash".to_string(),
            "/bin/sh".to_string(),
        ]
    );
}

#[test]
fn linux_shell_detection_skips_missing_shell_env_value() {
    let candidates = detect_shell_candidates_with(
        ShellPlatform::Linux,
        Some("/tmp/not-a-shell"),
        None,
        &[],
        |path| matches!(path.to_str(), Some("/bin/bash" | "/bin/sh")),
    );

    assert_eq!(
        candidates,
        vec!["/bin/bash".to_string(), "/bin/sh".to_string()]
    );
}

#[test]
fn windows_shell_detection_uses_comspec_and_path_candidates() {
    let path_entries = vec![
        PathBuf::from("C:/Program Files/PowerShell/7"),
        PathBuf::from("C:/Windows/System32"),
    ];
    let candidates = detect_shell_candidates_with(
        ShellPlatform::Windows,
        None,
        Some("C:/Windows/System32/cmd.exe"),
        &path_entries,
        |path| {
            matches!(
                path.to_str(),
                Some("C:/Windows/System32/cmd.exe" | "C:/Program Files/PowerShell/7/pwsh.exe")
            )
        },
    );

    assert_eq!(
        candidates,
        vec![
            "C:/Windows/System32/cmd.exe".to_string(),
            "C:/Program Files/PowerShell/7/pwsh.exe".to_string(),
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
