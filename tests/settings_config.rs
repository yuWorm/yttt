use std::path::Path;

use tempfile::tempdir;
use yttt::config::{paths::AppConfigPaths, settings::load_or_create_settings};

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

    assert_eq!(loaded.settings.theme.name, "yttt-dark");
    assert_eq!(loaded.settings.theme.terminal, None);
    assert_eq!(loaded.settings.terminal.font_family, "monospace");
    assert_eq!(loaded.settings.terminal.font_size, 13.0);
    assert_eq!(loaded.settings.terminal.line_height, 1.15);
    assert_eq!(loaded.settings.terminal.padding, 6.0);
    assert_eq!(loaded.settings.terminal.scrollback, 10000);
    assert!(paths.settings_file().exists());
    assert!(loaded.warnings.is_empty());
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
