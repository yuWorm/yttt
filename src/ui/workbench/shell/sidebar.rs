use gpui::{
    AnyElement, App, ClickEvent, FocusHandle, InteractiveElement as _, IntoElement, MouseButton,
    MouseDownEvent, SharedString, StatefulInteractiveElement as _, Window, div, prelude::*,
};
use gpui_component::{
    Icon, IconName, Sizable as _,
    menu::{ContextMenuExt as _, PopupMenuItem},
    spinner::Spinner,
    tooltip::Tooltip,
};

use crate::commands::CommandId;
use crate::config::default_layout::BuiltinAgent;
use crate::model::workspace::Workspace;
use crate::ui::app::assets::{
    BUILTIN_CLAUDE_ICON_ASSET_PATH, BUILTIN_CODEX_ICON_ASSET_PATH, BUILTIN_OMP_ICON_ASSET_PATH,
    BUILTIN_OPENCODE_ICON_ASSET_PATH, BUILTIN_PI_ICON_ASSET_PATH,
};
use crate::ui::components::SelectableState;
use crate::ui::i18n::{UiText, UiTextKey};
use crate::ui::interaction::actions::{
    CreateProject, LayoutExportProjectConfig, LayoutOpenFile, LayoutProjectEdit,
    LayoutResetLocalOverride, LayoutSaveCurrent, ProjectClose,
};
use crate::ui::terminal::status::{
    agent_status_label, is_agent_pane, pane_agent_status, project_agent_status,
};
use crate::ui::{
    primitives::{
        icon_button::{YtttIconButtonKind, yttt_icon_button},
        row::{YtttRowKind, yttt_row, yttt_row_style},
        sidebar::{
            PROJECT_SIDEBAR_MAX_WIDTH, PROJECT_SIDEBAR_MIN_WIDTH, resize_sidebar_width,
            yttt_sidebar_style,
        },
    },
    theme::{UiStyle, WorkbenchTheme},
};
use yttt_agent_core::{AgentTurnState, AgentViewState};

const PROJECT_CONTEXT_COMMANDS: &[CommandId] = &[
    CommandId::ProjectCreate,
    CommandId::ProjectOpenSsh,
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
pub struct ProjectSidebarChildAgentItem {
    pub name: String,
    pub state: AgentViewState,
    pub task: String,
    pub action: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectSidebarAgentItem {
    pub tab_id: String,
    pub pane_id: String,
    pub name: String,
    pub provider_id: String,
    pub state: AgentViewState,
    pub task: String,
    pub action: Option<String>,
    pub children: Vec<ProjectSidebarChildAgentItem>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectSidebarItem {
    pub id: String,
    pub title: String,
    pub initial: String,
    pub path: String,
    pub agent_state: Option<AgentViewState>,
    pub agents: Vec<ProjectSidebarAgentItem>,
    pub state: SelectableState,
}

fn project_initial(name: &str) -> String {
    name.trim()
        .chars()
        .next()
        .map(|character| character.to_uppercase().collect())
        .unwrap_or_else(|| "?".to_string())
}

fn project_agent_items(
    project: &crate::model::workspace::OpenedProject,
) -> Vec<ProjectSidebarAgentItem> {
    let mut agents = Vec::new();
    for tab in &project.layout.tabs {
        let Some(tab_state) = project.tab_state(&tab.id) else {
            continue;
        };
        for pane_state in &tab_state.pane_states {
            let Some(pane) = tab.layout.find_pane(&pane_state.pane_id) else {
                continue;
            };
            let configured_agent = is_agent_pane(pane);
            if !configured_agent && pane_state.agent_snapshot.is_none() {
                continue;
            }
            let Some(state) = pane_agent_status(pane, pane_state) else {
                continue;
            };
            let (mut task, action, children, provider_id) = pane_state
                .agent_snapshot
                .as_ref()
                .map(|snapshot| {
                    (
                        snapshot.primary_text(),
                        snapshot.secondary_text(),
                        snapshot
                            .children
                            .iter()
                            .map(|child| ProjectSidebarChildAgentItem {
                                name: child.name.clone().unwrap_or_else(|| child.id.clone()),
                                state: child_agent_view_state(child.turn_state),
                                task: child.primary_text(),
                                action: child.secondary_text(),
                            })
                            .collect(),
                        snapshot.provider_id.to_string(),
                    )
                })
                .unwrap_or_else(|| {
                    (
                        String::new(),
                        None,
                        Vec::new(),
                        agent_provider_id_from_command(&pane.command),
                    )
                });
            let name = if configured_agent {
                pane.title.clone()
            } else {
                BuiltinAgent::ALL
                    .into_iter()
                    .find(|agent| agent.id() == provider_id)
                    .map(|agent| agent.display_name().to_string())
                    .unwrap_or_else(|| pane.title.clone())
            };
            if !configured_agent
                && pane_state
                    .agent_snapshot
                    .as_ref()
                    .is_some_and(|snapshot| snapshot.task.is_none())
            {
                task.clear();
            }
            agents.push(ProjectSidebarAgentItem {
                tab_id: tab.id.clone(),
                pane_id: pane.id.clone(),
                name,
                provider_id,
                state,
                task,
                action,
                children,
            });
        }
    }
    agents
}

fn child_agent_view_state(state: AgentTurnState) -> AgentViewState {
    match state {
        AgentTurnState::Idle => AgentViewState::Idle,
        AgentTurnState::Working => AgentViewState::Working,
        AgentTurnState::Waiting => AgentViewState::Waiting,
        AgentTurnState::Completed => AgentViewState::Completed,
        AgentTurnState::Failed => AgentViewState::Failed,
        AgentTurnState::Interrupted => AgentViewState::Interrupted,
        AgentTurnState::Unknown => AgentViewState::Stale,
    }
}
fn agent_provider_id_from_command(command: &str) -> String {
    let program = command.split_whitespace().next().unwrap_or_default();
    match program
        .rsplit(['/', '\\'])
        .find(|component| !component.is_empty())
        .unwrap_or_default()
    {
        "oh-my-pi" => "omp".to_string(),
        provider => provider.to_string(),
    }
}

pub fn visible_project_items(workspace: &Workspace) -> Vec<ProjectSidebarItem> {
    let selected_project_id = workspace.selected_project_id();

    workspace
        .opened_projects()
        .iter()
        .map(|project| {
            let configured_name = &project.layout.project.name;
            let path = project.location.display_path();
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
                agent_state: project_agent_status(project),
                agents: project_agent_items(project),
                state: if Some(&project.id) == selected_project_id {
                    SelectableState::Active
                } else {
                    SelectableState::Inactive
                },
            }
        })
        .collect()
}

pub fn project_sidebar<
    FocusH,
    SelectH,
    SelectF,
    ProjectToggleH,
    ProjectToggleF,
    AgentSelectH,
    AgentSelectF,
    ContextH,
    ContextF,
    ToggleH,
>(
    workspace: &Workspace,
    theme: WorkbenchTheme,
    ui_style: UiStyle,
    text: UiText,
    action_context: FocusHandle,
    has_keyboard_focus: bool,
    expanded_width: f32,
    collapsed: bool,
    collapsed_agent_projects: &[String],
    on_focus_sidebar: FocusH,
    on_toggle_sidebar: ToggleH,
    mut on_select_project: SelectF,
    mut on_toggle_project: ProjectToggleF,
    mut on_select_agent: AgentSelectF,
    mut on_context_project: ContextF,
) -> impl IntoElement
where
    FocusH: Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
    SelectH: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    SelectF: FnMut(String) -> SelectH,
    ProjectToggleH: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ProjectToggleF: FnMut(String) -> ProjectToggleH,
    AgentSelectH: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    AgentSelectF: FnMut(String, String, String) -> AgentSelectH,
    ContextH: Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
    ContextF: FnMut(String) -> ContextH,
    ToggleH: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
{
    let style = yttt_sidebar_style(theme, ui_style);
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
        .on_mouse_down(MouseButton::Left, on_focus_sidebar)
        .flex()
        .flex_col()
        .flex_none()
        .h_full()
        .w(width)
        .bg(style.background)
        .when(collapsed, |this| {
            this.border_r(style.border_width).border_color(theme.border)
        })
        .px(ui_style.spacing.md)
        .py(ui_style.spacing.lg)
        .child(project_sidebar_header(
            collapsed,
            theme,
            ui_style,
            on_toggle_sidebar,
        ));

    for (index, item) in visible_project_items(workspace).into_iter().enumerate() {
        let compacted_path = compact_path(&item.path);
        let suffix = (compacted_path != item.title)
            .then_some(compacted_path)
            .unwrap_or_default();
        let expanded = !collapsed_agent_projects.contains(&item.id);
        let agents = item.agents.clone();
        let on_click = on_select_project(item.id.clone());
        let on_toggle = on_toggle_project(item.id.clone());
        let on_context = on_context_project(item.id.clone());
        sidebar = sidebar.child(project_sidebar_item(
            index,
            item.clone(),
            suffix,
            collapsed,
            expanded,
            theme,
            ui_style,
            has_keyboard_focus,
            text,
            action_context.clone(),
            on_click,
            on_toggle,
            on_context,
        ));
        if !collapsed && expanded {
            for (agent_index, agent) in agents.into_iter().enumerate() {
                let on_click =
                    on_select_agent(item.id.clone(), agent.tab_id.clone(), agent.pane_id.clone());
                sidebar = sidebar.child(project_sidebar_agent_item(
                    index,
                    agent_index,
                    agent,
                    theme,
                    ui_style,
                    on_click,
                ));
            }
        }
    }

    sidebar
}

fn project_sidebar_header<H>(
    collapsed: bool,
    theme: WorkbenchTheme,
    ui_style: UiStyle,
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
        .pb(ui_style.spacing.lg)
        .text_xs()
        .text_color(theme.text_subtle);

    if !collapsed {
        header = header.child(div().px(ui_style.spacing.xs).child("Projects"));
    }

    header.child(yttt_icon_button(
        "sidebar-toggle",
        icon,
        YtttIconButtonKind::SidebarHeader,
        theme,
        ui_style,
        on_toggle_sidebar,
    ))
}

fn agent_state_color(state: AgentViewState, theme: WorkbenchTheme) -> gpui::Rgba {
    match state {
        AgentViewState::Starting | AgentViewState::Working => theme.warning,
        AgentViewState::Waiting => theme.warning,
        AgentViewState::Completed => theme.success,
        AgentViewState::Failed | AgentViewState::Interrupted => theme.danger,
        AgentViewState::Idle | AgentViewState::Stale => theme.text_subtle,
    }
}

fn agent_state_icon(
    element_id: SharedString,
    state: AgentViewState,
    theme: WorkbenchTheme,
) -> AnyElement {
    let color = agent_state_color(state, theme);
    let label: SharedString = agent_status_label(state).into();
    let glyph = match state {
        AgentViewState::Starting | AgentViewState::Working => Spinner::new()
            .with_size(gpui::px(11.0))
            .color(color.into())
            .into_any_element(),
        AgentViewState::Waiting => Icon::new(IconName::TriangleAlert)
            .size(gpui::px(11.0))
            .text_color(color)
            .into_any_element(),
        AgentViewState::Completed => Icon::new(IconName::CircleCheck)
            .size(gpui::px(11.0))
            .text_color(color)
            .into_any_element(),
        AgentViewState::Failed | AgentViewState::Interrupted => Icon::new(IconName::CircleX)
            .size(gpui::px(11.0))
            .text_color(color)
            .into_any_element(),
        AgentViewState::Idle | AgentViewState::Stale => Icon::new(IconName::Pause)
            .size(gpui::px(10.0))
            .text_color(color)
            .into_any_element(),
    };

    div()
        .id(element_id)
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .size(gpui::px(14.0))
        .tooltip(move |window, cx| Tooltip::new(label.clone()).build(window, cx))
        .child(glyph)
        .into_any_element()
}

#[derive(Clone, Copy)]
enum AgentLogo {
    Polychrome(&'static str),
    Monochrome(&'static str),
    Generic,
}

pub(in super::super) fn agent_type_icon(
    element_id: SharedString,
    provider_id: &str,
    theme: WorkbenchTheme,
) -> AnyElement {
    let (label, logo): (SharedString, AgentLogo) = match provider_id {
        "codex" => (
            "Codex".into(),
            AgentLogo::Polychrome(BUILTIN_CODEX_ICON_ASSET_PATH),
        ),
        "claude" => (
            "Claude Code".into(),
            AgentLogo::Polychrome(BUILTIN_CLAUDE_ICON_ASSET_PATH),
        ),
        "opencode" => (
            "OpenCode".into(),
            AgentLogo::Monochrome(BUILTIN_OPENCODE_ICON_ASSET_PATH),
        ),
        "pi" => (
            "Pi".into(),
            AgentLogo::Monochrome(BUILTIN_PI_ICON_ASSET_PATH),
        ),
        "omp" => (
            "Oh My Pi".into(),
            AgentLogo::Monochrome(BUILTIN_OMP_ICON_ASSET_PATH),
        ),
        "child" => ("Child agent".into(), AgentLogo::Generic),
        provider => (provider.to_string().into(), AgentLogo::Generic),
    };
    let glyph = match logo {
        // GPUI's `svg()` paints an alpha mask and intentionally applies one text color.
        // `img()` routes SVG bytes through the full-color renderer, preserving fills and gradients.
        AgentLogo::Polychrome(path) => gpui::img(path).size(gpui::px(14.0)).into_any_element(),
        AgentLogo::Monochrome(path) => gpui::svg()
            .path(path)
            .size(gpui::px(14.0))
            .text_color(theme.accent)
            .into_any_element(),
        AgentLogo::Generic => Icon::new(IconName::Bot)
            .size(gpui::px(12.0))
            .text_color(if provider_id == "child" {
                theme.text_subtle
            } else {
                theme.accent
            })
            .into_any_element(),
    };
    let debug_element_id = element_id.clone();

    div()
        .id(element_id)
        .debug_selector(move || debug_element_id.to_string())
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .size(gpui::px(16.0))
        .tooltip(move |window, cx| Tooltip::new(label.clone()).build(window, cx))
        .child(glyph)
        .into_any_element()
}

fn compact_agent_label(
    mut name: String,
    provider_id: &str,
    task: String,
    action: Option<String>,
    show_name_with_task: bool,
) -> String {
    let task = task.trim();
    let has_distinct_task = !task.is_empty()
        && !task.eq_ignore_ascii_case(name.trim())
        && !task.eq_ignore_ascii_case(provider_id);
    if has_distinct_task {
        if show_name_with_task {
            name.push_str(" — ");
        } else {
            name.clear();
        }
        name.push_str(task);
    }
    if let Some(action) = action.as_deref().map(str::trim).filter(|action| {
        !action.is_empty()
            && !action.eq_ignore_ascii_case(task)
            && !action.eq_ignore_ascii_case(name.trim())
    }) {
        name.push_str(" · ");
        name.push_str(action);
    }
    name
}

fn child_agent_row(
    row_id: String,
    child: ProjectSidebarChildAgentItem,
    theme: WorkbenchTheme,
    ui_style: UiStyle,
) -> impl IntoElement {
    let state_id = format!("{row_id}-state").into();
    let type_id = format!("{row_id}-type").into();
    let label_id: SharedString = format!("{row_id}-label").into();
    let label: SharedString =
        compact_agent_label(child.name, "child", child.task, child.action, true).into();
    let tooltip_label = label.clone();
    div()
        .ml(gpui::px(34.0))
        .mr(ui_style.spacing.xs)
        .mb(gpui::px(3.0))
        .h(gpui::px(24.0))
        .px(ui_style.spacing.sm)
        .flex()
        .items_center()
        .min_w_0()
        .gap(ui_style.spacing.xs)
        .rounded_sm()
        .hover(|style| style.bg(theme.hover_surface))
        .child(agent_type_icon(type_id, "child", theme))
        .child(
            div()
                .id(label_id)
                .min_w_0()
                .flex_1()
                .text_xs()
                .text_color(theme.text_subtle)
                .truncate()
                .tooltip(move |window, cx| Tooltip::new(tooltip_label.clone()).build(window, cx))
                .child(label),
        )
        .child(agent_state_icon(state_id, child.state, theme))
}

fn project_sidebar_agent_item<H>(
    project_index: usize,
    agent_index: usize,
    agent: ProjectSidebarAgentItem,
    theme: WorkbenchTheme,
    ui_style: UiStyle,
    on_select_agent: H,
) -> impl IntoElement
where
    H: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
{
    let ProjectSidebarAgentItem {
        name,
        provider_id,
        state,
        task,
        action,
        children,
        ..
    } = agent;
    let label: SharedString = compact_agent_label(name, &provider_id, task, action, false).into();
    let tooltip_label = label.clone();
    let row_id = format!("project-sidebar-agent-{project_index}-{agent_index}");
    let state_id = format!("{row_id}-state").into();
    let type_id = format!("{row_id}-type").into();
    let label_id: SharedString = format!("{row_id}-label").into();
    div()
        .w_full()
        .id(SharedString::from(row_id.clone()))
        .cursor_pointer()
        .on_click(on_select_agent)
        .child(
            div()
                .ml(gpui::px(18.0))
                .mr(ui_style.spacing.xs)
                .mb(gpui::px(3.0))
                .h(gpui::px(26.0))
                .px(ui_style.spacing.sm)
                .flex()
                .items_center()
                .min_w_0()
                .gap(ui_style.spacing.xs)
                .rounded_sm()
                .hover(|style| style.bg(theme.hover_surface))
                .child(agent_type_icon(type_id, &provider_id, theme))
                .child(
                    div()
                        .id(label_id)
                        .min_w_0()
                        .flex_1()
                        .text_xs()
                        .text_color(theme.text_muted)
                        .truncate()
                        .tooltip(move |window, cx| {
                            Tooltip::new(tooltip_label.clone()).build(window, cx)
                        })
                        .child(label),
                )
                .child(agent_state_icon(state_id, state, theme)),
        )
        .children(
            children
                .into_iter()
                .enumerate()
                .map(move |(child_index, child)| {
                    child_agent_row(
                        format!("{row_id}-child-{child_index}"),
                        child,
                        theme,
                        ui_style,
                    )
                }),
        )
}
fn project_sidebar_item<H, T, C>(
    index: usize,
    item: ProjectSidebarItem,
    suffix: String,
    collapsed: bool,
    expanded: bool,
    theme: WorkbenchTheme,
    ui_style: UiStyle,
    has_keyboard_focus: bool,
    text: UiText,
    action_context: FocusHandle,
    on_select_project: H,
    on_toggle_project: T,
    on_context_project: C,
) -> impl IntoElement
where
    T: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    H: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    C: Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
{
    let row_style = yttt_row_style(YtttRowKind::Sidebar, item.state, true, theme, ui_style);
    let focused_selection = item.state == SelectableState::Active && has_keyboard_focus;
    let compact_agent_state = (!expanded).then_some(item.agent_state).flatten();
    let show_trailing = !collapsed && (!suffix.is_empty() || compact_agent_state.is_some());

    yttt_row(YtttRowKind::Sidebar, item.state, true, theme, ui_style)
        .id(("project-sidebar-item", index))
        .mb(gpui::px(3.0))
        .flex()
        .relative()
        .items_center()
        .justify_between()
        .gap(ui_style.spacing.md)
        .on_click(on_select_project)
        .on_mouse_down(MouseButton::Right, on_context_project)
        .children(focused_selection.then(|| {
            div()
                .debug_selector(|| "project-sidebar-focus-indicator".to_string())
                .absolute()
                .left(gpui::px(2.0))
                .top(gpui::px(6.0))
                .bottom(gpui::px(6.0))
                .w(gpui::px(2.0))
                .rounded_full()
                .bg(theme.accent)
        }))
        .child(
            div()
                .flex()
                .items_center()
                .gap(ui_style.spacing.md)
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
                    div()
                        .id(("project-sidebar-disclosure", index))
                        .flex_none()
                        .flex()
                        .items_center()
                        .justify_center()
                        .size(gpui::px(16.0))
                        .rounded_sm()
                        .hover(|style| style.bg(theme.hover_surface))
                        .on_click(on_toggle_project)
                        .child(
                            Icon::new(if expanded {
                                IconName::ChevronDown
                            } else {
                                IconName::ChevronRight
                            })
                            .size_3()
                            .text_color(row_style.subtitle),
                        )
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
        .children(show_trailing.then(|| {
            div()
                .flex_none()
                .flex()
                .items_center()
                .gap(ui_style.spacing.xs)
                .children((!suffix.is_empty()).then(|| {
                    div()
                        .text_xs()
                        .text_color(row_style.status)
                        .truncate()
                        .child(suffix)
                }))
                .children(compact_agent_state.map(|state| {
                    agent_state_icon(
                        format!("project-sidebar-project-state-{index}").into(),
                        state,
                        theme,
                    )
                }))
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

#[cfg(test)]
mod tests {
    use super::compact_agent_label;

    #[test]
    fn agent_label_uses_task_and_action_without_repeating_logo_identity() {
        assert_eq!(
            compact_agent_label(
                "Oh My Pi".to_string(),
                "omp",
                "Implement provider hooks".to_string(),
                Some("Read: src/runtime/agent.rs".to_string()),
                false,
            ),
            "Implement provider hooks · Read: src/runtime/agent.rs"
        );
    }

    #[test]
    fn agent_label_keeps_identity_without_a_distinct_task() {
        assert_eq!(
            compact_agent_label("OMP".to_string(), "omp", "omp".to_string(), None, false),
            "OMP"
        );
    }

    #[test]
    fn child_agent_label_keeps_identity_because_its_logo_is_generic() {
        assert_eq!(
            compact_agent_label(
                "Reviewer".to_string(),
                "child",
                "Review runtime mapping".to_string(),
                Some("Review runtime mapping".to_string()),
                true,
            ),
            "Reviewer — Review runtime mapping"
        );
    }
}

fn compact_path(path: &str) -> String {
    path.rsplit(['/', '\\'])
        .find(|part| !part.is_empty())
        .unwrap_or(path)
        .to_string()
}
