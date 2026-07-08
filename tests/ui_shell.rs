use yttt::ui::theme::WorkbenchTheme;

#[test]
fn workbench_theme_exposes_terminal_first_tokens() {
    let theme = WorkbenchTheme::dark();

    assert_ne!(theme.app_background, theme.surface);
    assert_ne!(theme.text, theme.text_muted);
    assert_eq!(theme.terminal_font_size, gpui::px(13.0));
}
