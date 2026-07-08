use yttt::ui::app::workbench_window_options;
use yttt::ui::components::{SelectableState, selectable_state_classes};
use yttt::ui::palette_surface::{
    PaletteFooterAction, PaletteRowTone, palette_footer_actions, palette_panel_style,
    palette_row_style,
};
use yttt::ui::sidebar::project_sidebar_style;
use yttt::ui::tabs::{ProjectTabCloseButtonVisibility, ProjectTabLeadingIcon, project_tabs_style};
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

    assert_eq!(theme.titlebar_background, theme.app_background);
    assert_eq!(theme.sidebar_background, theme.app_background);
    assert_eq!(theme.tabbar_background, theme.app_background);
    assert_ne!(theme.surface, theme.terminal_background);
    assert_eq!(theme.split_line_width, gpui::px(1.0));
}

#[test]
fn workbench_theme_keeps_focus_and_selection_muted() {
    let theme = WorkbenchTheme::dark();

    assert_ne!(theme.focused_pane_border, theme.focus_ring);
    assert_ne!(theme.focused_pane_border, theme.border_strong);
    assert_ne!(theme.focused_pane_border, theme.split_line);
    assert_ne!(theme.active_surface, theme.accent);
    assert_ne!(theme.hover_surface, theme.accent);
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
    assert_eq!(sidebar.item_height, gpui::px(28.0));
    assert_eq!(sidebar.background, WorkbenchTheme::dark().app_background);
    assert!(tabs.height <= gpui::px(34.0));
    assert_eq!(tabs.border_width, gpui::px(1.0));
    assert_eq!(
        tabs.close_button_visibility,
        ProjectTabCloseButtonVisibility::Hover
    );
    assert_eq!(tabs.leading_icon, ProjectTabLeadingIcon::Terminal);
    assert_ne!(tabs.active_background, tabs.inactive_background);
}

#[test]
fn palette_surface_style_is_wide_elevated_and_scroll_bounded() {
    let style = palette_panel_style();

    assert!(style.width >= gpui::px(720.0));
    assert!(style.max_width >= style.width);
    assert_eq!(style.max_height, gpui::px(640.0));
    assert_eq!(style.row_height, gpui::px(54.0));
    assert_eq!(style.footer_height, gpui::px(44.0));
    assert!(style.list_max_height < style.max_height);
    assert!(style.scrollable);
}

#[test]
fn palette_row_style_uses_muted_selection_without_focus_ring() {
    let theme = WorkbenchTheme::dark();
    let active = palette_row_style(SelectableState::Active, true, theme);
    let inactive = palette_row_style(SelectableState::Inactive, true, theme);
    let disabled = palette_row_style(SelectableState::Inactive, false, theme);

    assert_eq!(active.tone, PaletteRowTone::Active);
    assert_eq!(active.background, theme.active_surface);
    assert_eq!(active.border, theme.active_surface);
    assert_ne!(active.border, theme.focus_ring);
    assert_eq!(inactive.background, theme.surface_elevated);
    assert_eq!(disabled.title, theme.text_subtle);
}

#[test]
fn palette_footer_exposes_keyboard_actions() {
    assert_eq!(
        palette_footer_actions(),
        vec![
            PaletteFooterAction {
                label: "Run",
                key: "enter",
            },
            PaletteFooterAction {
                label: "Close",
                key: "esc",
            },
        ]
    );
}
