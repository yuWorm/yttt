use tempfile::tempdir;
use yttt::config::{
    paths::AppConfigPaths,
    settings::AppSettings,
    theme::{ThemeStore, load_theme_store},
};
use yttt::ui::theme::ThemeRuntime;

#[test]
fn theme_store_contains_builtin_yttt_dark() {
    let store = ThemeStore::builtin();

    assert!(store.theme("yttt-dark").is_some());
}

#[test]
fn theme_loader_reads_user_theme_toml() {
    let dir = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(dir.path());
    std::fs::create_dir_all(paths.themes_dir()).unwrap();
    std::fs::write(
        paths.themes_dir().join("custom.toml"),
        r##"
name = "custom"
mode = "dark"

[ui]
background = "#101010"
text = "#eeeeee"

[terminal.colors.primary]
background = "#101010"
foreground = "#eeeeee"
"##,
    )
    .unwrap();

    let loaded = load_theme_store(&paths).unwrap();

    assert!(loaded.store.theme("custom").is_some());
    assert!(loaded.warnings.is_empty());
}

#[test]
fn theme_store_exposes_sorted_theme_names_for_settings_selects() {
    let dir = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(dir.path());
    std::fs::create_dir_all(paths.themes_dir()).unwrap();
    std::fs::write(
        paths.themes_dir().join("zed-like.toml"),
        r##"
name = "zed-like"
mode = "dark"

[ui]
background = "#20232a"
"##,
    )
    .unwrap();

    let loaded = load_theme_store(&paths).unwrap();

    assert_eq!(
        loaded.store.theme_names(),
        vec!["yttt-dark".to_string(), "zed-like".to_string()]
    );
}

#[test]
fn theme_runtime_resolves_ui_and_terminal_from_theme_name() {
    let settings = AppSettings::default();
    let store = ThemeStore::builtin();

    let runtime = ThemeRuntime::resolve(&settings, &store);

    assert_eq!(runtime.theme_name, "yttt-dark");
    assert_eq!(runtime.terminal_settings.font_size, 13.0);
    assert_eq!(runtime.ui.terminal_background, runtime.terminal.background);
}

#[test]
fn workbench_theme_maps_to_gpui_component_theme_config() {
    let runtime = ThemeRuntime::default();
    let config = runtime.to_gpui_component_theme_config();

    assert_eq!(config.name.as_ref(), "yttt-dark");
    assert_eq!(config.mode, gpui_component::ThemeMode::Dark);
    assert!(config.colors.background.is_some());
    assert!(config.colors.border.is_some());
    assert!(config.colors.input.is_some());
    assert!(config.colors.title_bar.is_some());
    assert!(config.colors.list_active.is_some());
}

#[test]
fn terminal_config_uses_runtime_settings_and_colors() {
    let mut runtime = ThemeRuntime::default();
    runtime.terminal_settings.font_family = "JetBrains Mono".to_string();
    runtime.terminal_settings.font_size = 15.0;
    runtime.terminal_settings.padding = 8.0;
    runtime.terminal_settings.show_scrollbar = false;

    let config = runtime.to_terminal_config();

    assert_eq!(config.font_family, "JetBrains Mono");
    assert_eq!(config.font_size, gpui::px(15.0));
    assert_eq!(config.padding.left, gpui::px(8.0));
    assert_eq!(config.scrollback, 10000);
    assert!(!config.show_scrollbar);
}

#[test]
fn terminal_config_uses_upstream_default_font_when_setting_is_empty() {
    let mut runtime = ThemeRuntime::default();
    runtime.terminal_settings.font_family = String::new();

    let config = runtime.to_terminal_config();

    assert_eq!(
        config.font_family,
        yttt_terminal::TerminalConfig::default().font_family
    );
}
