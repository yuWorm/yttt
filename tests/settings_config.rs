use std::path::{Path, PathBuf};

use tempfile::tempdir;
use yttt::config::{
    bars::{
        BarModuleSettings, BarsLoadWarning, ShellBarModule, ShellBarsSettings, bar_icon_path,
        format_bar_template, load_bars, parse_bar_template, save_bars,
    },
    default_layout::BuiltinAgent,
    paths::AppConfigPaths,
    scope::save_scoped_settings,
    settings::{
        AUTO_SHELL, AppSettings, EditorAutosave, LanguageSetting, SettingsLoadWarning,
        ShellPlatform, VimModeSetting, WindowBackgroundEffect, detect_shell_candidates_with,
        language_setting_for_locale, load_settings, resolve_default_shell, save_settings,
    },
};
use yttt::ui::theme::UiStyleId;

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
    assert_eq!(paths.bars_file(), Path::new("/tmp/yttt-config/bars.toml"));
    assert_eq!(paths.themes_dir(), Path::new("/tmp/yttt-config/themes"));
}

#[test]
fn missing_settings_file_loads_defaults_without_creating_files() {
    let dir = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(dir.path());

    let loaded = load_settings(&paths).unwrap();

    assert_eq!(loaded.settings, AppSettings::default());
    assert!(loaded.warnings.is_empty());
    assert!(!paths.settings_file().exists());
    assert!(!paths.bars_file().exists());
    assert!(!paths.config_dir().join("device/settings.toml").exists());
}

#[test]
fn host_settings_save_persists_agent_fields_only() {
    let dir = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(dir.path());
    let mut settings = AppSettings::default();
    settings.agent.primary = Some(BuiltinAgent::OhMyPi);
    settings.agent.sessions_enabled = false;
    settings.agent.additional_session_agents = vec![BuiltinAgent::Claude, BuiltinAgent::Pi];
    settings.notifications.system = true;

    save_settings(&paths, &settings).unwrap();
    let loaded = load_settings(&paths).unwrap();

    assert_eq!(loaded.settings.agent.primary, Some(BuiltinAgent::OhMyPi));
    assert!(!loaded.settings.agent.sessions_enabled);
    assert_eq!(
        loaded.settings.agent.additional_session_agents,
        vec![BuiltinAgent::Claude, BuiltinAgent::Pi]
    );
    assert!(!loaded.settings.notifications.system);
    let source: toml::Value =
        toml::from_str(&std::fs::read_to_string(paths.settings_file()).unwrap()).unwrap();
    let source = source.as_table().unwrap();
    assert!(source.contains_key("agent"));
    assert!(!source.contains_key("notifications"));
}

#[test]
fn window_background_settings_load_without_touching_other_defaults() {
    let dir = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(dir.path());
    std::fs::create_dir_all(paths.config_dir()).unwrap();
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

    let loaded = load_settings(&paths).unwrap();

    assert_eq!(loaded.settings.window.effect, WindowBackgroundEffect::None);
    assert_eq!(loaded.settings.window.opacity, 0.42);
    assert!(loaded.settings.general.onboarding_completed);
    assert_eq!(loaded.settings.theme.name, "one-dark-theme");
    assert!(loaded.warnings.is_empty());
}

#[test]
fn opacity_alone_keeps_windows_opaque_but_explicit_blur_is_preserved() {
    let dir = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(dir.path());
    let source = "[window]\nopacity = 0.42\n";
    std::fs::write(paths.settings_file(), source).unwrap();
    let loaded = load_settings(&paths).unwrap();
    assert_eq!(loaded.settings.window.resolved_opacity(), 1.0);
    assert_eq!(
        std::fs::read_to_string(paths.settings_file()).unwrap(),
        source
    );

    let explicit = format!("{source}effect = \"blurred\"\n");
    std::fs::write(paths.settings_file(), &explicit).unwrap();
    let loaded = load_settings(&paths).unwrap();
    assert_eq!(
        loaded.settings.window.effect,
        WindowBackgroundEffect::Blurred
    );
    assert_eq!(loaded.settings.window.resolved_opacity(), 0.42);
    assert_eq!(
        std::fs::read_to_string(paths.settings_file()).unwrap(),
        explicit
    );
}

#[test]
fn legacy_performance_metric_keys_load_and_are_omitted_from_device_saves() {
    let dir = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(dir.path());
    let device_settings_file = paths.config_dir().join("device/settings.toml");
    std::fs::create_dir_all(device_settings_file.parent().unwrap()).unwrap();
    std::fs::write(
        &device_settings_file,
        r#"
[general]
onboarding_completed = true
performance_metrics_enabled = false
system_performance_metrics_enabled = true
"#,
    )
    .unwrap();

    let confirmed = load_settings(&paths).unwrap();

    assert!(confirmed.settings.general.onboarding_completed);
    assert!(confirmed.warnings.is_empty());

    let mut candidate = confirmed.settings.clone();
    candidate.general.language = LanguageSetting::Chinese;
    save_scoped_settings(&paths, &candidate, &confirmed.settings, false).unwrap();

    let source: toml::Value =
        toml::from_str(&std::fs::read_to_string(device_settings_file).unwrap()).unwrap();
    let general = source
        .get("general")
        .and_then(toml::Value::as_table)
        .expect("saved device settings include general settings");
    assert!(!general.contains_key("performance_metrics_enabled"));
    assert!(!general.contains_key("system_performance_metrics_enabled"));
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

    let loaded = load_settings(&paths).unwrap();

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

    let loaded = load_settings(&paths).unwrap();
    let terminal = loaded.settings.terminal;
    assert_eq!(terminal.cursor_blink_interval_ms, 750);
    assert_eq!(terminal.cursor_blink_timeout_secs, 5);
    assert_eq!(terminal.cursor_thickness, 0.15);
    assert_eq!(terminal.hint_alphabet, "jfkdls;ahgurieowpq");
    assert_eq!(loaded.warnings.len(), 4);
}

#[test]
fn terminal_settings_reject_non_portable_environment_names() {
    let dir = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(dir.path());
    std::fs::create_dir_all(paths.config_dir()).unwrap();
    std::fs::write(
        paths.settings_file(),
        r#"
[terminal.environment]
VALID_NAME = "kept"
"9INVALID" = "removed"
"BAD-NAME" = "removed"
"ALSO.INVALID" = "removed"
"#,
    )
    .unwrap();

    let loaded = load_settings(&paths).unwrap();

    assert_eq!(
        loaded.settings.terminal.environment.get("VALID_NAME"),
        Some(&"kept".to_string())
    );
    assert_eq!(loaded.settings.terminal.environment.len(), 1);
    assert_eq!(
        loaded.warnings,
        vec![SettingsLoadWarning::InvalidTerminalValue {
            field: "environment"
        }]
    );
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

    let loaded = load_settings(&paths).unwrap();

    assert_eq!(loaded.settings.general.language, LanguageSetting::System);
    assert_eq!(
        loaded.warnings,
        vec![SettingsLoadWarning::InvalidGeneralValue { field: "language" }]
    );
}

#[test]
fn host_settings_save_persists_shared_terminal_choices_only() {
    let dir = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(dir.path());
    let mut settings = AppSettings::default();
    settings.notifications.system = true;
    settings.terminal.shell = "/bin/zsh".to_string();
    settings.terminal.custom_shells =
        vec!["/opt/homebrew/bin/fish".to_string(), "/bin/zsh".to_string()];
    settings.terminal.font_size = 15.0;
    settings
        .terminal
        .environment
        .insert("RUST_LOG".to_string(), "yttt=debug".to_string());

    save_settings(&paths, &settings).unwrap();
    let loaded = load_settings(&paths).unwrap();

    assert!(!loaded.settings.notifications.system);
    assert_eq!(loaded.settings.terminal.shell, "/bin/zsh");
    assert_eq!(
        loaded.settings.terminal.custom_shells,
        vec!["/opt/homebrew/bin/fish", "/bin/zsh"]
    );
    assert_eq!(loaded.settings.terminal.font_size, 13.0);
    assert_eq!(
        loaded.settings.terminal.environment.get("RUST_LOG"),
        Some(&"yttt=debug".to_string())
    );
    let source: toml::Value =
        toml::from_str(&std::fs::read_to_string(paths.settings_file()).unwrap()).unwrap();
    let source = source.as_table().unwrap();
    assert!(!source.contains_key("notifications"));
    assert!(source["terminal"].get("font_size").is_none());
}

#[test]
fn device_settings_persist_language_and_terminal_scrollbar() {
    let dir = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(dir.path());
    let confirmed = AppSettings::default();
    let mut settings = confirmed.clone();
    settings.general.language = LanguageSetting::Chinese;
    settings.general.ui_font_family = "  Menlo  ".to_string();
    settings.general.ui_font_size = 20.0;
    settings.general.ui_line_height = 1.75;
    settings.general.onboarding_completed = true;
    settings.general.restore_last_session = true;
    settings.general.new_tab_command_picker_enabled = true;
    settings.notifications.system = true;
    settings.terminal.show_scrollbar = false;

    save_scoped_settings(&paths, &settings, &confirmed, false).unwrap();
    assert!(!paths.settings_file().exists());
    assert!(paths.config_dir().join("device/settings.toml").exists());
    let loaded = load_settings(&paths).unwrap();

    assert_eq!(loaded.settings.general.language, LanguageSetting::Chinese);
    assert_eq!(loaded.settings.general.ui_font_family, "Menlo");
    assert_eq!(loaded.settings.general.ui_font_size, 20.0);
    assert_eq!(loaded.settings.general.ui_line_height, 1.75);
    assert!(loaded.settings.general.onboarding_completed);
    assert!(loaded.settings.general.restore_last_session);
    assert!(loaded.settings.general.new_tab_command_picker_enabled);
    assert!(loaded.settings.notifications.system);
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
    let loaded = load_settings(&paths).unwrap();

    assert!(!loaded.settings.editor.auto_detect_language);
    assert_eq!(loaded.settings.editor.default_language, "toml");
    assert!(loaded.settings.editor.lsp.enabled);
    assert_eq!(loaded.settings.editor.lsp.command, "taplo lsp stdio");
}

#[test]
fn settings_persist_selected_ui_style() {
    let dir = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(dir.path());
    let mut settings = AppSettings::default();
    settings.theme.ui_style = UiStyleId::Rounded;

    save_scoped_settings(&paths, &settings, &AppSettings::default(), false).unwrap();
    let loaded = load_settings(&paths).unwrap();

    assert_eq!(loaded.settings.theme.ui_style, UiStyleId::Rounded);
    assert!(loaded.warnings.is_empty());
}

#[test]
fn device_settings_persist_editor_and_project_panel_choices() {
    let dir = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(dir.path());

    for autosave in [
        EditorAutosave::Off,
        EditorAutosave::OnFocusChange,
        EditorAutosave::AfterDelay,
    ] {
        let confirmed = load_settings(&paths).unwrap().settings;
        let mut settings = confirmed.clone();
        settings.editor.font_family = "JetBrains Mono".to_string();
        settings.editor.font_size = 16.0;
        settings.editor.line_height = 1.6;
        settings.editor.soft_wrap = true;
        settings.editor.line_numbers = false;
        settings.editor.autosave = autosave;
        settings.editor.autosave_delay_ms = 750;
        settings.project_panel.default_open = false;
        settings.project_panel.show_hidden = true;
        settings.project_panel.width = 320.0;
        settings.project_panel.project_sidebar_width = 240.0;

        save_scoped_settings(&paths, &settings, &confirmed, false).unwrap();
        let loaded = load_settings(&paths).unwrap();

        assert_eq!(loaded.settings.editor.font_family, "JetBrains Mono");
        assert_eq!(loaded.settings.editor.font_size, 16.0);
        assert_eq!(loaded.settings.editor.line_height, 1.6);
        assert!(loaded.settings.editor.soft_wrap);
        assert!(!loaded.settings.editor.line_numbers);
        assert_eq!(loaded.settings.editor.autosave, autosave);
        assert_eq!(loaded.settings.editor.autosave_delay_ms, 750);
        assert!(!loaded.settings.project_panel.default_open);
        assert!(loaded.settings.project_panel.show_hidden);
        assert_eq!(loaded.settings.project_panel.width, 320.0);
        assert_eq!(loaded.settings.project_panel.project_sidebar_width, 240.0);
        assert!(loaded.warnings.is_empty());
    }
}

#[test]
fn vim_mode_persists_as_device_preference() {
    let dir = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(dir.path());

    let mut confirmed = AppSettings::default();
    for mode in [
        VimModeSetting::Global,
        VimModeSetting::Editor,
        VimModeSetting::Disabled,
    ] {
        let mut settings = confirmed.clone();
        settings.vim.mode = mode;
        save_scoped_settings(&paths, &settings, &confirmed, false).unwrap();

        let loaded = load_settings(&paths).unwrap();
        assert_eq!(loaded.settings.vim.mode, mode);
        assert!(loaded.warnings.is_empty());
        confirmed = loaded.settings;
    }
}

#[test]
fn legacy_vim_toggles_load_as_one_mode_without_rewriting_source() {
    let dir = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(dir.path());
    let legacy_source = r#"
[general]
workspace_vim_navigation = false
settings_vim_navigation = false

[editor]
vim_mode = true

[terminal]
start_in_vim_mode = false
"#;
    std::fs::create_dir_all(paths.config_dir()).unwrap();
    std::fs::write(paths.settings_file(), legacy_source).unwrap();

    let loaded = load_settings(&paths).unwrap();
    assert_eq!(loaded.settings.vim.mode, VimModeSetting::Editor);
    assert_eq!(
        std::fs::read_to_string(paths.settings_file()).unwrap(),
        legacy_source
    );
}

#[test]
fn legacy_non_editor_vim_toggle_loads_as_global_without_rewriting_source() {
    let dir = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(dir.path());
    let legacy_source = "[terminal]\nstart_in_vim_mode = true\n";
    std::fs::create_dir_all(paths.config_dir()).unwrap();
    std::fs::write(paths.settings_file(), legacy_source).unwrap();

    let loaded = load_settings(&paths).unwrap();
    assert_eq!(loaded.settings.vim.mode, VimModeSetting::Global);
    assert_eq!(
        std::fs::read_to_string(paths.settings_file()).unwrap(),
        legacy_source
    );
}

#[test]
fn invalid_general_window_theme_editor_and_project_panel_values_are_normalized() {
    let dir = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(dir.path());
    std::fs::create_dir_all(paths.config_dir()).unwrap();
    let invalid_source = r#"
[general]
language = "zh-CN"
ui_font_size = nan
ui_line_height = 0.5

[vim]
mode = "all"

[window]
effect = "glass"
opacity = 2.0

[theme]
ui_style = "pillowy"

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
"#;
    std::fs::write(paths.settings_file(), invalid_source).unwrap();

    let loaded = load_settings(&paths).unwrap();

    assert_eq!(loaded.settings.general.language, LanguageSetting::Chinese);
    assert_eq!(loaded.settings.general.ui_font_size, 16.0);
    assert_eq!(loaded.settings.general.ui_line_height, 1.618_034);
    assert_eq!(loaded.settings.window.effect, WindowBackgroundEffect::None);
    assert_eq!(loaded.settings.window.opacity, 0.72);
    assert_eq!(loaded.settings.theme.ui_style, UiStyleId::Zed);
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
        SettingsLoadWarning::InvalidThemeValue { field: "ui_style" },
        SettingsLoadWarning::InvalidVimValue { field: "mode" },
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
    assert_eq!(
        std::fs::read_to_string(paths.settings_file()).unwrap(),
        invalid_source
    );
}

#[test]
fn settings_allow_lsp_enabled_without_command_for_reserved_slot() {
    let dir = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(dir.path());
    let mut settings = AppSettings::default();
    settings.editor.lsp.enabled = true;

    save_settings(&paths, &settings).unwrap();
    let loaded = load_settings(&paths).unwrap();

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

#[test]
fn shell_bar_layout_and_module_options_round_trip_in_standalone_file() {
    let dir = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(dir.path());
    let mut bars = ShellBarsSettings::default();
    bars.window.layout.left = vec![ShellBarModule::ActiveItem];
    bars.window.layout.center = vec![ShellBarModule::Surface];
    bars.window.layout.right = vec![ShellBarModule::Settings];
    bars.status.enabled = false;
    bars.status.layout.modules.insert(
        "active-item".to_string(),
        BarModuleSettings {
            max_width: Some(280.0),
            hide_when_empty: false,
        },
    );

    let confirmed = AppSettings::default();
    let mut candidate = confirmed.clone();
    candidate.bars = bars.clone();
    save_scoped_settings(&paths, &candidate, &confirmed, false).unwrap();
    let loaded = load_bars(&paths).unwrap();

    assert_eq!(loaded.settings, bars);
    assert!(loaded.warnings.is_empty());
}

#[test]
fn bar_template_round_trips_unicode_escapes_icons_and_duplicates() {
    let template =
        r" [Settings] [Space: 1] [sPaCe:5] [text: 你好: \[x\] \\ ] [icon:CPU] [|] [settings] ";

    let modules = parse_bar_template(template).unwrap();

    assert_eq!(
        modules,
        vec![
            ShellBarModule::Settings,
            ShellBarModule::Space(1),
            ShellBarModule::Space(5),
            ShellBarModule::Text(" 你好: [x] \\ ".to_string()),
            ShellBarModule::Icon("cpu".to_string()),
            ShellBarModule::Separator,
            ShellBarModule::Settings,
        ]
    );
    assert_eq!(
        format_bar_template(&modules),
        r"[settings] [Space: 1] [Space: 5] [text: 你好: \[x\] \\ ] [icon:cpu] [|] [settings]"
    );
    assert_eq!(
        bar_icon_path("MeMoRy-StIcK"),
        Some("icons/memory-stick.svg")
    );
}

#[test]
fn bar_template_rejects_invalid_tokens() {
    for template in [
        "[Space]",
        "[Space*5]",
        "[Space:]",
        "[Space: 0]",
        "[Space: 257]",
        "[Space: -1]",
        "[Space: 1.5]",
        "[icon:not-bundled]",
        "[not-a-module]",
        r"[text:\q]",
    ] {
        assert!(parse_bar_template(template).is_err(), "{template}");
    }
    assert_eq!(parse_bar_template(" \n\t ").unwrap(), Vec::new());
    assert_eq!(
        parse_bar_template("[Space: 256]").unwrap(),
        vec![ShellBarModule::Space(256)]
    );
}

#[test]
fn legacy_array_bars_preserve_window_identity_and_save_templates() {
    let dir = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(dir.path());
    std::fs::create_dir_all(paths.config_dir()).unwrap();
    std::fs::write(
        paths.bars_file(),
        r#"
[window]
left = ["project-name", "active-item", "active-item"]
center = ["project-path"]
right = ["settings"]
"#,
    )
    .unwrap();

    let loaded = load_bars(&paths).unwrap();

    assert_eq!(
        format_bar_template(&loaded.settings.window.layout.left),
        "[project-name] [|] [project-path] [|] [git-branch] [|] [git-changes] [active-item] [active-item]"
    );
    assert!(loaded.settings.window.layout.center.is_empty());
    assert!(loaded.warnings.is_empty());
    save_bars(&paths, &loaded.settings).unwrap();

    let saved: toml::Value =
        toml::from_str(&std::fs::read_to_string(paths.bars_file()).unwrap()).unwrap();
    assert!(saved["window"]["left"].is_str());
    assert!(saved["window"]["center"].is_str());
    assert!(saved["window"]["right"].is_str());
}

#[test]
fn explicit_bar_templates_and_empty_regions_survive_loading_and_saving() {
    let dir = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(dir.path());
    std::fs::create_dir_all(paths.config_dir()).unwrap();
    let source = r#"
[window]
left = ""
center = "[text:custom] [Space: 7] [project-path]"
right = ""
[status]
enabled = false
left = ""
center = ""
right = "[app-memory]"
[window.modules.project-path]
max_width = 144
hide_when_empty = false
"#;
    std::fs::write(paths.bars_file(), source).unwrap();

    let loaded = load_bars(&paths).unwrap();

    let expected: ShellBarsSettings = toml::from_str(source).unwrap();
    assert_eq!(loaded.settings, expected);
    assert!(loaded.warnings.is_empty());
    assert_eq!(std::fs::read_to_string(paths.bars_file()).unwrap(), source);
    save_bars(&paths, &loaded.settings).unwrap();
    assert_eq!(load_bars(&paths).unwrap().settings, expected);
}

#[test]
fn invalid_legacy_modules_are_removed_without_filtering_duplicates_or_identity_settings() {
    let dir = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(dir.path());
    std::fs::create_dir_all(paths.config_dir()).unwrap();
    let source = r#"
[window]
left = ["project-name", "missing-module", "active-item", "active-item"]
center = ["project-name", "active_item"]
right = ["settings"]

[window.modules.project_path]
max_width = 12.0
hide_when_empty = false

[window.modules.missing-module]
max_width = 120.0
"#;
    std::fs::write(paths.bars_file(), source).unwrap();

    let loaded = load_bars(&paths).unwrap();

    assert_eq!(
        format_bar_template(&loaded.settings.window.layout.left),
        "[project-name] [|] [project-path] [|] [git-branch] [|] [git-changes] [active-item] [active-item]"
    );
    assert_eq!(
        loaded.settings.window.layout.center,
        vec![ShellBarModule::ActiveItem]
    );
    assert_eq!(
        loaded.settings.window.layout.right,
        vec![ShellBarModule::Settings]
    );
    assert_eq!(
        loaded.settings.window.layout.modules["project-path"],
        BarModuleSettings {
            max_width: None,
            hide_when_empty: false,
        }
    );
    assert!(
        !loaded
            .settings
            .window
            .layout
            .modules
            .contains_key("missing-module")
    );
    assert!(loaded.warnings.iter().any(|warning| matches!(
        warning,
        BarsLoadWarning::InvalidValue {
            field: "window.left",
            value,
        } if value == "missing-module"
    )));
    assert_eq!(std::fs::read_to_string(paths.bars_file()).unwrap(), source);
}

#[test]
fn legacy_bars_load_in_memory_without_rewriting_source() {
    let dir = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(dir.path());
    let legacy_source = r#"
[general]
language = "en"

[bars.window]
left = ["project-name", "active-item"]
center = []
right = ["settings"]

[bars.status]
enabled = false
left = ["surface"]
center = []
right = ["git-branch"]

[bars.status.modules.active_item]
max_width = 280.0
hide_when_empty = false
"#;
    std::fs::create_dir_all(paths.config_dir()).unwrap();
    std::fs::write(paths.settings_file(), legacy_source).unwrap();

    let loaded = load_settings(&paths).unwrap();

    assert!(!loaded.settings.bars.status.enabled);
    assert_eq!(
        format_bar_template(&loaded.settings.bars.window.layout.left),
        "[project-name] [|] [project-path] [|] [git-branch] [|] [git-changes] [active-item]"
    );
    assert!(loaded.warnings.is_empty());
    assert_eq!(
        std::fs::read_to_string(paths.settings_file()).unwrap(),
        legacy_source
    );
    assert!(!paths.bars_file().exists());
}

#[test]
fn standalone_bars_take_precedence_without_rewriting_legacy_settings() {
    let dir = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(dir.path());
    let legacy = "[bars.status]\nenabled = false\nleft = [\"surface\"]\n";
    let standalone = "[status]\nenabled = true\nleft = \"[active-item]\"\n";
    std::fs::write(paths.settings_file(), legacy).unwrap();
    std::fs::write(paths.bars_file(), standalone).unwrap();

    let loaded = load_settings(&paths).unwrap();

    assert!(loaded.settings.bars.status.enabled);
    assert_eq!(
        loaded.settings.bars.status.layout.left,
        vec![ShellBarModule::ActiveItem]
    );
    assert_eq!(
        std::fs::read_to_string(paths.settings_file()).unwrap(),
        legacy
    );
    assert_eq!(
        std::fs::read_to_string(paths.bars_file()).unwrap(),
        standalone
    );
}

#[test]
fn invalid_standalone_bars_fall_back_without_rewriting_source() {
    let dir = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(dir.path());
    std::fs::create_dir_all(paths.config_dir()).unwrap();
    let invalid_source = "[status";
    std::fs::write(paths.bars_file(), invalid_source).unwrap();

    let loaded = load_bars(&paths).unwrap();

    assert_eq!(loaded.settings, ShellBarsSettings::default());
    assert!(matches!(
        loaded.warnings.as_slice(),
        [BarsLoadWarning::InvalidToml { path, message }]
            if path == &paths.bars_file() && !message.is_empty()
    ));
    assert_eq!(
        std::fs::read_to_string(paths.bars_file()).unwrap(),
        invalid_source
    );
}
