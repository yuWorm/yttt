use std::mem::discriminant;
use yttt::config::{paths::AppConfigPaths, settings::TerminalSettings};
use yttt::ui::app::workbench_window_options;
use yttt::ui::components::{SelectableState, selectable_state_classes};
use yttt::ui::font_options::{
    SYSTEM_FONT_FAMILY_LABEL, terminal_font_family_option_for_setting,
    terminal_font_family_options_from_system, terminal_font_family_setting_from_option,
};
use yttt::ui::overlay::{
    KeyboardCapture, overlay_input_capture_policy, popover_overlay_event_policy,
};
use yttt::ui::palette_surface::{
    PaletteFooterAction, PaletteRowTone, palette_footer_actions, palette_panel_style,
    palette_row_style, palette_scroll_anchor_index,
};
use yttt::ui::primitives::{
    button::{YtttButtonVariant, yttt_button_style},
    dialog::yttt_dialog_style,
    input::{YtttInputKind, yttt_input_style},
};
use yttt::ui::settings::{SettingsGroupId, settings_panel_style, settings_rows_for_group};
use yttt::ui::sidebar::project_sidebar_style;
use yttt::ui::tabs::{
    ProjectTabCloseButtonVisibility, ProjectTabLeadingIcon, ProjectTabStatusIndicator,
    project_tabs_style, tab_toolbar_icon,
};
use yttt::ui::terminal_pane::TerminalPaneView;
use yttt::ui::theme::WorkbenchTheme;
use yttt::ui::titlebar::TitlebarInfo;
use yttt::{model::layout::SplitDirection, ui::root::RootView};

#[test]
fn workbench_theme_exposes_terminal_first_tokens() {
    let theme = WorkbenchTheme::dark();
    let terminal = TerminalSettings::default();

    assert_ne!(theme.app_background, theme.surface);
    assert_ne!(theme.text, theme.text_muted);
    assert_eq!(terminal.font_size, 13.0);
}

#[test]
fn root_view_uses_loaded_theme_runtime() {
    let dir = tempfile::tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(dir.path());
    std::fs::create_dir_all(paths.config_dir()).unwrap();
    std::fs::write(
        paths.settings_file(),
        r#"
[theme]
name = "yttt-dark"

[terminal]
font_size = 15
"#,
    )
    .unwrap();

    let root = RootView::with_config_paths(paths);

    assert_eq!(root.theme_runtime().terminal_settings.font_size, 15.0);
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
    assert_eq!(
        options.window_min_size,
        Some(gpui::size(gpui::px(960.0), gpui::px(640.0)))
    );
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
    let theme = WorkbenchTheme::dark();
    let sidebar = project_sidebar_style(theme);
    let tabs = project_tabs_style(theme);

    assert!(sidebar.width <= gpui::px(220.0));
    assert!(sidebar.collapsed_width < sidebar.width);
    assert_eq!(sidebar.border_width, gpui::px(1.0));
    assert_eq!(sidebar.item_height, gpui::px(28.0));
    assert_eq!(sidebar.item_padding_x, gpui::px(8.0));
    assert_eq!(sidebar.background, theme.app_background);
    assert!(tabs.height <= gpui::px(34.0));
    assert_eq!(tabs.border_width, gpui::px(1.0));
    assert_eq!(
        tabs.close_button_visibility,
        ProjectTabCloseButtonVisibility::Hover
    );
    assert_eq!(tabs.leading_icon, ProjectTabLeadingIcon::Terminal);
    assert_eq!(tabs.status_indicator, ProjectTabStatusIndicator::Dot);
    assert_ne!(tabs.active_background, tabs.inactive_background);
}

#[test]
fn sidebar_style_uses_passed_theme() {
    let mut theme = WorkbenchTheme::dark();
    theme.active_surface = gpui::rgb(0x123456);

    let style = project_sidebar_style(theme);

    assert_eq!(style.active_background, gpui::rgb(0x123456));
}

#[test]
fn project_tabs_style_uses_passed_theme() {
    let mut theme = WorkbenchTheme::dark();
    theme.surface = gpui::rgb(0x222244);

    let style = project_tabs_style(theme);

    assert_eq!(style.active_background, gpui::rgb(0x222244));
}

#[test]
fn palette_surface_style_is_wide_elevated_and_scroll_bounded() {
    let style = palette_panel_style();

    assert!(style.width >= gpui::px(720.0));
    assert!(style.max_width >= style.width);
    assert!(style.max_height < gpui::px(520.0));
    assert_eq!(style.row_height, gpui::px(54.0));
    assert_eq!(style.footer_height, gpui::px(44.0));
    assert!(style.list_max_height < style.max_height);
    assert!(style.scrollable);
}

#[test]
fn tab_toolbar_icons_match_split_orientation() {
    assert_eq!(
        discriminant(&tab_toolbar_icon(SplitDirection::Vertical)),
        discriminant(&gpui_component::IconName::PanelBottom)
    );
    assert_eq!(
        discriminant(&tab_toolbar_icon(SplitDirection::Horizontal)),
        discriminant(&gpui_component::IconName::PanelRight)
    );
}

#[test]
fn palette_keyboard_selection_scrolls_to_center() {
    assert_eq!(palette_scroll_anchor_index(0), None);
    assert_eq!(palette_scroll_anchor_index(8), Some(4));
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

#[test]
fn settings_panel_style_uses_zed_like_sidebar_and_content_bounds() {
    let style = settings_panel_style();

    assert_eq!(style.width, gpui::px(900.0));
    assert!(style.max_width >= style.width);
    assert_eq!(style.height, gpui::px(560.0));
    assert!(style.max_height < gpui::px(640.0));
    assert_eq!(style.sidebar_width, gpui::px(240.0));
    assert_eq!(style.row_min_height, gpui::px(72.0));
    assert_eq!(style.control_width, gpui::px(220.0));
    assert_eq!(style.compact_control_width, gpui::px(128.0));
    assert_eq!(style.control_height, gpui::px(32.0));
    assert_eq!(style.search_height, gpui::px(36.0));
    assert_eq!(style.select_menu_width, gpui::px(280.0));
    assert_eq!(style.border_width, gpui::px(1.0));
}

#[test]
fn settings_rows_are_grouped_by_user_facing_sections() {
    let general_rows = settings_rows_for_group(SettingsGroupId::General);
    let terminal_rows = settings_rows_for_group(SettingsGroupId::Terminal);
    let layout_rows = settings_rows_for_group(SettingsGroupId::ProjectLayout);

    assert!(general_rows.iter().any(|row| row.title == "Language"));
    assert!(terminal_rows.iter().any(|row| row.title == "Default shell"));
    assert!(terminal_rows.iter().any(|row| row.title == "Font size"));
    assert!(
        terminal_rows
            .iter()
            .any(|row| row.title == "Close pane on exit")
    );
    assert!(terminal_rows.iter().any(|row| row.title == "Scrollbar"));
    assert!(
        layout_rows
            .iter()
            .any(|row| row.title == "Edit layout TOML")
    );
}

#[test]
fn floating_layers_leave_keyboard_events_for_focused_inputs() {
    let policy = overlay_input_capture_policy();

    assert_eq!(policy.keyboard, KeyboardCapture::ScopeOnly);
    assert!(policy.mouse);
    assert!(policy.scroll);
}

#[test]
fn modal_overlay_policy_captures_pointer_and_scroll_without_global_keyboard_capture() {
    let policy = overlay_input_capture_policy();

    assert_eq!(policy.keyboard, KeyboardCapture::ScopeOnly);
    assert!(policy.mouse);
    assert!(policy.scroll);
    assert!(policy.dismiss_on_escape);
}

#[test]
fn popover_overlay_policy_captures_pointer_and_click_outside() {
    let policy = popover_overlay_event_policy();

    assert_eq!(policy.keyboard, KeyboardCapture::ScopeOnly);
    assert!(policy.mouse);
    assert!(policy.scroll);
    assert!(policy.dismiss_on_click_outside);
}

#[test]
fn font_options_sort_and_dedupe_system_fonts() {
    let options = terminal_font_family_options_from_system(
        "B Font",
        ["Z Font", "A Font", "A Font", "B Font"],
    );

    assert_eq!(
        options,
        vec![SYSTEM_FONT_FAMILY_LABEL, "A Font", "B Font", "Z Font"]
    );
}

#[test]
fn font_options_do_not_inject_hardcoded_recommendations() {
    let options = terminal_font_family_options_from_system("Custom Font", ["Alpha"]);

    assert_eq!(
        options,
        vec![SYSTEM_FONT_FAMILY_LABEL, "Custom Font", "Alpha"]
    );
    assert!(!options.iter().any(|font| font == "monospace"));
    assert!(!options.iter().any(|font| font == "SF Mono"));
    assert!(!options.iter().any(|font| font == "Menlo"));
}

#[test]
fn font_options_prepend_missing_current_font() {
    let options = terminal_font_family_options_from_system("Custom Font", ["Alpha", "Beta"]);

    assert_eq!(
        options,
        vec![SYSTEM_FONT_FAMILY_LABEL, "Custom Font", "Alpha", "Beta"]
    );
}

#[test]
fn font_option_maps_system_default_to_empty_setting() {
    assert_eq!(
        terminal_font_family_option_for_setting(""),
        SYSTEM_FONT_FAMILY_LABEL
    );
    assert_eq!(
        terminal_font_family_setting_from_option(SYSTEM_FONT_FAMILY_LABEL),
        ""
    );
    assert_eq!(
        terminal_font_family_setting_from_option("JetBrains Mono"),
        "JetBrains Mono"
    );
}

#[test]
fn yttt_button_style_keeps_primary_muted_and_compact() {
    let theme = WorkbenchTheme::dark();
    let style = yttt_button_style(YtttButtonVariant::Primary, theme);

    assert_eq!(style.height, gpui::px(28.0));
    assert_eq!(style.radius, gpui::px(6.0));
    assert_eq!(style.background, theme.active_surface);
    assert_ne!(style.background, gpui::rgb(0xffffff));
}

#[test]
fn yttt_input_style_makes_dialog_input_visible() {
    let theme = WorkbenchTheme::dark();
    let style = yttt_input_style(YtttInputKind::Dialog, theme);

    assert_eq!(style.height, gpui::px(34.0));
    assert_eq!(style.background, theme.surface_elevated);
    assert_eq!(style.border, theme.border);
    assert_eq!(style.focused_border, theme.border_strong);
}

#[test]
fn yttt_input_style_has_settings_control_variant() {
    let theme = WorkbenchTheme::dark();
    let style = yttt_input_style(YtttInputKind::Settings, theme);

    assert_eq!(style.height, gpui::px(32.0));
    assert_eq!(style.radius, gpui::px(6.0));
    assert_eq!(style.background, theme.surface_elevated);
    assert_eq!(style.border, theme.border);
    assert_eq!(style.focused_border, theme.focused_pane_border);
}

#[test]
fn yttt_dialog_style_uses_bounded_panel_surface() {
    let theme = WorkbenchTheme::dark();
    let style = yttt_dialog_style(theme);

    assert_eq!(style.max_width, gpui::px(420.0));
    assert_eq!(style.radius, gpui::px(8.0));
    assert_eq!(style.background, theme.surface);
    assert_eq!(style.border, theme.border_strong);
}
