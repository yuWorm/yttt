use gpui::{
    App, ClickEvent, Div, InteractiveElement as _, IntoElement, Pixels, Rgba, Stateful,
    StatefulInteractiveElement as _, Window, div, prelude::*, px, rgba,
};
use gpui_component::{Icon, IconName, tooltip::Tooltip};

use crate::ui::editor::{DocumentId, WorkItemId};
use crate::{
    model::workspace::{TabStartState, Workspace},
    ui::{
        agent_status::{agent_status_label, tab_agent_status},
        components::{SelectableState, workbench_icon_button},
        primitives::{
            icon_button::YtttIconButtonKind,
            row::{YtttRowKind, yttt_row_style},
            status::{YtttStatusTone, yttt_status_dot_style},
            tabs::yttt_tabbar_style,
        },
        theme::WorkbenchTheme,
    },
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectTabItem {
    pub id: String,
    pub title: String,
    pub status: Option<String>,
    pub status_tone: ProjectTabStatusTone,
    pub state: SelectableState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkbenchTabKind {
    Terminal,
    File,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileTabSnapshot {
    pub id: DocumentId,
    pub relative_path: std::path::PathBuf,
    pub dirty: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkbenchTabItem {
    pub id: WorkItemId,
    pub kind: WorkbenchTabKind,
    pub title: String,
    pub tooltip: String,
    pub status: Option<String>,
    pub status_tone: Option<ProjectTabStatusTone>,
    pub dirty: bool,
    pub state: SelectableState,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ProjectTabsStyle {
    pub height: Pixels,
    pub item_height: Pixels,
    pub border_width: Pixels,
    pub active_background: Rgba,
    pub inactive_background: Rgba,
    pub hover_background: Rgba,
    pub close_button_visibility: ProjectTabCloseButtonVisibility,
    pub leading_icon: ProjectTabLeadingIcon,
    pub status_indicator: ProjectTabStatusIndicator,
    pub dirty_marker_uses_close_slot: bool,
    pub toolbar_placement: ProjectTabToolbarPlacement,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProjectTabCloseButtonVisibility {
    Hover,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProjectTabLeadingIcon {
    Terminal,
    PerItem,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProjectTabStatusIndicator {
    Dot,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProjectTabToolbarPlacement {
    FixedAfterScrollableTabs,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProjectTabStatusTone {
    Lazy,
    Started,
    AgentRunning,
    AgentCompleted,
    AgentFailed,
    Dirty,
}

pub fn project_tabs_style(theme: WorkbenchTheme) -> ProjectTabsStyle {
    let primitive = yttt_tabbar_style(theme);
    ProjectTabsStyle {
        height: primitive.height,
        item_height: primitive.item_height,
        border_width: primitive.border_width,
        active_background: primitive.active_background,
        inactive_background: primitive.inactive_background,
        hover_background: primitive.hover_background,
        close_button_visibility: ProjectTabCloseButtonVisibility::Hover,
        leading_icon: ProjectTabLeadingIcon::PerItem,
        status_indicator: ProjectTabStatusIndicator::Dot,
        dirty_marker_uses_close_slot: true,
        toolbar_placement: ProjectTabToolbarPlacement::FixedAfterScrollableTabs,
    }
}

pub fn visible_tab_titles(workspace: &Workspace) -> Vec<String> {
    visible_tab_items(workspace)
        .into_iter()
        .map(|tab| tab.title)
        .collect()
}

pub fn visible_tab_items(workspace: &Workspace) -> Vec<ProjectTabItem> {
    let Some(selected_project_id) = workspace.selected_project_id() else {
        return Vec::new();
    };
    let Some(project) = workspace.project(selected_project_id) else {
        return Vec::new();
    };

    project
        .layout
        .tabs
        .iter()
        .map(|tab| ProjectTabItem {
            id: tab.id.clone(),
            title: tab.title.clone(),
            status: project.tab_state(&tab.id).map(|state| {
                let mut parts = vec![tab_start_state_label(state.start_state).to_string()];
                if let Some(agent_status) = tab_agent_status(project, &tab.id) {
                    parts.push(agent_status_label(agent_status).to_string());
                }
                parts.join(" · ")
            }),
            status_tone: project
                .tab_state(&tab.id)
                .map(|state| tab_status_tone(state.start_state, tab_agent_status(project, &tab.id)))
                .unwrap_or(ProjectTabStatusTone::Lazy),
            state: if tab.id == project.selected_tab_id {
                SelectableState::Active
            } else {
                SelectableState::Inactive
            },
        })
        .collect()
}

pub fn visible_work_item_tabs(
    terminal_items: &[ProjectTabItem],
    file_items: &[FileTabSnapshot],
    active_work_item: Option<&WorkItemId>,
) -> Vec<WorkbenchTabItem> {
    terminal_items
        .iter()
        .map(|item| {
            let id = WorkItemId::Terminal(item.id.clone());
            WorkbenchTabItem {
                state: if active_work_item == Some(&id) {
                    SelectableState::Active
                } else {
                    SelectableState::Inactive
                },
                id,
                kind: WorkbenchTabKind::Terminal,
                title: item.title.clone(),
                tooltip: item.status.clone().unwrap_or_else(|| item.title.clone()),
                status: item.status.clone(),
                status_tone: Some(item.status_tone),
                dirty: false,
            }
        })
        .chain(file_items.iter().map(|file| {
            let id = WorkItemId::File(file.id.clone());
            WorkbenchTabItem {
                state: if active_work_item == Some(&id) {
                    SelectableState::Active
                } else {
                    SelectableState::Inactive
                },
                id,
                kind: WorkbenchTabKind::File,
                title: file
                    .relative_path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| file.relative_path.to_string_lossy().into_owned()),
                tooltip: file.relative_path.to_string_lossy().into_owned(),
                status: None,
                status_tone: file.dirty.then_some(ProjectTabStatusTone::Dirty),
                dirty: file.dirty,
            }
        }))
        .collect()
}

pub fn visible_terminal_work_item_tabs(workspace: &Workspace) -> Vec<WorkbenchTabItem> {
    let terminal_items = visible_tab_items(workspace);
    let active = workspace
        .selected_project_id()
        .and_then(|project_id| workspace.project(project_id))
        .map(|project| WorkItemId::Terminal(project.selected_tab_id.clone()));
    visible_work_item_tabs(&terminal_items, &[], active.as_ref())
}

pub fn project_tree_toggle_icon(open: bool) -> IconName {
    if open {
        IconName::FolderOpen
    } else {
        IconName::FolderClosed
    }
}

pub fn project_tree_toggle_tooltip(open: bool) -> &'static str {
    if open { "Hide Files" } else { "Show Files" }
}

pub fn project_tabs<SelectH, SelectF, CloseH, CloseF, NewH, SplitVH, SplitHH, ToggleTreeH>(
    items: Vec<WorkbenchTabItem>,
    theme: WorkbenchTheme,
    project_tree_open: bool,
    mut on_select_tab: SelectF,
    mut on_close_tab: CloseF,
    on_new_tab: NewH,
    on_split_vertical: SplitVH,
    on_split_horizontal: SplitHH,
    on_toggle_project_tree: ToggleTreeH,
) -> impl IntoElement
where
    SelectH: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    SelectF: FnMut(WorkItemId) -> SelectH,
    CloseH: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    CloseF: FnMut(WorkItemId) -> CloseH,
    NewH: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    SplitVH: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    SplitHH: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ToggleTreeH: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
{
    let style = project_tabs_style(theme);

    let mut tab_row = div()
        .id("project-tab-row")
        .flex()
        .items_center()
        .h_full()
        .overflow_x_scroll();
    for (index, item) in items.into_iter().enumerate() {
        let select_tab_id = item.id.clone();
        let close_tab_id = item.id.clone();
        tab_row = tab_row.child(project_tab(
            index,
            item,
            theme,
            on_select_tab(select_tab_id),
            on_close_tab(close_tab_id),
        ));
    }

    div()
        .flex()
        .items_center()
        .justify_between()
        .h(style.height)
        .bg(theme.tabbar_background)
        .border_b_1()
        .border_color(theme.border)
        .child(tab_row.flex_1())
        .child(tab_toolbar(
            theme,
            on_new_tab,
            on_split_vertical,
            on_split_horizontal,
            project_tree_open,
            on_toggle_project_tree,
        ))
        .into_any_element()
}

fn project_tab<SelectH, CloseH>(
    index: usize,
    item: WorkbenchTabItem,
    theme: WorkbenchTheme,
    on_select_tab: SelectH,
    on_close_tab: CloseH,
) -> impl IntoElement
where
    SelectH: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    CloseH: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
{
    let row_style = yttt_row_style(YtttRowKind::Tab, item.state, true, theme);
    let group_name = format!("project-tab-{index}");
    let tooltip = item.tooltip.clone();
    let kind = item.kind;
    let dirty = item.dirty;
    let status_tone = item.status_tone;

    let mut tab = div()
        .id(("project-tab", index))
        .debug_selector(move || format!("project-tab-{index}"))
        .group(group_name.clone())
        .flex()
        .items_center()
        .gap_2()
        .h(row_style.height)
        .min_w(px(128.0))
        .max_w(px(220.0))
        .border_r(row_style.border_width)
        .border_color(row_style.border)
        .bg(row_style.background)
        .px(row_style.padding_x)
        .text_xs()
        .hover(move |this| this.bg(row_style.hover_background))
        .on_click(on_select_tab)
        .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
        .child(
            Icon::new(match kind {
                WorkbenchTabKind::Terminal => IconName::SquareTerminal,
                WorkbenchTabKind::File => IconName::File,
            })
            .size_3()
            .text_color(row_style.subtitle),
        )
        .child(
            div()
                .flex_1()
                .truncate()
                .text_color(row_style.title)
                .child(item.title),
        );

    tab = match kind {
        WorkbenchTabKind::Terminal => tab
            .children(status_tone.map(|tone| tab_status_dot(tone, theme)))
            .child(tab_close_button(index, group_name, theme, on_close_tab)),
        WorkbenchTabKind::File => tab.child(file_tab_close_slot(
            index,
            group_name,
            dirty,
            theme,
            on_close_tab,
        )),
    };

    tab
}

fn tab_status_dot(tone: ProjectTabStatusTone, theme: WorkbenchTheme) -> impl IntoElement {
    let tone = match tone {
        ProjectTabStatusTone::Lazy => YtttStatusTone::Neutral,
        ProjectTabStatusTone::Started => YtttStatusTone::Success,
        ProjectTabStatusTone::AgentRunning => YtttStatusTone::Running,
        ProjectTabStatusTone::AgentCompleted => YtttStatusTone::Success,
        ProjectTabStatusTone::AgentFailed => YtttStatusTone::Error,
        ProjectTabStatusTone::Dirty => YtttStatusTone::Warning,
    };
    let style = yttt_status_dot_style(tone, theme);

    div()
        .flex_none()
        .w(style.size)
        .h(style.size)
        .rounded_full()
        .bg(style.color)
}

fn tab_close_button<CloseH>(
    index: usize,
    group_name: String,
    theme: WorkbenchTheme,
    on_close_tab: CloseH,
) -> Stateful<Div>
where
    CloseH: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
{
    workbench_icon_button(
        ("project-tab-close", index),
        IconName::Close,
        YtttIconButtonKind::TabClose,
        theme,
        on_close_tab,
    )
    .invisible()
    .group_hover(group_name, |this| this.visible())
}

fn file_tab_close_slot<CloseH>(
    index: usize,
    group_name: String,
    dirty: bool,
    theme: WorkbenchTheme,
    on_close_tab: CloseH,
) -> impl IntoElement
where
    CloseH: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
{
    let dirty_style = yttt_status_dot_style(YtttStatusTone::Warning, theme);
    div()
        .relative()
        .flex()
        .items_center()
        .justify_center()
        .flex_none()
        .size(px(24.0))
        .children(dirty.then(|| {
            div()
                .size(dirty_style.size)
                .rounded_full()
                .bg(dirty_style.color)
                .group_hover(group_name.clone(), |this| this.invisible())
        }))
        .child(
            tab_close_button(index, group_name, theme, on_close_tab)
                .absolute()
                .top_0()
                .left_0(),
        )
}

fn tab_toolbar<NewH, SplitVH, SplitHH, ToggleTreeH>(
    theme: WorkbenchTheme,
    on_new_tab: NewH,
    on_split_vertical: SplitVH,
    on_split_horizontal: SplitHH,
    project_tree_open: bool,
    on_toggle_project_tree: ToggleTreeH,
) -> impl IntoElement
where
    NewH: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    SplitVH: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    SplitHH: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ToggleTreeH: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
{
    let tooltip = project_tree_toggle_tooltip(project_tree_open);
    div()
        .flex()
        .items_center()
        .h_full()
        .border_l_1()
        .border_color(theme.border)
        .bg(rgba(0x00000000))
        .child(tab_toolbar_button(
            "tab-new",
            IconName::Plus,
            theme,
            on_new_tab,
        ))
        .child(tab_toolbar_button(
            "pane-split-vertical",
            tab_toolbar_icon(crate::model::layout::SplitDirection::Vertical),
            theme,
            on_split_vertical,
        ))
        .child(tab_toolbar_button(
            "pane-split-horizontal",
            tab_toolbar_icon(crate::model::layout::SplitDirection::Horizontal),
            theme,
            on_split_horizontal,
        ))
        .child(
            tab_toolbar_button(
                "project-tree-toggle",
                project_tree_toggle_icon(project_tree_open),
                theme,
                on_toggle_project_tree,
            )
            .when(project_tree_open, |this| {
                this.bg(theme.active_surface).text_color(theme.text)
            })
            .tooltip(move |window, cx| Tooltip::new(tooltip).build(window, cx)),
        )
}

pub fn tab_toolbar_icon(direction: crate::model::layout::SplitDirection) -> IconName {
    match direction {
        crate::model::layout::SplitDirection::Vertical => IconName::PanelBottom,
        crate::model::layout::SplitDirection::Horizontal => IconName::PanelRight,
    }
}

fn tab_toolbar_button<H>(
    id: &'static str,
    icon: IconName,
    theme: WorkbenchTheme,
    on_click: H,
) -> Stateful<Div>
where
    H: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
{
    workbench_icon_button(id, icon, YtttIconButtonKind::Toolbar, theme, on_click)
        .debug_selector(|| id.to_string())
}

fn tab_start_state_label(state: TabStartState) -> &'static str {
    match state {
        TabStartState::Lazy => "lazy",
        TabStartState::Started => "started",
    }
}

fn tab_status_tone(
    state: TabStartState,
    agent_status: Option<crate::model::workspace::AgentStatus>,
) -> ProjectTabStatusTone {
    match agent_status {
        Some(crate::model::workspace::AgentStatus::Running) => ProjectTabStatusTone::AgentRunning,
        Some(crate::model::workspace::AgentStatus::Completed) => {
            ProjectTabStatusTone::AgentCompleted
        }
        Some(crate::model::workspace::AgentStatus::Failed) => ProjectTabStatusTone::AgentFailed,
        None => match state {
            TabStartState::Lazy => ProjectTabStatusTone::Lazy,
            TabStartState::Started => ProjectTabStatusTone::Started,
        },
    }
}
