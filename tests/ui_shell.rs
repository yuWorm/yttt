use yttt::ui::app::workbench_window_options;
use yttt::ui::components::{SelectableState, selectable_state_classes};
use yttt::ui::palette::palette_surface_style;
use yttt::ui::sidebar::project_sidebar_style;
use yttt::ui::tabs::project_tabs_style;
use yttt::ui::terminal_pane::TerminalPaneView;
use yttt::ui::theme::WorkbenchTheme;
use yttt::ui::titlebar::TitlebarInfo;
use yttt::{model::layout::SplitDirection, ui::root::RootView};

#[test]
fn workbench_theme_exposes_terminal_first_tokens() {
    let theme = WorkbenchTheme::dark();

    assert_ne!(theme.app_background, theme.surface);
    assert_ne!(theme.text, theme.text_muted);
    assert_eq!(theme.terminal_font_size, gpui::px(13.0));
}

#[test]
fn workbench_theme_exposes_zed_like_shell_tokens() {
    let theme = WorkbenchTheme::dark();

    assert_ne!(theme.titlebar_background, theme.terminal_background);
    assert_ne!(theme.sidebar_background, theme.terminal_background);
    assert_eq!(theme.split_line_width, gpui::px(1.0));
}

#[test]
fn selectable_state_classes_distinguish_active_rows() {
    assert!(selectable_state_classes(SelectableState::Active).contains("active"));
    assert!(selectable_state_classes(SelectableState::Inactive).contains("inactive"));
}

#[test]
fn app_window_options_use_custom_titlebar() {
    let bounds = gpui::Bounds {
        origin: gpui::point(gpui::px(0.0), gpui::px(0.0)),
        size: gpui::size(gpui::px(960.0), gpui::px(640.0)),
    };

    let options = workbench_window_options(bounds);

    assert!(options.titlebar.is_some());
}

#[test]
fn titlebar_info_parts_use_compact_project_metadata() {
    let info = TitlebarInfo {
        project_name: "yttt".to_string(),
        compact_path: Some("/Volumes/.../yttt".to_string()),
        git_branch: Some("main".to_string()),
        git_counters: Some("+2 ~4 -1".to_string()),
    };

    assert_eq!(
        info.parts(),
        vec!["yttt", "/Volumes/.../yttt", "main", "+2 ~4 -1"]
    );
}

#[test]
fn split_resize_handle_style_uses_thin_visible_line() {
    let style = RootView::visible_split_handle_style(SplitDirection::Horizontal);

    assert_eq!(style.visible_line_width, gpui::px(1.0));
    assert!(style.hit_area_width >= gpui::px(5.0));
}

#[test]
fn terminal_pane_default_chrome_has_no_header() {
    assert!(!TerminalPaneView::default_chrome().shows_header);
}

#[test]
fn sidebar_and_tabs_use_compact_zed_like_density() {
    let sidebar = project_sidebar_style();
    let tabs = project_tabs_style();

    assert!(sidebar.width <= gpui::px(220.0));
    assert_eq!(sidebar.border_width, gpui::px(1.0));
    assert!(tabs.height <= gpui::px(34.0));
    assert_eq!(tabs.border_width, gpui::px(1.0));
}

#[test]
fn palette_surface_style_is_wide_and_elevated() {
    let style = palette_surface_style();

    assert!(style.width >= gpui::px(720.0));
    assert!(style.max_width >= style.width);
}
