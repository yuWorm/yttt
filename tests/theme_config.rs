use gpui::rgb;
use tempfile::tempdir;
use yttt::config::{
    paths::AppConfigPaths,
    settings::AppSettings,
    theme::{ThemeStore, load_theme_store},
};
use yttt::ui::theme::{AnsiColors, ThemeRuntime};

#[test]
fn theme_store_contains_builtin_one_dark_theme() {
    let store = ThemeStore::builtin();

    assert!(store.theme("one-dark-theme").is_some());
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
        vec!["one-dark-theme".to_string(), "zed-like".to_string()]
    );
}

#[test]
fn theme_runtime_resolves_ui_and_terminal_from_theme_name() {
    let settings = AppSettings::default();
    let store = ThemeStore::builtin();

    let runtime = ThemeRuntime::resolve(&settings, &store);

    assert_eq!(runtime.theme_name, "one-dark-theme");
    assert_eq!(runtime.terminal_settings.font_size, 13.0);
    assert_eq!(runtime.ui.terminal_background, runtime.terminal.background);
}

#[test]
fn workbench_theme_maps_to_gpui_component_theme_config() {
    let runtime = ThemeRuntime::default();
    let config = runtime.to_gpui_component_theme_config();

    assert_eq!(config.name.as_ref(), "one-dark-theme");
    assert_eq!(config.mode, gpui_component::ThemeMode::Dark);
    assert!(config.colors.background.is_some());
    assert!(config.colors.border.is_some());
    assert!(config.colors.input.is_some());
    assert_eq!(
        config
            .colors
            .ring
            .as_ref()
            .map(|color| color.to_string())
            .as_deref(),
        Some("#3e4452")
    );
    assert_eq!(
        config
            .colors
            .selection
            .as_ref()
            .map(|color| color.to_string())
            .as_deref(),
        Some("#67769640")
    );
    assert!(config.colors.title_bar.is_some());
    assert!(config.colors.list_active.is_some());
    assert_eq!(
        config
            .colors
            .switch
            .as_ref()
            .map(|color| color.to_string())
            .as_deref(),
        Some("#1e2227")
    );
    assert_eq!(
        config
            .colors
            .switch_thumb
            .as_ref()
            .map(|color| color.to_string())
            .as_deref(),
        Some("#abb2bf")
    );
}

#[test]
fn theme_loader_overrides_selection_without_changing_focus_ring() {
    let dir = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(dir.path());
    std::fs::create_dir_all(paths.themes_dir()).unwrap();
    std::fs::write(
        paths.themes_dir().join("selection-custom.toml"),
        r##"
name = "selection-custom"
mode = "dark"

[ui]
focus_ring = "#112233"
selection = "#445566"
"##,
    )
    .unwrap();

    let loaded = load_theme_store(&paths).unwrap();
    let mut settings = AppSettings::default();
    settings.theme.name = "selection-custom".to_string();
    let config = ThemeRuntime::resolve(&settings, &loaded.store).to_gpui_component_theme_config();

    assert_eq!(
        config
            .colors
            .ring
            .as_ref()
            .map(|color| color.to_string())
            .as_deref(),
        Some("#112233")
    );
    assert_eq!(
        config
            .colors
            .selection
            .as_ref()
            .map(|color| color.to_string())
            .as_deref(),
        Some("#445566")
    );
    assert!(loaded.warnings.is_empty());
}

#[test]
fn theme_loader_defaults_selection_to_resolved_focus_ring() {
    let dir = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(dir.path());
    std::fs::create_dir_all(paths.themes_dir()).unwrap();
    std::fs::write(
        paths.themes_dir().join("focus-only.toml"),
        r##"
name = "focus-only"
mode = "dark"

[ui]
focus_ring = "#112233"
"##,
    )
    .unwrap();

    let loaded = load_theme_store(&paths).unwrap();
    let mut settings = AppSettings::default();
    settings.theme.name = "focus-only".to_string();
    let config = ThemeRuntime::resolve(&settings, &loaded.store).to_gpui_component_theme_config();

    for color in [&config.colors.ring, &config.colors.selection] {
        assert_eq!(
            color.as_ref().map(|color| color.to_string()).as_deref(),
            Some("#112233")
        );
    }
    assert!(loaded.warnings.is_empty());
}
#[test]
fn builtin_one_dark_theme_maps_editor_and_terminal_palettes() {
    let runtime = ThemeRuntime::default();
    let config = runtime.to_gpui_component_theme_config();
    let highlight = config
        .highlight
        .expect("builtin theme should include editor highlight theme");

    assert_eq!(
        highlight.editor_background,
        Some(gpui::Hsla::from(rgb(0x23272e)))
    );
    assert_eq!(
        highlight.editor_foreground,
        Some(gpui::Hsla::from(rgb(0xabb2bf)))
    );
    assert_eq!(
        highlight.editor_active_line,
        Some(gpui::Hsla::from(rgb(0x2c313c)))
    );
    assert_eq!(
        highlight.editor_line_number,
        Some(gpui::Hsla::from(rgb(0x495162)))
    );
    assert_eq!(
        highlight.editor_active_line_number,
        Some(gpui::Hsla::from(rgb(0xabb2bf)))
    );

    let expected_syntax: [(&str, u32); 16] = [
        ("boolean", 0xd19a66),
        ("comment", 0x7f838c),
        ("comment.doc", 0x7f848e),
        ("constant", 0xd19a66),
        ("constructor", 0xe06c75),
        ("function", 0x61afef),
        ("keyword", 0xc678dd),
        ("number", 0xd19a66),
        ("operator", 0xabb2bf),
        ("property", 0xe06c75),
        ("punctuation", 0xabb2bf),
        ("string", 0x98c379),
        ("string.escape", 0x56b6c2),
        ("type", 0xe5c07b),
        ("variable", 0xe06c75),
        ("variable.special", 0xe5c07b),
    ];
    for (name, color) in expected_syntax {
        assert_eq!(
            highlight.syntax.style(name).and_then(|style| style.color),
            Some(gpui::Hsla::from(rgb(color))),
            "syntax color for {name}"
        );
    }

    assert_eq!(runtime.terminal.background, rgb(0x23272e));
    assert_eq!(runtime.terminal.foreground, rgb(0xabb2bf));
    assert_eq!(runtime.terminal.cursor, Some(rgb(0xabb2bf)));
    assert_eq!(runtime.terminal.selection_background, Some(rgb(0x343b48)));
    assert_eq!(
        runtime.terminal.normal,
        AnsiColors {
            black: rgb(0x3f4451),
            red: rgb(0xe05561),
            green: rgb(0x8cc265),
            yellow: rgb(0xd18f52),
            blue: rgb(0x4aa5f0),
            magenta: rgb(0xc162de),
            cyan: rgb(0x42b3c2),
            white: rgb(0xd7dae0),
        }
    );
    assert_eq!(
        runtime.terminal.bright,
        AnsiColors {
            black: rgb(0x4f5666),
            red: rgb(0xff616e),
            green: rgb(0xa5e075),
            yellow: rgb(0xf0a45d),
            blue: rgb(0x4dc4ff),
            magenta: rgb(0xde73ff),
            cyan: rgb(0x4cd1e0),
            white: rgb(0xe6e6e6),
        }
    );
}

#[test]
fn theme_loader_reads_editor_highlight_theme_from_toml() {
    let dir = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(dir.path());
    std::fs::create_dir_all(paths.themes_dir()).unwrap();
    std::fs::write(
        paths.themes_dir().join("editor-custom.toml"),
        r##"
name = "editor-custom"
mode = "dark"

[editor]
background = "#111111"
foreground = "#eeeeee"
active_line = "#222222"
line_number = "#333333"
active_line_number = "#dddddd"

[editor.syntax]
keyword = "#ff0000"
string = "#00ff00"
comment = "#555555"
"##,
    )
    .unwrap();

    let loaded = load_theme_store(&paths).unwrap();
    let mut settings = AppSettings::default();
    settings.theme.name = "editor-custom".to_string();
    let runtime = ThemeRuntime::resolve(&settings, &loaded.store);
    let config = runtime.to_gpui_component_theme_config();
    let highlight = config
        .highlight
        .expect("custom theme should include editor highlight theme");

    assert_eq!(
        highlight.editor_background,
        Some(gpui::Hsla::from(rgb(0x111111)))
    );
    assert_eq!(
        highlight.editor_active_line,
        Some(gpui::Hsla::from(rgb(0x222222)))
    );
    assert_eq!(
        highlight
            .syntax
            .style("keyword")
            .and_then(|style| style.color),
        Some(gpui::Hsla::from(rgb(0xff0000)))
    );
    assert_eq!(
        highlight
            .syntax
            .style("string")
            .and_then(|style| style.color),
        Some(gpui::Hsla::from(rgb(0x00ff00)))
    );
    assert!(loaded.warnings.is_empty());
}

#[test]
fn terminal_config_uses_runtime_settings_and_colors() {
    let mut runtime = ThemeRuntime::default();
    runtime.terminal_settings.font_family = "JetBrains Mono".to_string();
    runtime.terminal_settings.font_size = 15.0;
    runtime.terminal_settings.padding = 8.0;
    runtime.terminal_settings.show_scrollbar = false;
    runtime.terminal_settings.cursor_shape = yttt_terminal::TerminalCursorShape::Beam;
    runtime.terminal_settings.cursor_blinking = true;
    runtime.terminal_settings.hide_mouse_when_typing = true;
    runtime.terminal_settings.copy_on_select = true;
    runtime.terminal_settings.osc52_policy = yttt_terminal::TerminalOsc52Policy::ReadWrite;
    runtime.terminal_settings.kitty_keyboard = true;

    let config = runtime.to_terminal_config();

    assert_eq!(config.font_family, "JetBrains Mono");
    assert_eq!(config.font_size, gpui::px(15.0));
    assert_eq!(config.padding.left, gpui::px(8.0));
    assert_eq!(config.scrollback, 10000);
    assert!(!config.show_scrollbar);
    assert_eq!(
        config.cursor_shape,
        yttt_terminal::TerminalCursorShape::Beam
    );
    assert!(config.cursor_blinking);
    assert!(config.hide_mouse_when_typing);
    assert!(config.copy_on_select);
    assert_eq!(
        config.osc52_policy,
        yttt_terminal::TerminalOsc52Policy::ReadWrite
    );
    assert!(config.kitty_keyboard);
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
