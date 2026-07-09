use gpui::{
    App, ClickEvent, InteractiveElement as _, IntoElement, Pixels, Rgba,
    StatefulInteractiveElement as _, Window, div, prelude::*, px, rgba,
};
use gpui_component::{Icon, IconName};

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
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProjectTabCloseButtonVisibility {
    Hover,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProjectTabLeadingIcon {
    Terminal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProjectTabStatusIndicator {
    Dot,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProjectTabStatusTone {
    Lazy,
    Started,
    AgentRunning,
    AgentCompleted,
    AgentFailed,
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
        leading_icon: ProjectTabLeadingIcon::Terminal,
        status_indicator: ProjectTabStatusIndicator::Dot,
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

pub fn project_tabs<SelectH, SelectF, CloseH, CloseF, NewH, SplitVH, SplitHH>(
    workspace: &Workspace,
    theme: WorkbenchTheme,
    mut on_select_tab: SelectF,
    mut on_close_tab: CloseF,
    on_new_tab: NewH,
    on_split_vertical: SplitVH,
    on_split_horizontal: SplitHH,
) -> impl IntoElement
where
    SelectH: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    SelectF: FnMut(String) -> SelectH,
    CloseH: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    CloseF: FnMut(String) -> CloseH,
    NewH: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    SplitVH: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    SplitHH: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
{
    let style = project_tabs_style(theme);
    let items = visible_tab_items(workspace);
    if items.is_empty() {
        return div().into_any_element();
    }

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
        ))
        .into_any_element()
}

fn project_tab<SelectH, CloseH>(
    index: usize,
    item: ProjectTabItem,
    theme: WorkbenchTheme,
    on_select_tab: SelectH,
    on_close_tab: CloseH,
) -> impl IntoElement
where
    SelectH: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    CloseH: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
{
    let row_style = yttt_row_style(YtttRowKind::Tab, item.state, true, theme);
    let group_name = format!("project-tab-{}", item.id);

    div()
        .id(("project-tab", index))
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
        .child(
            Icon::new(IconName::SquareTerminal)
                .size_3()
                .text_color(row_style.subtitle),
        )
        .child(
            div()
                .flex_1()
                .truncate()
                .text_color(row_style.title)
                .child(item.title),
        )
        .child(tab_status_dot(item.status_tone, theme))
        .child(tab_close_button(index, group_name, theme, on_close_tab))
}

fn tab_status_dot(tone: ProjectTabStatusTone, theme: WorkbenchTheme) -> impl IntoElement {
    let tone = match tone {
        ProjectTabStatusTone::Lazy => YtttStatusTone::Neutral,
        ProjectTabStatusTone::Started => YtttStatusTone::Success,
        ProjectTabStatusTone::AgentRunning => YtttStatusTone::Running,
        ProjectTabStatusTone::AgentCompleted => YtttStatusTone::Success,
        ProjectTabStatusTone::AgentFailed => YtttStatusTone::Error,
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
) -> impl IntoElement
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

fn tab_toolbar<NewH, SplitVH, SplitHH>(
    theme: WorkbenchTheme,
    on_new_tab: NewH,
    on_split_vertical: SplitVH,
    on_split_horizontal: SplitHH,
) -> impl IntoElement
where
    NewH: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    SplitVH: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    SplitHH: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
{
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
) -> impl IntoElement
where
    H: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
{
    workbench_icon_button(id, icon, YtttIconButtonKind::Toolbar, theme, on_click)
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
