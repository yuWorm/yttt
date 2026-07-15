use std::{cell::Cell, mem::discriminant, rc::Rc, time::Duration};
use yttt::config::{paths::AppConfigPaths, settings::TerminalSettings};
use yttt::ui::app::workbench_window_options;
use yttt::ui::components::{
    SelectableState, notification_tone_for_toast, selectable_state_classes,
    workbench_agent_notification,
};
use yttt::ui::editor::{DocumentId, WorkItemId};
use yttt::ui::i18n::{Locale, UiText};
use yttt::ui::interaction::overlay::{
    KeyboardCapture, overlay_input_capture_policy, popover_overlay_event_policy,
};
use yttt::ui::notifications::{ToastItem, ToastTone};
use yttt::ui::palette::surface::{
    PaletteFooterAction, PaletteRowTone, palette_footer_actions, palette_input_placeholder,
    palette_panel_style, palette_row_style, palette_scroll_anchor_index,
};
use yttt::ui::primitives::{
    button::{YtttButtonVariant, yttt_button_style},
    dialog::yttt_dialog_style,
    icon_button::{YtttIconButtonKind, yttt_icon_button_style},
    input::{YtttInputKind, yttt_input_style},
    notification::{YtttNotificationTone, yttt_notification_style},
    panel::{YtttPanelKind, yttt_panel_style},
    row::{YtttRowKind, yttt_row_style},
    select::yttt_select_style,
    sidebar::{SidebarSide, SidebarWidthState, resize_sidebar_width, yttt_sidebar_style},
    status::{YtttStatusTone, yttt_status_dot_style},
    switch::yttt_switch_style,
    tabs::yttt_tabbar_style,
};
use yttt::ui::settings::font_options::{
    SYSTEM_FONT_FAMILY_LABEL, font_family_option_for_setting, font_family_options_from_system,
    font_family_setting_from_option, terminal_font_family_option_for_setting,
    terminal_font_family_options_from_system, terminal_font_family_setting_from_option,
};
use yttt::ui::settings::{SettingsGroupId, settings_panel_style, settings_rows_for_group};
use yttt::ui::terminal::pane::TerminalPaneView;
use yttt::ui::theme::WorkbenchTheme;
use yttt::ui::theme::icons::IconTheme;
use yttt::ui::workbench::shell::sidebar::{project_context_commands, project_sidebar_style};
use yttt::ui::workbench::shell::tabs::{
    ProjectTabCloseButtonVisibility, ProjectTabLeadingIcon, ProjectTabStatusIndicator,
    ProjectTabStatusTone, ProjectTabToolbarPlacement, ProjectTabsToolbar, WorkbenchTabCloseScope,
    WorkbenchTabItem, WorkbenchTabKind, project_tabs, project_tabs_style, project_tree_toggle_icon,
    project_tree_toggle_tooltip, tab_close_targets, tab_toolbar_icon,
};
use yttt::ui::workbench::shell::titlebar::TitlebarInfo;
use yttt::{
    commands::CommandId,
    model::{ids::ProjectId, layout::SplitDirection},
    ui::workbench::WorkbenchView,
};

#[test]
fn workbench_theme_exposes_one_dark_base_tokens() {
    let theme = WorkbenchTheme::one_dark();
    let terminal = TerminalSettings::default();

    assert_eq!(theme.app_background, gpui::rgb(0x23272e));
    assert_eq!(theme.surface, gpui::rgb(0x23272e));
    assert_eq!(theme.surface_elevated, gpui::rgb(0x1e2227));
    assert_eq!(theme.terminal_background, gpui::rgb(0x23272e));
    assert_eq!(theme.text, gpui::rgb(0xabb2bf));
    assert_eq!(theme.text_muted, gpui::rgb(0x7f838c));
    assert_eq!(theme.text_subtle, gpui::rgb(0x495162));
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
name = "one-dark-theme"

[terminal]
font_size = 15
"#,
    )
    .unwrap();

    let root = WorkbenchView::with_config_paths(paths);

    assert_eq!(root.theme_runtime().terminal_settings.font_size, 15.0);
}

#[test]
fn workbench_theme_exposes_one_dark_shell_tokens() {
    let theme = WorkbenchTheme::one_dark();

    assert_eq!(theme.titlebar_background, gpui::rgb(0x23272e));
    assert_eq!(theme.sidebar_background, gpui::rgb(0x23272e));
    assert_eq!(theme.tabbar_background, gpui::rgb(0x1e2227));
    assert_eq!(theme.border, gpui::rgb(0x3e4452));
    assert_eq!(theme.border_strong, gpui::rgb(0x4e5666));
    assert_eq!(theme.split_line, gpui::rgb(0x3e4452));
    assert_eq!(theme.split_line_active, gpui::rgb(0x5a6375));
    assert_eq!(theme.split_line_width, gpui::px(1.0));
    assert_eq!(theme.split_hit_area_width, gpui::px(7.0));
}

#[test]
fn workbench_theme_exposes_one_dark_interaction_and_status_tokens() {
    let theme = WorkbenchTheme::one_dark();

    assert_eq!(theme.accent, gpui::rgb(0x61afef));
    assert_eq!(theme.active_surface, gpui::rgb(0x2c313a));
    assert_eq!(theme.hover_surface, gpui::rgb(0x2c313a));
    assert_eq!(theme.danger, gpui::rgb(0xc24038));
    assert_eq!(theme.success, gpui::rgb(0xa5e075));
    assert_eq!(theme.warning, gpui::rgb(0xd19a66));
    assert_eq!(theme.focus_ring, gpui::rgb(0x3e4452));
    assert_eq!(theme.selection, gpui::rgba(0x67769640));
    assert_eq!(theme.focused_pane_border, gpui::rgb(0x3e4452));
}

#[test]
fn selectable_state_classes_distinguish_active_rows() {
    assert!(selectable_state_classes(SelectableState::Active).contains("active"));
    assert!(selectable_state_classes(SelectableState::Inactive).contains("inactive"));
}

#[test]
fn app_window_options_use_custom_titlebar_and_blurred_background() {
    let bounds = gpui::Bounds {
        origin: gpui::point(gpui::px(0.0), gpui::px(0.0)),
        size: gpui::size(gpui::px(960.0), gpui::px(640.0)),
    };

    let options = workbench_window_options(bounds);

    assert!(options.titlebar.is_some());
    assert_eq!(
        options.window_background,
        gpui::WindowBackgroundAppearance::Blurred
    );
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
    let style = WorkbenchView::visible_split_handle_style(SplitDirection::Horizontal);

    assert_eq!(style.visible_line_width, gpui::px(1.0));
    assert!(style.hit_area_width >= gpui::px(5.0));
}

#[test]
fn terminal_pane_default_chrome_has_no_header() {
    assert!(!TerminalPaneView::default_chrome().shows_header);
}

#[test]
fn sidebar_and_tabs_use_compact_zed_like_density() {
    let theme = WorkbenchTheme::one_dark();
    let sidebar = project_sidebar_style(theme);
    let tabs = project_tabs_style(theme);

    assert!(sidebar.width <= gpui::px(220.0));
    assert_eq!(sidebar.default_width, sidebar.width);
    assert_eq!(sidebar.min_width, gpui::px(160.0));
    assert_eq!(sidebar.max_width, gpui::px(420.0));
    assert!(sidebar.collapsed_width < sidebar.width);
    assert_eq!(sidebar.border_width, gpui::px(1.0));
    assert_eq!(sidebar.resize_hit_area_width, gpui::px(5.0));
    assert_eq!(sidebar.item_height, gpui::px(28.0));
    assert_eq!(sidebar.item_padding_x, gpui::px(8.0));
    assert_eq!(sidebar.background, theme.app_background);
    assert!(tabs.height <= gpui::px(34.0));
    assert_eq!(tabs.border_width, gpui::px(1.0));
    assert_eq!(
        tabs.close_button_visibility,
        ProjectTabCloseButtonVisibility::Hover
    );
    assert_eq!(tabs.leading_icon, ProjectTabLeadingIcon::PerItem);
    assert_eq!(tabs.status_indicator, ProjectTabStatusIndicator::Dot);
    assert!(tabs.dirty_marker_uses_close_slot);
    assert_eq!(
        tabs.toolbar_placement,
        ProjectTabToolbarPlacement::FixedAfterScrollableTabs
    );
    assert_ne!(tabs.active_background, tabs.inactive_background);
    assert_eq!(tabs.active_indicator, theme.accent);
    assert_eq!(tabs.active_indicator_height, gpui::px(2.0));
}

#[test]
fn tab_context_close_scopes_follow_visible_mixed_order() {
    let project_id = ProjectId::new("/tmp/yttt");
    let first_file = WorkItemId::File(DocumentId {
        project_id: project_id.clone(),
        canonical_path: "first.rs".into(),
    });
    let second_file = WorkItemId::File(DocumentId {
        project_id,
        canonical_path: "second.rs".into(),
    });
    let first_terminal = WorkItemId::Terminal("dev".to_string());
    let second_terminal = WorkItemId::Terminal("agent".to_string());
    let items = vec![
        first_terminal.clone(),
        first_file.clone(),
        second_terminal.clone(),
        second_file.clone(),
    ];

    assert_eq!(
        tab_close_targets(&items, &second_terminal, WorkbenchTabCloseScope::Before),
        vec![first_terminal.clone(), first_file.clone()]
    );
    assert_eq!(
        tab_close_targets(&items, &second_terminal, WorkbenchTabCloseScope::After),
        vec![second_file.clone()]
    );
    assert_eq!(
        tab_close_targets(&items, &second_terminal, WorkbenchTabCloseScope::Files),
        vec![first_file, second_file]
    );
    assert_eq!(
        tab_close_targets(&items, &second_terminal, WorkbenchTabCloseScope::Terminals,),
        vec![first_terminal, second_terminal.clone()]
    );
    assert_eq!(
        tab_close_targets(&items, &second_terminal, WorkbenchTabCloseScope::All),
        items
    );
}

#[test]
fn tab_project_tree_toggle_reflects_panel_state() {
    assert_eq!(
        discriminant(&project_tree_toggle_icon(false)),
        discriminant(&gpui_component::IconName::FolderClosed)
    );
    assert_eq!(
        discriminant(&project_tree_toggle_icon(true)),
        discriminant(&gpui_component::IconName::FolderOpen)
    );
    assert_eq!(project_tree_toggle_tooltip(false), "Show Files");
    assert_eq!(project_tree_toggle_tooltip(true), "Hide Files");
}

struct EmptyProjectTabs;

fn noop_tab_toolbar_click(_: &gpui::ClickEvent, _: &mut gpui::Window, _: &mut gpui::App) {}

impl gpui::Render for EmptyProjectTabs {
    fn render(
        &mut self,
        _window: &mut gpui::Window,
        _cx: &mut gpui::Context<Self>,
    ) -> impl gpui::IntoElement {
        project_tabs(
            Vec::new(),
            WorkbenchTheme::one_dark(),
            IconTheme::default(),
            UiText::new(Locale::English),
            |_| |_, _, _| {},
            |_| |_, _, _| {},
            |_| |_, _, _| {},
            |_| |_, _, _| {},
            ProjectTabsToolbar::new(
                false,
                project_tree_toggle_tooltip(false),
                noop_tab_toolbar_click,
                noop_tab_toolbar_click,
                noop_tab_toolbar_click,
                noop_tab_toolbar_click,
            ),
        )
    }
}

#[gpui::test]
fn empty_tabs_keep_project_tree_toggle_visible(cx: &mut gpui::TestAppContext) {
    cx.update(gpui_component::init);
    let (_view, cx) = cx.add_window_view(|_, _| EmptyProjectTabs);

    assert!(cx.debug_bounds("project-tree-toggle").is_some());
}

struct TerminalAndFileTabs;

impl gpui::Render for TerminalAndFileTabs {
    fn render(
        &mut self,
        _window: &mut gpui::Window,
        _cx: &mut gpui::Context<Self>,
    ) -> impl gpui::IntoElement {
        let file_id = DocumentId {
            project_id: ProjectId::new("/tmp/yttt"),
            canonical_path: "commands.rs".into(),
        };
        let items = vec![
            WorkbenchTabItem {
                id: WorkItemId::Terminal("shell".to_string()),
                kind: WorkbenchTabKind::Terminal,
                title: "Shell".to_string(),
                tooltip: "Shell".to_string(),
                status: Some("started".to_string()),
                status_tone: Some(ProjectTabStatusTone::Started),
                dirty: false,
                icon_path: None,
                state: SelectableState::Active,
            },
            WorkbenchTabItem {
                id: WorkItemId::File(file_id),
                kind: WorkbenchTabKind::File,
                title: "commands.rs".to_string(),
                tooltip: "commands.rs".to_string(),
                status: None,
                status_tone: None,
                dirty: false,
                icon_path: Some("commands.rs".into()),
                state: SelectableState::Inactive,
            },
        ];

        project_tabs(
            items,
            WorkbenchTheme::one_dark(),
            IconTheme::default(),
            UiText::new(Locale::English),
            |_| |_, _, _| {},
            |_| |_, _, _| {},
            |_| |_, _, _| {},
            |_| |_, _, _| {},
            ProjectTabsToolbar::new(
                false,
                project_tree_toggle_tooltip(false),
                noop_tab_toolbar_click,
                noop_tab_toolbar_click,
                noop_tab_toolbar_click,
                noop_tab_toolbar_click,
            ),
        )
    }
}

#[gpui::test]
fn file_close_button_uses_terminal_trailing_position(cx: &mut gpui::TestAppContext) {
    cx.update(gpui_component::init);
    let (_view, cx) = cx.add_window_view(|_, _| TerminalAndFileTabs);

    let terminal_tab = cx.debug_bounds("project-tab-0").unwrap();
    let file_tab = cx.debug_bounds("project-tab-1").unwrap();
    let terminal_close = cx.debug_bounds("project-tab-close-0").unwrap();
    let file_close = cx.debug_bounds("project-tab-close-1").unwrap();
    let terminal_trailing_inset = terminal_tab.origin.x + terminal_tab.size.width
        - terminal_close.origin.x
        - terminal_close.size.width;
    let file_trailing_inset =
        file_tab.origin.x + file_tab.size.width - file_close.origin.x - file_close.size.width;

    assert_eq!(terminal_close.size, file_close.size);
    assert_eq!(file_trailing_inset, terminal_trailing_inset);
}

struct ReorderableTabs {
    items: Vec<WorkbenchTabItem>,
    context_selected: Option<WorkItemId>,
}

impl ReorderableTabs {
    fn new() -> Self {
        Self {
            items: ["first", "second", "third"]
                .into_iter()
                .enumerate()
                .map(|(index, id)| WorkbenchTabItem {
                    id: WorkItemId::Terminal(id.to_string()),
                    kind: WorkbenchTabKind::Terminal,
                    title: id.to_string(),
                    tooltip: id.to_string(),
                    status: None,
                    status_tone: None,
                    dirty: false,
                    icon_path: None,
                    state: if index == 0 {
                        SelectableState::Active
                    } else {
                        SelectableState::Inactive
                    },
                })
                .collect(),
            context_selected: None,
        }
    }
}

impl gpui::Render for ReorderableTabs {
    fn render(
        &mut self,
        _window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) -> impl gpui::IntoElement {
        project_tabs(
            self.items.clone(),
            WorkbenchTheme::one_dark(),
            IconTheme::default(),
            UiText::new(Locale::English),
            |_| |_, _, _| {},
            |selected| {
                cx.listener(move |this, _event: &gpui::MouseDownEvent, _window, cx| {
                    this.context_selected = Some(selected.clone());
                    cx.notify();
                })
            },
            |_| |_, _, _| {},
            |target_index| {
                cx.listener(move |this, dragged: &WorkItemId, _window, cx| {
                    let Some(from_index) = this.items.iter().position(|item| &item.id == dragged)
                    else {
                        return;
                    };
                    if from_index == target_index {
                        return;
                    }
                    let moved = this.items.remove(from_index);
                    this.items.insert(target_index.min(this.items.len()), moved);
                    cx.notify();
                })
            },
            ProjectTabsToolbar::new(
                false,
                project_tree_toggle_tooltip(false),
                noop_tab_toolbar_click,
                noop_tab_toolbar_click,
                noop_tab_toolbar_click,
                noop_tab_toolbar_click,
            ),
        )
    }
}

#[gpui::test]
fn dragging_a_tab_reorders_the_rendered_tab_model(cx: &mut gpui::TestAppContext) {
    cx.update(gpui_component::init);
    let (view, cx) = cx.add_window_view(|_, _| ReorderableTabs::new());
    let first = cx.debug_bounds("project-tab-0").unwrap();
    let third = cx.debug_bounds("project-tab-2").unwrap();

    cx.simulate_mouse_down(
        first.center(),
        gpui::MouseButton::Left,
        gpui::Modifiers::none(),
    );
    cx.simulate_mouse_move(
        third.center(),
        gpui::MouseButton::Left,
        gpui::Modifiers::none(),
    );
    cx.simulate_mouse_up(
        third.center(),
        gpui::MouseButton::Left,
        gpui::Modifiers::none(),
    );
    cx.run_until_parked();

    cx.read(|app| {
        assert_eq!(
            view.read(app)
                .items
                .iter()
                .map(|item| item.id.clone())
                .collect::<Vec<_>>(),
            vec![
                WorkItemId::Terminal("second".to_string()),
                WorkItemId::Terminal("third".to_string()),
                WorkItemId::Terminal("first".to_string()),
            ]
        );
    });
}

#[gpui::test]
fn right_clicking_a_tab_selects_the_context_menu_target(cx: &mut gpui::TestAppContext) {
    cx.update(gpui_component::init);
    let (view, cx) = cx.add_window_view(|_, _| ReorderableTabs::new());
    let first = cx.debug_bounds("project-tab-0").unwrap();

    cx.simulate_mouse_move(first.center(), None, gpui::Modifiers::none());
    cx.simulate_mouse_down(
        first.center(),
        gpui::MouseButton::Right,
        gpui::Modifiers::none(),
    );
    cx.run_until_parked();

    cx.read(|app| {
        assert_eq!(
            view.read(app).context_selected,
            Some(WorkItemId::Terminal("first".to_string()))
        );
    });
}

#[gpui::test]
fn agent_notifications_have_a_visible_working_close_button(cx: &mut gpui::TestAppContext) {
    cx.update(gpui_component::init);
    let closed = Rc::new(Cell::new(false));
    let closed_for_callback = closed.clone();
    let (_notification, cx) = cx.add_window_view(move |_, _| {
        workbench_agent_notification(
            ToastItem {
                title: "Codex completed".to_string(),
                context: "yttt / Agent".to_string(),
                tone: ToastTone::Success,
            },
            "Open",
            WorkbenchTheme::one_dark(),
        )
        .autohide(false)
        .on_close(move |_, _| closed_for_callback.set(true))
    });
    cx.background_executor
        .advance_clock(Duration::from_millis(300));
    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });

    cx.simulate_click(
        gpui::point(gpui::px(346.0), gpui::px(14.0)),
        gpui::Modifiers::none(),
    );
    cx.background_executor
        .advance_clock(Duration::from_millis(200));
    cx.run_until_parked();

    assert!(
        closed.get(),
        "clicking the close button should dismiss the notification"
    );
}

#[test]
fn right_sidebar_grows_when_dragged_left() {
    assert_eq!(
        resize_sidebar_width(SidebarSide::Right, 280.0, -40.0, 200.0, 520.0),
        320.0
    );
}

#[test]
fn sidebar_resize_clamps_at_both_bounds() {
    assert_eq!(
        resize_sidebar_width(SidebarSide::Left, 400.0, 80.0, 160.0, 420.0),
        420.0
    );
    assert_eq!(
        resize_sidebar_width(SidebarSide::Left, 170.0, -80.0, 160.0, 420.0),
        160.0
    );
    assert_eq!(
        resize_sidebar_width(SidebarSide::Right, 510.0, -80.0, 200.0, 520.0),
        520.0
    );
    assert_eq!(
        resize_sidebar_width(SidebarSide::Right, 210.0, 80.0, 200.0, 520.0),
        200.0
    );
}

#[test]
fn sidebar_inactive_width_does_not_overwrite_expanded_width() {
    let mut left = SidebarWidthState::new(SidebarSide::Left, 216.0, 160.0, 420.0, 46.0);
    left.set_active(false);
    assert_eq!(left.visible_width(), 46.0);
    assert_eq!(left.expanded_width(), 216.0);
    left.set_active(true);
    assert_eq!(left.visible_width(), 216.0);

    let mut right = SidebarWidthState::new(SidebarSide::Right, 280.0, 200.0, 520.0, 0.0);
    right.set_active(false);
    assert_eq!(right.visible_width(), 0.0);
    assert_eq!(right.expanded_width(), 280.0);
    right.set_active(true);
    assert_eq!(right.visible_width(), 280.0);
}

#[test]
fn project_sidebar_context_exposes_project_commands() {
    assert_eq!(
        project_context_commands(),
        &[
            CommandId::LayoutProjectEdit,
            CommandId::LayoutSaveCurrent,
            CommandId::LayoutExportProjectConfig,
            CommandId::LayoutResetLocalOverride,
            CommandId::LayoutOpenFile,
            CommandId::ProjectClose,
        ]
    );
}

#[test]
fn sidebar_style_uses_passed_theme() {
    let mut theme = WorkbenchTheme::one_dark();
    theme.active_surface = gpui::rgb(0x123456);

    let style = project_sidebar_style(theme);

    assert_eq!(style.active_background, gpui::rgb(0x123456));
}

#[test]
fn project_tabs_style_uses_passed_theme() {
    let mut theme = WorkbenchTheme::one_dark();
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
    let theme = WorkbenchTheme::one_dark();
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
fn yttt_row_style_centralizes_selectable_row_density_and_tones() {
    let theme = WorkbenchTheme::one_dark();
    let active = yttt_row_style(YtttRowKind::Palette, SelectableState::Active, true, theme);
    let inactive = yttt_row_style(YtttRowKind::Palette, SelectableState::Inactive, true, theme);
    let disabled = yttt_row_style(
        YtttRowKind::Palette,
        SelectableState::Inactive,
        false,
        theme,
    );

    assert_eq!(active.height, gpui::px(54.0));
    assert_eq!(active.radius, gpui::px(6.0));
    assert_eq!(active.background, theme.active_surface);
    assert_eq!(active.border, theme.active_surface);
    assert_eq!(active.title, theme.text);
    assert_eq!(inactive.background, theme.surface_elevated);
    assert_eq!(inactive.hover_background, theme.hover_surface);
    assert_eq!(disabled.background, theme.surface_elevated);
    assert_eq!(disabled.title, theme.text_subtle);
    assert_eq!(disabled.subtitle, theme.text_subtle);
}

#[test]
fn yttt_row_style_centralizes_settings_row_spacing() {
    let theme = WorkbenchTheme::one_dark();
    let row = yttt_row_style(
        YtttRowKind::Settings,
        SelectableState::Inactive,
        true,
        theme,
    );

    assert_eq!(row.height, gpui::px(72.0));
    assert_eq!(row.padding_y, gpui::px(12.0));
    assert_eq!(row.border_width, gpui::px(1.0));
    assert_eq!(row.border, theme.border);
    assert_eq!(row.background, theme.surface);
    assert_eq!(row.title, theme.text);
    assert_eq!(row.subtitle, theme.text_subtle);
}

#[test]
fn yttt_row_style_uses_domain_specific_sidebar_and_tab_surfaces() {
    let theme = WorkbenchTheme::one_dark();
    let sidebar = yttt_row_style(YtttRowKind::Sidebar, SelectableState::Inactive, true, theme);
    let tab = yttt_row_style(YtttRowKind::Tab, SelectableState::Active, true, theme);

    assert_eq!(sidebar.height, gpui::px(28.0));
    assert_eq!(sidebar.background, theme.app_background);
    assert_eq!(sidebar.hover_background, theme.hover_surface);
    assert_eq!(tab.height, gpui::px(32.0));
    assert_eq!(tab.background, theme.surface);
    assert_eq!(tab.border, theme.border);
}

#[test]
fn yttt_status_dot_style_maps_common_tones_to_theme_colors() {
    let theme = WorkbenchTheme::one_dark();
    let neutral = yttt_status_dot_style(YtttStatusTone::Neutral, theme);
    let running = yttt_status_dot_style(YtttStatusTone::Running, theme);
    let success = yttt_status_dot_style(YtttStatusTone::Success, theme);
    let error = yttt_status_dot_style(YtttStatusTone::Error, theme);

    assert_eq!(neutral.size, gpui::px(6.0));
    assert_eq!(neutral.color, theme.text_subtle);
    assert_eq!(running.color, theme.accent);
    assert_eq!(success.color, theme.success);
    assert_eq!(error.color, theme.danger);
}

#[test]
fn palette_footer_exposes_keyboard_actions() {
    let text = UiText::english();

    assert_eq!(
        palette_footer_actions(&text),
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
fn palette_surface_text_is_localized() {
    let text = UiText::new(Locale::Chinese);

    assert_eq!(
        palette_input_placeholder(yttt::palette::PaletteKind::Command, &text),
        "执行命令..."
    );
    assert_eq!(
        palette_input_placeholder(yttt::palette::PaletteKind::Project, &text),
        "切换项目..."
    );
    assert_eq!(
        palette_input_placeholder(yttt::palette::PaletteKind::OpenedProject, &text),
        "切换已打开项目..."
    );
    assert_eq!(
        palette_input_placeholder(yttt::palette::PaletteKind::RecentProject, &text),
        "打开最近项目..."
    );
    assert_eq!(
        palette_footer_actions(&text),
        vec![
            PaletteFooterAction {
                label: "运行",
                key: "enter",
            },
            PaletteFooterAction {
                label: "关闭",
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
    let text = UiText::english();
    let general_rows = settings_rows_for_group(SettingsGroupId::General, &text);
    let language_rows = settings_rows_for_group(SettingsGroupId::Languages, &text);
    let terminal_rows = settings_rows_for_group(SettingsGroupId::Terminal, &text);
    let layout_rows = settings_rows_for_group(SettingsGroupId::DefaultLayout, &text);

    assert!(general_rows.iter().any(|row| row.title == "Language"));
    let appearance_rows = settings_rows_for_group(SettingsGroupId::Appearance, &text);
    assert!(appearance_rows.iter().any(|row| row.title == "UI font"));
    assert!(
        appearance_rows
            .iter()
            .any(|row| row.title == "UI font size")
    );
    assert!(
        appearance_rows
            .iter()
            .any(|row| row.title == "UI line height")
    );
    assert!(appearance_rows.iter().any(|row| row.title == "Icon theme"));
    assert!(
        appearance_rows
            .iter()
            .any(|row| row.title == "Import Zed themes")
    );
    assert!(
        language_rows
            .iter()
            .any(|row| row.title == "Language detection")
    );
    assert!(
        language_rows
            .iter()
            .any(|row| row.title == "Default code language")
    );
    assert!(
        language_rows
            .iter()
            .any(|row| row.title == "Language server")
    );
    assert!(terminal_rows.iter().any(|row| row.title == "Default shell"));
    assert!(terminal_rows.iter().any(|row| row.title == "Font size"));
    assert!(terminal_rows.iter().any(|row| row.title == "Cursor shape"));
    assert!(terminal_rows.iter().any(|row| row.title == "Scrollbar"));
    assert!(
        layout_rows
            .iter()
            .any(|row| row.title == "Edit default layout TOML")
    );
    assert!(
        layout_rows
            .iter()
            .any(|row| row.title == "Default layout file")
    );
    assert!(
        layout_rows
            .iter()
            .any(|row| row.title == "Reload default layout")
    );
    assert!(
        layout_rows
            .iter()
            .any(|row| row.title == "Reset default layout")
    );
    assert!(
        !layout_rows
            .iter()
            .any(|row| row.title == "Save current layout")
    );
}

#[test]
fn editor_settings_rows_expose_effective_controls() {
    let text = UiText::english();
    let rows = settings_rows_for_group(SettingsGroupId::Editor, &text);
    let titles = rows.iter().map(|row| row.title).collect::<Vec<_>>();

    assert_eq!(
        titles,
        vec![
            "Font family",
            "Font size",
            "Line height",
            "Tab size",
            "Soft wrap",
            "Line numbers",
            "Autosave",
            "Autosave delay",
            "Open file tree by default",
            "Show hidden files",
            "File tree width",
            "Project sidebar width",
        ]
    );

    let language_titles = settings_rows_for_group(SettingsGroupId::Languages, &text)
        .into_iter()
        .map(|row| row.title)
        .collect::<Vec<_>>();
    assert!(!language_titles.contains(&"Font family"));
    assert!(!language_titles.contains(&"Autosave"));
}

#[test]
fn settings_rows_are_localized() {
    let text = UiText::new(Locale::Chinese);
    let general_rows = settings_rows_for_group(SettingsGroupId::General, &text);
    let language_rows = settings_rows_for_group(SettingsGroupId::Languages, &text);
    let editor_rows = settings_rows_for_group(SettingsGroupId::Editor, &text);
    let terminal_rows = settings_rows_for_group(SettingsGroupId::Terminal, &text);

    assert!(general_rows.iter().any(|row| row.title == "语言"));
    assert!(general_rows.iter().any(|row| row.title == "系统通知"));
    assert!(language_rows.iter().any(|row| row.title == "语言检测"));
    assert!(language_rows.iter().any(|row| row.title == "默认代码语言"));
    assert!(editor_rows.iter().any(|row| row.title == "字体"));
    assert!(editor_rows.iter().any(|row| row.title == "自动保存"));
    assert!(editor_rows.iter().any(|row| row.title == "显示隐藏文件"));
    assert!(terminal_rows.iter().any(|row| row.title == "默认 Shell"));
    assert!(terminal_rows.iter().any(|row| row.title == "光标形状"));
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
fn generic_font_options_are_shared_by_editor_and_terminal_settings() {
    assert_eq!(
        font_family_options_from_system("Custom Font", ["Beta", "Alpha"]),
        vec![SYSTEM_FONT_FAMILY_LABEL, "Custom Font", "Alpha", "Beta"]
    );
    assert_eq!(font_family_option_for_setting(""), SYSTEM_FONT_FAMILY_LABEL);
    assert_eq!(
        font_family_setting_from_option(SYSTEM_FONT_FAMILY_LABEL),
        ""
    );
}

#[test]
fn yttt_button_style_keeps_primary_muted_and_compact() {
    let theme = WorkbenchTheme::one_dark();
    let style = yttt_button_style(YtttButtonVariant::Primary, theme);

    assert_eq!(style.height, gpui::px(28.0));
    assert_eq!(style.radius, gpui::px(6.0));
    assert_eq!(style.background, theme.active_surface);
    assert_ne!(style.background, gpui::rgb(0xffffff));
}

#[test]
fn yttt_icon_button_style_covers_toolbar_sidebar_and_close_density() {
    let theme = WorkbenchTheme::one_dark();
    let toolbar = yttt_icon_button_style(YtttIconButtonKind::Toolbar, theme);
    let sidebar = yttt_icon_button_style(YtttIconButtonKind::SidebarHeader, theme);
    let close = yttt_icon_button_style(YtttIconButtonKind::TabClose, theme);

    assert_eq!(toolbar.size, gpui::px(28.0));
    assert_eq!(toolbar.icon_size, gpui::px(12.0));
    assert_eq!(toolbar.border_width, gpui::px(1.0));
    assert_eq!(toolbar.border, theme.border);
    assert_eq!(toolbar.text, theme.text_muted);
    assert_eq!(toolbar.hover_text, theme.text);
    assert_eq!(sidebar.size, gpui::px(24.0));
    assert_eq!(sidebar.border_width, gpui::px(0.0));
    assert_eq!(sidebar.text, theme.text_subtle);
    assert_eq!(close.size, gpui::px(16.0));
    assert_eq!(close.radius, gpui::px(4.0));
}

#[test]
fn yttt_input_style_makes_dialog_input_visible() {
    let theme = WorkbenchTheme::one_dark();
    let style = yttt_input_style(YtttInputKind::Dialog, theme);

    assert_eq!(style.height, gpui::px(34.0));
    assert_eq!(style.background, theme.surface_elevated);
    assert_eq!(style.border, theme.border);
    assert_eq!(style.focused_border, theme.focus_ring);
}

#[test]
fn yttt_input_style_has_settings_control_variant() {
    let theme = WorkbenchTheme::one_dark();
    let style = yttt_input_style(YtttInputKind::Settings, theme);

    assert_eq!(style.height, gpui::px(32.0));
    assert_eq!(style.radius, gpui::px(6.0));
    assert_eq!(style.background, theme.surface_elevated);
    assert_eq!(style.border, theme.border);
    assert_eq!(style.focused_border, theme.focus_ring);
}

#[test]
fn yttt_dialog_style_uses_bounded_panel_surface() {
    let theme = WorkbenchTheme::one_dark();
    let style = yttt_dialog_style(theme);

    assert_eq!(style.max_width, gpui::px(420.0));
    assert_eq!(style.radius, gpui::px(8.0));
    assert_eq!(style.background, theme.surface);
    assert_eq!(style.border, theme.border_strong);
}

#[test]
fn yttt_panel_style_centralizes_overlay_bounds() {
    let theme = WorkbenchTheme::one_dark();
    let palette = yttt_panel_style(YtttPanelKind::Palette, theme);
    let settings = yttt_panel_style(YtttPanelKind::Settings, theme);
    let dialog = yttt_panel_style(YtttPanelKind::Dialog, theme);

    assert_eq!(palette.width, gpui::px(760.0));
    assert_eq!(palette.max_height, gpui::px(480.0));
    assert_eq!(settings.width, gpui::px(900.0));
    assert_eq!(settings.height, Some(gpui::px(560.0)));
    assert_eq!(dialog.width, gpui::px(420.0));
    assert_eq!(dialog.height, None);
    assert_eq!(palette.background, theme.surface);
    assert_eq!(settings.border, theme.border_strong);
    assert_eq!(dialog.overlay, gpui::rgba(0x00000073));
}

#[test]
fn yttt_select_style_matches_settings_input_density() {
    let theme = WorkbenchTheme::one_dark();
    let select = yttt_select_style(theme);
    let input = yttt_input_style(YtttInputKind::Settings, theme);

    assert_eq!(select.height, input.height);
    assert_eq!(select.radius, input.radius);
    assert_eq!(select.background, theme.surface_elevated);
    assert_eq!(select.border, theme.border);
    assert_eq!(select.text, theme.text);
    assert_eq!(select.menu_width, gpui::px(280.0));
}

#[test]
fn yttt_switch_style_matches_settings_control_density() {
    let theme = WorkbenchTheme::one_dark();
    let switch = yttt_switch_style(theme);

    assert_eq!(switch.width, gpui::px(42.0));
    assert_eq!(switch.height, gpui::px(26.0));
    assert_eq!(switch.track_width, gpui::px(34.0));
    assert_eq!(switch.track_height, gpui::px(20.0));
    assert_eq!(switch.thumb_size, gpui::px(14.0));
    assert_eq!(switch.track_padding, gpui::px(2.0));
    assert_eq!(switch.control_height, gpui::px(32.0));
    assert_eq!(switch.active_background, theme.accent);
    assert_eq!(switch.inactive_background, theme.active_surface);
    assert_eq!(switch.active_border, theme.focus_ring);
    assert_eq!(switch.inactive_border, theme.border_strong);
    assert_eq!(switch.active_thumb, theme.text);
    assert_eq!(switch.inactive_thumb, theme.text_subtle);
}

#[test]
fn yttt_notification_style_matches_zed_like_status_toast_density() {
    let theme = WorkbenchTheme::one_dark();
    let notification = yttt_notification_style(YtttNotificationTone::Success, theme);

    assert_eq!(notification.width, gpui::px(360.0));
    assert_eq!(notification.min_height, gpui::px(44.0));
    assert_eq!(notification.padding_x, gpui::px(12.0));
    assert_eq!(notification.padding_y, gpui::px(8.0));
    assert_eq!(notification.radius, gpui::px(8.0));
    assert_eq!(notification.border_width, gpui::px(1.0));
    assert_eq!(notification.icon_size, gpui::px(14.0));
    assert_eq!(notification.background, theme.surface);
    assert_eq!(notification.border, theme.border);
    assert_eq!(notification.title, theme.text);
    assert_eq!(notification.context, theme.text_subtle);
    assert_eq!(notification.action, theme.text_muted);
    assert_eq!(notification.tone, theme.success);
}

#[test]
fn yttt_notification_error_style_uses_danger_tone() {
    let theme = WorkbenchTheme::one_dark();
    let notification = yttt_notification_style(YtttNotificationTone::Error, theme);

    assert_eq!(notification.tone, theme.danger);
}

#[test]
fn yttt_notification_warning_style_uses_warning_tone() {
    let theme = WorkbenchTheme::one_dark();
    let notification = yttt_notification_style(YtttNotificationTone::Warning, theme);

    assert_eq!(notification.tone, theme.warning);
}

#[test]
fn toast_tones_map_to_workbench_notification_tones() {
    assert_eq!(
        notification_tone_for_toast(ToastTone::Success),
        YtttNotificationTone::Success
    );
    assert_eq!(
        notification_tone_for_toast(ToastTone::Error),
        YtttNotificationTone::Error
    );
    assert_eq!(
        notification_tone_for_toast(ToastTone::Warning),
        YtttNotificationTone::Warning
    );
}

#[test]
fn yttt_sidebar_style_matches_project_sidebar_density() {
    let theme = WorkbenchTheme::one_dark();
    let primitive = yttt_sidebar_style(theme);
    let project = project_sidebar_style(theme);

    assert_eq!(primitive.width, project.width);
    assert_eq!(primitive.default_width, project.default_width);
    assert_eq!(primitive.min_width, project.min_width);
    assert_eq!(primitive.max_width, project.max_width);
    assert_eq!(primitive.collapsed_width, project.collapsed_width);
    assert_eq!(
        primitive.resize_hit_area_width,
        project.resize_hit_area_width
    );
    assert_eq!(primitive.item_height, gpui::px(28.0));
    assert_eq!(primitive.item_padding_x, gpui::px(8.0));
    assert_eq!(primitive.background, theme.app_background);
    assert_eq!(primitive.active_background, theme.active_surface);
}

#[test]
fn yttt_tabbar_style_matches_project_tab_density() {
    let theme = WorkbenchTheme::one_dark();
    let primitive = yttt_tabbar_style(theme);
    let project = project_tabs_style(theme);

    assert_eq!(primitive.height, project.height);
    assert_eq!(primitive.item_height, project.item_height);
    assert_eq!(primitive.close_slot_size, project.close_slot_size);
    assert_eq!(
        primitive.close_slot_size,
        yttt_icon_button_style(YtttIconButtonKind::TabClose, theme).size
    );
    assert_eq!(primitive.border_width, gpui::px(1.0));
    assert_eq!(primitive.active_background, theme.surface);
    assert_eq!(primitive.inactive_background, theme.tabbar_background);
    assert_eq!(primitive.hover_background, theme.hover_surface);
}
