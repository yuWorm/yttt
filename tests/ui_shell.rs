use yttt::ui::components::{SelectableState, selectable_state_classes};
use yttt::ui::theme::WorkbenchTheme;

#[test]
fn workbench_theme_exposes_terminal_first_tokens() {
    let theme = WorkbenchTheme::dark();

    assert_ne!(theme.app_background, theme.surface);
    assert_ne!(theme.text, theme.text_muted);
    assert_eq!(theme.terminal_font_size, gpui::px(13.0));
}

#[test]
fn selectable_state_classes_distinguish_active_rows() {
    assert!(selectable_state_classes(SelectableState::Active).contains("active"));
    assert!(selectable_state_classes(SelectableState::Inactive).contains("inactive"));
}
