use gpui::{
    App, ClickEvent, FocusHandle, InteractiveElement as _, IntoElement, MouseButton,
    MouseDownEvent, Pixels, Rems, Rgba, StatefulInteractiveElement as _, Window, div, prelude::*,
};
use gpui_component::{
    Icon, IconName,
    menu::{ContextMenuExt as _, PopupMenuItem},
};

use crate::commands::CommandId;
use crate::config::paths::display_path;
use crate::model::workspace::Workspace;
use crate::ui::components::{SelectableState, workbench_icon_button};
use crate::ui::i18n::{UiText, UiTextKey};
use crate::ui::interaction::actions::{
    CreateProject, LayoutExportProjectConfig, LayoutOpenFile, LayoutProjectEdit,
    LayoutResetLocalOverride, LayoutSaveCurrent, ProjectClose,
};
use crate::ui::terminal::status::{agent_status_label, project_agent_status};
use crate::ui::{
    primitives::{
        icon_button::YtttIconButtonKind,
        row::{YtttRowKind, yttt_row_style},
        sidebar::{
            PROJECT_SIDEBAR_MAX_WIDTH, PROJECT_SIDEBAR_MIN_WIDTH, resize_sidebar_width,
            yttt_sidebar_style,
        },
    },
    theme::WorkbenchTheme,
};

const PROJECT_CONTEXT_COMMANDS: &[CommandId] = &[
    CommandId::ProjectCreate,
    CommandId::LayoutProjectEdit,
    CommandId::LayoutSaveCurrent,
    CommandId::LayoutExportProjectConfig,
    CommandId::LayoutResetLocalOverride,
    CommandId::LayoutOpenFile,
    CommandId::ProjectClose,
];

pub fn project_context_commands() -> &'static [CommandId] {
    PROJECT_CONTEXT_COMMANDS
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectSidebarItem {
    pub id: String,
    pub title: String,
    pub initial: String,
    pub path: String,
    pub agent_status: Option<String>,
    pub state: SelectableState,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ProjectSidebarStyle {
    pub width: Pixels,
    pub default_width: Pixels,
    pub min_width: Pixels,
    pub max_width: Pixels,
    pub collapsed_width: Pixels,
    pub border_width: Pixels,
    pub resize_hit_area_width: Pixels,
    pub item_height: Rems,
    pub item_padding_x: Rems,
    pub background: Rgba,
    pub active_background: Rgba,
    pub hover_background: Rgba,
}

pub fn project_sidebar_style(theme: WorkbenchTheme) -> ProjectSidebarStyle {
    let primitive = yttt_sidebar_style(theme);
    ProjectSidebarStyle {
        width: primitive.width,
        default_width: primitive.default_width,
        min_width: primitive.min_width,
        max_width: primitive.max_width,
        collapsed_width: primitive.collapsed_width,
        border_width: primitive.border_width,
        resize_hit_area_width: primitive.resize_hit_area_width,
        item_height: primitive.item_height,
        item_padding_x: primitive.item_padding_x,
        background: primitive.background,
        active_background: primitive.active_background,
        hover_background: primitive.hover_background,
    }
}

fn project_initial(name: &str) -> String {
    name.trim()
        .chars()
        .next()
        .map(|character| character.to_uppercase().collect())
        .unwrap_or_else(|| "?".to_string())
}

pub fn visible_project_items(workspace: &Workspace) -> Vec<ProjectSidebarItem> {
    let selected_project_id = workspace.selected_project_id();

    workspace
        .opened_projects()
        .iter()
        .map(|project| {
            let configured_name = &project.layout.project.name;
            let path = display_path(&project.path);
            let title = if configured_name.contains(['/', '\\']) {
                compact_path(&path)
            } else {
                configured_name.clone()
            };
            ProjectSidebarItem {
                id: project.id.as_str().to_string(),
                initial: project_initial(&title),
                title,
                path,
                agent_status: project_agent_status(project)
                    .map(agent_status_label)
                    .map(String::from),
                state: if Some(&project.id) == selected_project_id {
                    SelectableState::Active
                } else {
                    SelectableState::Inactive
                },
            }
        })
        .collect()
}

pub fn project_sidebar<SelectH, SelectF, ContextH, ContextF, ToggleH>(
    workspace: &Workspace,
    theme: WorkbenchTheme,
    text: UiText,
    action_context: FocusHandle,
    expanded_width: f32,
    collapsed: bool,
    on_toggle_sidebar: ToggleH,
    mut on_select_project: SelectF,
    mut on_context_project: ContextF,
) -> impl IntoElement
where
    SelectH: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    SelectF: FnMut(String) -> SelectH,
    ContextH: Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
    ContextF: FnMut(String) -> ContextH,
    ToggleH: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
{
    let style = project_sidebar_style(theme);
    let width = if collapsed {
        style.collapsed_width
    } else {
        gpui::px(resize_sidebar_width(
            crate::ui::primitives::sidebar::SidebarSide::Left,
            expanded_width,
            0.0,
            PROJECT_SIDEBAR_MIN_WIDTH,
            PROJECT_SIDEBAR_MAX_WIDTH,
        ))
    };
    let mut sidebar = div()
        .flex()
        .flex_col()
        .flex_none()
        .h_full()
        .w(width)
        .bg(style.background)
        .when(collapsed, |this| {
            this.border_r_1().border_color(theme.border)
        })
        .px_2()
        .py_3()
        .child(project_sidebar_header(collapsed, theme, on_toggle_sidebar));

    for (index, item) in visible_project_items(workspace).into_iter().enumerate() {
        let suffix = match item.agent_status.as_deref() {
            Some(status) => format!("{} · {status}", compact_path(&item.path)),
            None => compact_path(&item.path),
        };
        let on_click = on_select_project(item.id.clone());
        let on_context = on_context_project(item.id.clone());
        sidebar = sidebar.child(project_sidebar_item(
            index,
            item,
            suffix,
            collapsed,
            theme,
            text,
            action_context.clone(),
            on_click,
            on_context,
        ));
    }

    sidebar
}

fn project_sidebar_header<H>(
    collapsed: bool,
    theme: WorkbenchTheme,
    on_toggle_sidebar: H,
) -> impl IntoElement
where
    H: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
{
    let icon = if collapsed {
        IconName::PanelLeftOpen
    } else {
        IconName::PanelLeftClose
    };
    let mut header = div()
        .flex()
        .items_center()
        .justify_between()
        .pb_3()
        .text_xs()
        .text_color(theme.text_subtle);

    if !collapsed {
        header = header.child(div().px_1().child("Projects"));
    }

    header.child(workbench_icon_button(
        "sidebar-toggle",
        icon,
        YtttIconButtonKind::SidebarHeader,
        theme,
        on_toggle_sidebar,
    ))
}

fn project_sidebar_item<H, C>(
    index: usize,
    item: ProjectSidebarItem,
    suffix: String,
    collapsed: bool,
    theme: WorkbenchTheme,
    text: UiText,
    action_context: FocusHandle,
    on_select_project: H,
    on_context_project: C,
) -> impl IntoElement
where
    H: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    C: Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
{
    let row_style = yttt_row_style(YtttRowKind::Sidebar, item.state, true, theme);

    div()
        .id(("project-sidebar-item", index))
        .flex()
        .items_center()
        .justify_between()
        .gap_2()
        .h(row_style.height)
        .w_full()
        .rounded(row_style.radius)
        .px(row_style.padding_x)
        .bg(row_style.background)
        .hover(move |this| this.bg(row_style.hover_background))
        .on_click(on_select_project)
        .on_mouse_down(MouseButton::Right, on_context_project)
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .overflow_hidden()
                .when(collapsed, |this| this.w_full().justify_center())
                .children(collapsed.then(|| {
                    div()
                        .text_sm()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(row_style.title)
                        .debug_selector(move || format!("project-sidebar-initial-{index}"))
                        .child(item.initial)
                }))
                .children((!collapsed).then(|| {
                    Icon::new(IconName::Folder)
                        .size_3()
                        .text_color(row_style.subtitle)
                }))
                .children((!collapsed).then(|| {
                    div()
                        .text_sm()
                        .text_color(row_style.title)
                        .truncate()
                        .child(item.title)
                })),
        )
        .children((!collapsed).then(|| {
            div()
                .flex_none()
                .text_xs()
                .text_color(row_style.status)
                .truncate()
                .child(suffix)
        }))
        .context_menu(move |menu, _, _| {
            menu.action_context(action_context.clone())
                .item(
                    PopupMenuItem::new(text.get(UiTextKey::CommandProjectCreateTitle))
                        .action(Box::new(CreateProject)),
                )
                .item(PopupMenuItem::separator())
                .item(
                    PopupMenuItem::new(text.get(UiTextKey::CommandLayoutProjectEditTitle))
                        .action(Box::new(LayoutProjectEdit)),
                )
                .item(
                    PopupMenuItem::new(text.get(UiTextKey::CommandLayoutSaveCurrentTitle))
                        .action(Box::new(LayoutSaveCurrent)),
                )
                .item(
                    PopupMenuItem::new(text.get(UiTextKey::CommandLayoutExportProjectConfigTitle))
                        .action(Box::new(LayoutExportProjectConfig)),
                )
                .item(PopupMenuItem::separator())
                .item(
                    PopupMenuItem::new(text.get(UiTextKey::CommandLayoutResetLocalOverrideTitle))
                        .action(Box::new(LayoutResetLocalOverride)),
                )
                .item(
                    PopupMenuItem::new(text.get(UiTextKey::CommandLayoutOpenFileTitle))
                        .action(Box::new(LayoutOpenFile)),
                )
                .item(PopupMenuItem::separator())
                .item(
                    PopupMenuItem::new(text.get(UiTextKey::CommandProjectCloseTitle))
                        .action(Box::new(ProjectClose)),
                )
        })
}

fn compact_path(path: &str) -> String {
    path.rsplit(['/', '\\'])
        .find(|part| !part.is_empty())
        .unwrap_or(path)
        .to_string()
}
