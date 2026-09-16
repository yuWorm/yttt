use gpui::{InteractiveElement as _, ParentElement as _, Styled as _};
use gpui_component::IconName;
use std::{cell::Cell, mem::discriminant, rc::Rc, time::Duration};
use yttt::config::{paths::AppConfigPaths, settings::AppSettings, theme::ThemeStore};
use yttt::ui::components::{
    SelectableState, notification_tone_for_toast, selectable_state_classes,
    workbench_agent_notification, workbench_error_notification,
};
use yttt::ui::editor::{DocumentId, TabGroupId, WorkItemId};
use yttt::ui::i18n::{Locale, UiText};
use yttt::ui::notifications::{ToastItem, ToastTone};
use yttt::ui::palette::surface::{
    PaletteFooterAction, palette_footer_actions, palette_input_placeholder,
    palette_scroll_anchor_index,
};
use yttt::ui::primitives::{
    button::{YtttButtonVariant, yttt_button},
    icon_button::{YtttIconButtonKind, yttt_icon_button, yttt_icon_button_style},
    input::{YtttInputKind, yttt_input_style},
    notification::{YtttNotificationTone, yttt_notification_style},
    panel::{
        KeyboardCapture, YtttPanelKind, modal_overlay_event_policy, popover_overlay_event_policy,
        yttt_panel,
    },
    row::{YtttRowKind, yttt_row_style},
    select::yttt_select_style,
    sidebar::{SidebarSide, SidebarWidthState, resize_sidebar_width, yttt_sidebar_style},
    split::yttt_split_handle_style,
    status::{YtttStatusTone, yttt_status_dot_style},
    switch::{yttt_switch, yttt_switch_style},
    tabs::yttt_tabbar_style,
};
use yttt::ui::settings::font_options::{
    SYSTEM_FONT_FAMILY_LABEL, font_family_option_for_setting, font_family_options_from_system,
    font_family_setting_from_option, recommend_installed_monospace_nerd_font,
    terminal_font_family_option_for_setting, terminal_font_family_options_from_system,
    terminal_font_family_setting_from_option,
};
use yttt::ui::settings::{SettingsGroupId, SettingsPageState, settings_rows_for_group};
use yttt::ui::terminal::pane::TerminalPaneView;
use yttt::ui::theme::icons::IconTheme;
use yttt::ui::theme::{ThemeRuntime, UiStyle, UiStyleId, WorkbenchTheme};
use yttt::ui::workbench::shell::sidebar::project_context_commands;
use yttt::ui::workbench::shell::tabs::{
    DraggedWorkbenchTab, ProjectTabStatusTone, ProjectTabsToolbar, WorkbenchTabCloseScope,
    WorkbenchTabItem, WorkbenchTabKind, project_tabs, project_tree_toggle_icon,
    project_tree_toggle_tooltip, tab_close_targets, tab_toolbar_icon,
};
use yttt::ui::workbench::shell::titlebar::display_path_for_titlebar;
use yttt::{
    commands::CommandId,
    model::{ids::ProjectId, layout::SplitDirection},
    ui::workbench::WorkbenchView,
};

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
ui_style = "rounded"

[terminal]
font_size = 15
"#,
    )
    .unwrap();

    let root = WorkbenchView::with_config_paths(paths);

    assert_eq!(root.theme_runtime().terminal_settings.font_size, 15.0);
    assert_eq!(root.theme_runtime().style_id, UiStyleId::Rounded);
}

#[test]
fn selectable_state_classes_distinguish_active_rows() {
    assert!(selectable_state_classes(SelectableState::Active).contains("active"));
    assert!(selectable_state_classes(SelectableState::Inactive).contains("inactive"));
}

#[test]
fn titlebar_preserves_complete_windows_paths_without_verbatim_prefixes() {
    assert_eq!(
        display_path_for_titlebar(r"\\?\D:\work\yttt"),
        r"D:\work\yttt"
    );
    assert_eq!(
        display_path_for_titlebar(
            r"\\?\C:\Users\example\Projects\Idea\very-long-project-directory"
        ),
        r"C:\Users\example\Projects\Idea\very-long-project-directory"
    );
    assert_eq!(
        display_path_for_titlebar(r"\\?\UNC\server\share\project"),
        r"\\server\share\project"
    );
}

#[test]
fn split_resize_handle_style_uses_thin_visible_line() {
    let style = yttt_split_handle_style(WorkbenchTheme::one_dark());

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
    let sidebar = yttt_sidebar_style(theme, UiStyle::default());
    let tabs = yttt_tabbar_style(theme, UiStyle::default());

    assert_eq!(sidebar.width, gpui::px(320.0));
    assert_eq!(sidebar.default_width, sidebar.width);
    assert_eq!(sidebar.min_width, gpui::px(160.0));
    assert_eq!(sidebar.max_width, gpui::px(420.0));
    assert!(sidebar.collapsed_width < sidebar.width);
    assert_eq!(sidebar.border_width, gpui::px(1.0));
    assert_eq!(sidebar.resize_hit_area_width, gpui::px(5.0));
    assert_eq!(sidebar.item_height, gpui::rems(1.75));
    assert_eq!(sidebar.item_padding_x, gpui::rems(0.5));
    assert_eq!(sidebar.background, theme.app_background);
    assert_eq!(tabs.height, gpui::rems(2.0));
    assert_eq!(tabs.item_height, tabs.height);
    assert_eq!(tabs.border_width, gpui::px(1.0));
    assert_ne!(tabs.active_background, tabs.inactive_background);
    assert_eq!(
        tabs.close_slot_size,
        yttt_icon_button_style(YtttIconButtonKind::TabClose, theme, UiStyle::default()).size
    );
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
        discriminant(&gpui_component::IconName::PanelRightOpen)
    );
    assert_eq!(
        discriminant(&project_tree_toggle_icon(true)),
        discriminant(&gpui_component::IconName::PanelRightClose)
    );
    assert_eq!(project_tree_toggle_tooltip(false), "Show Project Panel");
    assert_eq!(project_tree_toggle_tooltip(true), "Hide Project Panel");
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
            ProjectId::new("test-project"),
            TabGroupId::new(1),
            Vec::new(),
            WorkbenchTheme::one_dark(),
            UiStyle::default(),
            IconTheme::default(),
            UiText::new(Locale::English),
            |_| |_, _, _| {},
            |_| |_, _, _| {},
            |_| |_, _, _| {},
            |_| |_, _, _| {},
            true,
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
                missing_on_disk: false,
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
                missing_on_disk: true,
                icon_path: Some("commands.rs".into()),
                state: SelectableState::Inactive,
            },
        ];

        project_tabs(
            ProjectId::new("test-project"),
            TabGroupId::new(1),
            items,
            WorkbenchTheme::one_dark(),
            UiStyle::default(),
            IconTheme::default(),
            UiText::new(Locale::English),
            |_| |_, _, _| {},
            |_| |_, _, _| {},
            |_| |_, _, _| {},
            |_| |_, _, _| {},
            true,
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
    assert!(cx.debug_bounds("project-tabbar-border-1").is_some());

    let terminal_tab = cx.debug_bounds("project-tab-1-0").unwrap();
    let file_tab = cx.debug_bounds("project-tab-1-1").unwrap();
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
                    missing_on_disk: false,
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
            ProjectId::new("test-project"),
            TabGroupId::new(1),
            self.items.clone(),
            WorkbenchTheme::one_dark(),
            UiStyle::default(),
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
                cx.listener(move |this, dragged: &DraggedWorkbenchTab, _window, cx| {
                    let Some(from_index) =
                        this.items.iter().position(|item| &item.id == dragged.id())
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
            true,
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
    let first = cx.debug_bounds("project-tab-1-0").unwrap();
    let third = cx.debug_bounds("project-tab-1-2").unwrap();

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
    let first = cx.debug_bounds("project-tab-1-0").unwrap();

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
fn agent_notification_close_is_visually_separate_from_action(cx: &mut gpui::TestAppContext) {
    cx.update(gpui_component::init);
    let closed = Rc::new(Cell::new(false));
    let action_clicked = Rc::new(Cell::new(false));
    let closed_for_callback = closed.clone();
    let action_for_callback = action_clicked.clone();
    let (_notification, cx) = cx.add_window_view(move |_, _| {
        workbench_agent_notification(
            ToastItem {
                title: "Refine agent notifications".to_string(),
                status: Some("Agent completed".to_string()),
                context: "yttt › Agent › Codex".to_string(),
                tone: ToastTone::Success,
            },
            "Open",
            WorkbenchTheme::one_dark(),
            UiStyle::default(),
            move |_, _, _| action_for_callback.set(true),
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

    let close = cx.debug_bounds("notification-close").unwrap();
    cx.simulate_click(close.center(), gpui::Modifiers::none());
    cx.background_executor
        .advance_clock(Duration::from_millis(200));
    cx.run_until_parked();

    assert!(
        closed.get(),
        "clicking the close button should dismiss the notification"
    );
    assert!(
        !action_clicked.get(),
        "clicking close must not activate the notification action"
    );
}

#[gpui::test]
fn agent_notification_action_opens_target_and_dismisses(cx: &mut gpui::TestAppContext) {
    cx.update(gpui_component::init);
    let closed = Rc::new(Cell::new(false));
    let action_clicked = Rc::new(Cell::new(false));
    let closed_for_callback = closed.clone();
    let action_for_callback = action_clicked.clone();
    let (_notification, cx) = cx.add_window_view(move |_, _| {
        workbench_agent_notification(
            ToastItem {
                title: "Refine agent notifications".to_string(),
                status: Some("Agent completed".to_string()),
                context: "yttt › Agent › Codex".to_string(),
                tone: ToastTone::Success,
            },
            "Open",
            WorkbenchTheme::one_dark(),
            UiStyle::default(),
            move |_, _, _| action_for_callback.set(true),
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

    let action = cx.debug_bounds("notification-action").unwrap();
    cx.simulate_click(action.center(), gpui::Modifiers::none());
    cx.background_executor
        .advance_clock(Duration::from_millis(200));
    cx.run_until_parked();

    assert!(action_clicked.get(), "action button should open the target");
    assert!(closed.get(), "action should dismiss the notification");
}

#[gpui::test]
fn error_notifications_have_a_visible_working_close_button(cx: &mut gpui::TestAppContext) {
    cx.update(gpui_component::init);
    let closed = Rc::new(Cell::new(false));
    let closed_for_callback = closed.clone();
    let (_notification, cx) = cx.add_window_view(move |_, _| {
        workbench_error_notification(
            ToastItem {
                title: "Error".to_string(),
                status: None,
                context: "Could not open the requested project because its layout is invalid"
                    .to_string(),
                tone: ToastTone::Error,
            },
            WorkbenchTheme::one_dark(),
            UiStyle::default(),
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
    let title = cx.debug_bounds("notification-title").unwrap();
    let icon = cx.debug_bounds("notification-icon").unwrap();
    let close = cx.debug_bounds("notification-close").unwrap();
    let body = cx.debug_bounds("notification-body").unwrap();
    assert_eq!(icon.center().y, title.center().y);
    assert_eq!(close.center().y, title.center().y);
    assert_eq!(body.left(), title.left());
    assert!(body.top() >= title.bottom());
    assert!(
        body.size.height > title.size.height,
        "long errors should wrap rather than truncate"
    );
    assert!(
        body.right() < close.left(),
        "body must not overlap the close control"
    );

    cx.simulate_click(close.center(), gpui::Modifiers::none());
    cx.background_executor
        .advance_clock(Duration::from_millis(200));
    cx.run_until_parked();

    assert!(
        closed.get(),
        "clicking the close button should dismiss the error notification"
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
            CommandId::ProjectCreate,
            CommandId::ProjectOpenSsh,
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
    theme.ghost_element_selected = gpui::rgb(0x123456);

    let style = yttt_sidebar_style(theme, UiStyle::default());

    assert_eq!(style.active_background, gpui::rgb(0x123456));
}

#[test]
fn yttt_tabbar_style_uses_passed_theme() {
    let mut theme = WorkbenchTheme::one_dark();
    theme.tab_active_background = gpui::rgb(0x222244);

    let style = yttt_tabbar_style(theme, UiStyle::default());

    assert_eq!(style.active_background, gpui::rgb(0x222244));
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
fn palette_profiles_centralize_row_density_and_surface_treatment() {
    let theme = WorkbenchTheme::one_dark();
    let zed = UiStyle::default();
    let active = yttt_row_style(
        YtttRowKind::Palette,
        SelectableState::Active,
        true,
        theme,
        zed,
    );
    let compact = yttt_row_style(
        YtttRowKind::PaletteCompact,
        SelectableState::Inactive,
        true,
        theme,
        zed,
    );
    let rounded_inactive = yttt_row_style(
        YtttRowKind::Palette,
        SelectableState::Inactive,
        true,
        theme,
        UiStyle::resolve(UiStyleId::Rounded),
    );

    assert_eq!(active.height, gpui::rems(3.0));
    assert_eq!(compact.height, gpui::rems(2.25));
    assert_eq!(active.radius, gpui::px(4.0));
    assert_eq!(active.background, theme.ghost_element_selected);
    assert_eq!(active.title, theme.text);
    assert_eq!(compact.background, gpui::rgba(0x00000000));
    assert_eq!(compact.hover_background, theme.ghost_element_hover);
    assert_eq!(rounded_inactive.background, theme.element_background);
    assert_eq!(rounded_inactive.hover_background, theme.element_hover);
}

#[test]
fn yttt_row_style_centralizes_settings_row_spacing() {
    let theme = WorkbenchTheme::one_dark();
    let row = yttt_row_style(
        YtttRowKind::Settings,
        SelectableState::Inactive,
        true,
        theme,
        UiStyle::default(),
    );

    assert_eq!(row.height, gpui::rems(4.0));
    assert_eq!(row.padding_y, gpui::rems(1.0));
    assert_eq!(row.border_width, gpui::px(0.0));
    assert_eq!(row.border, gpui::rgba(0x00000000));
    assert_eq!(row.background, theme.ghost_element_background);
    assert_eq!(row.title, theme.text);
    assert_eq!(row.subtitle, theme.text_subtle);
}

#[test]
fn yttt_row_style_uses_domain_specific_sidebar_and_tab_surfaces() {
    let theme = WorkbenchTheme::one_dark();
    let sidebar = yttt_row_style(
        YtttRowKind::Sidebar,
        SelectableState::Inactive,
        true,
        theme,
        UiStyle::default(),
    );
    let tab = yttt_row_style(
        YtttRowKind::Tab,
        SelectableState::Active,
        true,
        theme,
        UiStyle::default(),
    );

    assert_eq!(sidebar.height, gpui::rems(1.75));
    assert_eq!(sidebar.background, gpui::rgba(0x00000000));
    assert_eq!(sidebar.hover_background, theme.ghost_element_hover);
    assert_eq!(tab.height, gpui::rems(2.0));
    assert_eq!(tab.background, theme.tab_active_background);
    assert_eq!(tab.border, theme.border_variant);
}

#[test]
fn yttt_status_dot_style_maps_common_tones_to_theme_colors() {
    let theme = WorkbenchTheme::one_dark();
    let neutral = yttt_status_dot_style(YtttStatusTone::Neutral, theme, UiStyle::default());
    let running = yttt_status_dot_style(YtttStatusTone::Running, theme, UiStyle::default());
    let success = yttt_status_dot_style(YtttStatusTone::Success, theme, UiStyle::default());
    let error = yttt_status_dot_style(YtttStatusTone::Error, theme, UiStyle::default());

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
fn settings_rows_are_grouped_by_user_facing_sections() {
    let text = UiText::english();
    let general_rows = settings_rows_for_group(SettingsGroupId::General, &text);
    let language_rows = settings_rows_for_group(SettingsGroupId::Languages, &text);
    let terminal_rows = settings_rows_for_group(SettingsGroupId::Terminal, &text);
    let agent_rows = settings_rows_for_group(SettingsGroupId::Agent, &text);
    let permission_rows = settings_rows_for_group(SettingsGroupId::Permissions, &text);
    let layout_rows = settings_rows_for_group(SettingsGroupId::DefaultLayout, &text);

    assert!(general_rows.iter().any(|row| row.title == "Language"));
    assert!(
        general_rows
            .iter()
            .any(|row| row.key == "host.settings.toml")
    );
    let appearance_rows = settings_rows_for_group(SettingsGroupId::Appearance, &text);
    assert!(appearance_rows.iter().any(|row| row.key == "bars"));
    assert!(
        appearance_rows
            .iter()
            .any(|row| row.key == "device.settings.toml")
    );
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
    assert!(
        terminal_rows
            .iter()
            .any(|row| row.title == "Environment variables")
    );
    assert!(terminal_rows.iter().any(|row| row.title == "Font size"));
    assert!(terminal_rows.iter().any(|row| row.title == "Cursor shape"));
    assert!(terminal_rows.iter().any(|row| row.title == "Scrollbar"));
    assert!(agent_rows.iter().any(|row| row.title == "Primary agent"));
    assert!(
        agent_rows
            .iter()
            .any(|row| row.key == "agent.sessions_enabled")
    );
    assert!(
        agent_rows
            .iter()
            .any(|row| row.key == "agent.additional_session_agents")
    );
    assert!(agent_rows.iter().any(|row| row.title == "Session list"));
    assert!(
        permission_rows
            .iter()
            .any(|row| row.title == "Protected file access")
    );
    assert!(
        permission_rows
            .iter()
            .any(|row| row.title == "Screen capture")
    );
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
    assert!(terminal_rows.iter().any(|row| row.title == "全局环境变量"));
    assert!(terminal_rows.iter().any(|row| row.title == "光标形状"));
}

#[test]
fn settings_search_keeps_every_category_discoverable_without_a_query() {
    let page = SettingsPageState::default();
    let groups = page.visible_groups(&UiText::english());

    assert_eq!(groups.len(), SettingsGroupId::ALL.len());
    assert!(
        groups
            .iter()
            .any(|group| group.id == SettingsGroupId::Agent)
    );
    assert!(
        groups
            .iter()
            .any(|group| group.id == SettingsGroupId::Terminal)
    );
}

#[test]
fn settings_search_matches_shell_aliases_and_canonical_key_in_chinese() {
    let text = UiText::new(Locale::Chinese);

    for query in ["shell", "default shell", "terminal.shell"] {
        let page = SettingsPageState {
            search_query: query.into(),
            ..Default::default()
        };

        assert!(
            page.matching_rows(SettingsGroupId::Terminal, &text)
                .iter()
                .any(|row| row.key == "terminal.shell")
        );
        assert!(page.matches_row(SettingsGroupId::Terminal, "terminal.shell", &text));
    }
}

#[test]
fn settings_search_matches_english_language_text_and_canonical_keys_in_chinese() {
    let text = UiText::new(Locale::Chinese);

    for (query, key) in [
        ("language server command", "editor.lsp.command"),
        ("editor.lsp.enabled", "editor.lsp.enabled"),
    ] {
        let page = SettingsPageState {
            search_query: query.into(),
            ..Default::default()
        };

        assert_eq!(
            page.visible_groups(&text)
                .into_iter()
                .map(|group| group.id)
                .collect::<Vec<_>>(),
            vec![SettingsGroupId::Languages]
        );
        assert!(page.matches_row(SettingsGroupId::Languages, key, &text));
    }

    let page = SettingsPageState {
        search_query: "not a settings term".into(),
        ..Default::default()
    };
    assert!(page.visible_groups(&text).is_empty());
    assert!(
        page.matching_rows(SettingsGroupId::Languages, &text)
            .is_empty()
    );
    assert!(!page.matches_row(SettingsGroupId::Languages, "editor.lsp.enabled", &text));
}

#[test]
fn floating_layers_leave_keyboard_events_for_focused_inputs() {
    let policy = modal_overlay_event_policy();

    assert_eq!(policy.keyboard, KeyboardCapture::ScopeOnly);
    assert!(policy.mouse);
    assert!(policy.scroll);
}

#[test]
fn modal_overlay_policy_captures_pointer_and_scroll_without_global_keyboard_capture() {
    let policy = modal_overlay_event_policy();

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
fn nerd_font_recommendation_requires_both_a_nerd_name_and_fixed_width() {
    assert_eq!(
        recommend_installed_monospace_nerd_font(["Fira Code", "Maple Mono NF CN"], |font_family| {
            font_family == "Maple Mono NF CN"
        },),
        Some("Maple Mono NF CN".to_string())
    );
    assert_eq!(
        recommend_installed_monospace_nerd_font(["FiraCode Nerd Font"], |_| true),
        Some("FiraCode Nerd Font".to_string())
    );
    assert_eq!(
        recommend_installed_monospace_nerd_font(["JetBrainsMono NFM"], |_| true),
        Some("JetBrainsMono NFM".to_string())
    );
    assert_eq!(
        recommend_installed_monospace_nerd_font(
            ["JetBrains Mono", "Example Nerd Font Propo"],
            |font_family| font_family == "JetBrains Mono",
        ),
        None
    );
}

#[test]
fn nerd_font_recommendation_prefers_maple_mono_then_explicit_mono_families() {
    assert_eq!(
        recommend_installed_monospace_nerd_font(
            [
                "FiraCode Nerd Font",
                "CaskaydiaCove Nerd Font Mono",
                "Maple Mono NF CN",
            ],
            |_| true,
        ),
        Some("Maple Mono NF CN".to_string())
    );
    assert_eq!(
        recommend_installed_monospace_nerd_font(
            ["FiraCode Nerd Font", "CaskaydiaCove Nerd Font Mono"],
            |_| true,
        ),
        Some("CaskaydiaCove Nerd Font Mono".to_string())
    );
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

struct RemScaledControls {
    rem_size: gpui::Pixels,
}

impl gpui::Render for RemScaledControls {
    fn render(
        &mut self,
        window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) -> impl gpui::IntoElement {
        window.set_rem_size(self.rem_size);
        let theme = WorkbenchTheme::one_dark();
        let mut settings = AppSettings::default();
        settings.general.ui_font_size = self.rem_size.into();
        let style = ThemeRuntime::resolve(&settings, &ThemeStore::builtin()).style;

        gpui::div()
            .flex()
            .flex_col()
            .items_start()
            .gap_1()
            .child(
                yttt_panel(YtttPanelKind::Palette, theme, style)
                    .debug_selector(|| "rem-scaled-palette".to_string()),
            )
            .child(
                yttt_button(
                    "rem-scaled-button",
                    "Refresh",
                    YtttButtonVariant::Ghost,
                    theme,
                    UiStyle::default(),
                    cx,
                )
                .debug_selector(|| "rem-scaled-button".to_string()),
            )
            .child(
                yttt_icon_button(
                    "rem-scaled-icon-button",
                    IconName::Search,
                    YtttIconButtonKind::Toolbar,
                    theme,
                    UiStyle::default(),
                    |_, _, _| {},
                )
                .debug_selector(|| "rem-scaled-icon-button".to_string()),
            )
            .child(
                yttt_switch(
                    "rem-scaled-switch",
                    false,
                    theme,
                    UiStyle::default(),
                    |_, _, _| {},
                )
                .debug_selector(|| "rem-scaled-switch".to_string()),
            )
    }
}

#[gpui::test]
fn shared_control_density_tracks_ui_font_size(cx: &mut gpui::TestAppContext) {
    cx.update(gpui_component::init);
    let (view, cx) = cx.add_window_view(|_, _| RemScaledControls {
        rem_size: gpui::px(12.0),
    });
    let compact_button = cx.debug_bounds("rem-scaled-button").unwrap();
    let compact_icon = cx.debug_bounds("rem-scaled-icon-button").unwrap();
    let compact_switch = cx.debug_bounds("rem-scaled-switch").unwrap();
    let compact_palette = cx.debug_bounds("rem-scaled-palette").unwrap();

    view.update(cx, |view, cx| {
        view.rem_size = gpui::px(20.0);
        cx.notify();
    });
    cx.refresh().unwrap();
    let enlarged_button = cx.debug_bounds("rem-scaled-button").unwrap();
    let enlarged_icon = cx.debug_bounds("rem-scaled-icon-button").unwrap();
    let enlarged_switch = cx.debug_bounds("rem-scaled-switch").unwrap();
    let enlarged_palette = cx.debug_bounds("rem-scaled-palette").unwrap();

    assert_eq!(compact_button.size.height, gpui::px(16.5));
    assert_eq!(enlarged_button.size.height, gpui::px(27.5));
    assert_eq!(compact_icon.size.height, gpui::px(21.0));
    assert_eq!(enlarged_icon.size.height, gpui::px(35.0));
    assert_eq!(compact_switch.size.height, gpui::px(24.0));
    assert_eq!(enlarged_switch.size.height, gpui::px(40.0));
    assert_eq!(compact_palette.size.width, gpui::px(34.0 * 12.0));
    assert_eq!(enlarged_palette.size.width, gpui::px(34.0 * 20.0));
}

#[test]
fn yttt_input_style_makes_dialog_input_visible() {
    let theme = WorkbenchTheme::one_dark();
    let style = yttt_input_style(YtttInputKind::Dialog, theme, UiStyle::default());

    assert_eq!(style.height, gpui::rems(2.125));
    assert_eq!(style.background, theme.element_background);
    assert_eq!(style.border, theme.border_variant);
    assert_eq!(style.focused_border, theme.border_focused);
}

#[test]
fn yttt_select_style_matches_settings_input_density() {
    let theme = WorkbenchTheme::one_dark();
    let select = yttt_select_style(theme, UiStyle::default());
    let input = yttt_input_style(YtttInputKind::Settings, theme, UiStyle::default());

    assert_eq!(select.height, input.height);
    assert_eq!(select.radius, input.radius);
    assert_eq!(select.background, theme.element_background);
    assert_eq!(select.border, theme.border_variant);
    assert_eq!(select.text, theme.text);
    assert_eq!(select.menu_width, gpui::px(210.0));
}

#[test]
fn yttt_switch_style_matches_settings_control_density() {
    let theme = WorkbenchTheme::one_dark();
    let switch = yttt_switch_style(theme, UiStyle::default());

    assert_eq!(switch.width, gpui::rems(2.625));
    assert_eq!(switch.height, gpui::rems(1.625));
    assert_eq!(switch.track_width, gpui::rems(2.125));
    assert_eq!(switch.track_height, gpui::rems(1.25));
    assert_eq!(switch.thumb_size, gpui::rems(0.875));
    assert_eq!(switch.track_padding, gpui::rems(0.125));
    assert_eq!(switch.control_height, gpui::rems(2.0));
    assert_eq!(switch.active_background, theme.accent);
    assert_eq!(switch.inactive_background, theme.element_background);
    assert_eq!(switch.active_border, theme.border_variant);
    assert_eq!(switch.inactive_border, theme.border_variant);
    assert_eq!(switch.active_thumb, theme.text);
    assert_eq!(switch.inactive_thumb, theme.text_muted);
}

#[test]
fn yttt_notification_surface_is_opaque_for_translucent_window_themes() {
    let mut theme = WorkbenchTheme::one_dark();
    theme.surface.a = 0.04;

    let notification =
        yttt_notification_style(YtttNotificationTone::Success, theme, UiStyle::default());

    assert_eq!(notification.background, theme.surface.alpha(1.0));
}

#[test]
fn yttt_notification_error_style_uses_danger_tone() {
    let theme = WorkbenchTheme::one_dark();
    let notification =
        yttt_notification_style(YtttNotificationTone::Error, theme, UiStyle::default());

    assert_eq!(notification.tone, theme.danger);
}

#[test]
fn yttt_notification_warning_style_uses_warning_tone() {
    let theme = WorkbenchTheme::one_dark();
    let notification =
        yttt_notification_style(YtttNotificationTone::Warning, theme, UiStyle::default());

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
fn yttt_sidebar_style_centralizes_project_sidebar_density() {
    let theme = WorkbenchTheme::one_dark();
    let primitive = yttt_sidebar_style(theme, UiStyle::default());

    assert_eq!(primitive.width, primitive.default_width);
    assert_eq!(primitive.min_width, gpui::px(160.0));
    assert_eq!(primitive.max_width, gpui::px(420.0));
    assert!(primitive.collapsed_width < primitive.width);
    assert_eq!(primitive.resize_hit_area_width, gpui::px(5.0));
    assert_eq!(primitive.item_height, gpui::rems(1.75));
    assert_eq!(primitive.item_padding_x, gpui::rems(0.5));
    assert_eq!(primitive.background, theme.panel_background);
    assert_eq!(primitive.active_background, theme.ghost_element_selected);
}

#[test]
fn yttt_tabbar_style_centralizes_project_tab_density() {
    let theme = WorkbenchTheme::one_dark();
    let primitive = yttt_tabbar_style(theme, UiStyle::default());

    assert_eq!(
        primitive.close_slot_size,
        yttt_icon_button_style(YtttIconButtonKind::TabClose, theme, UiStyle::default()).size
    );
    assert_eq!(primitive.border_width, gpui::px(1.0));
    assert_eq!(primitive.active_background, theme.tab_active_background);
    assert_eq!(primitive.inactive_background, theme.tab_inactive_background);
    assert_eq!(primitive.hover_background, theme.ghost_element_hover);
}
