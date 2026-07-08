use gpui::{
    App, ClickEvent, InteractiveElement as _, IntoElement, Pixels, Rgba,
    StatefulInteractiveElement as _, Window, div, prelude::*, px, rgba,
};
use gpui_component::{Icon, IconName};

use crate::{
    model::workspace::{TabStartState, Workspace},
    ui::{
        agent_status::{agent_status_label, tab_agent_status},
        components::SelectableState,
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
    ProjectTabsStyle {
        height: px(32.0),
        item_height: px(32.0),
        border_width: px(1.0),
        active_background: theme.surface,
        inactive_background: theme.app_background,
        hover_background: theme.hover_surface,
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
            style,
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
    style: ProjectTabsStyle,
    theme: WorkbenchTheme,
    on_select_tab: SelectH,
    on_close_tab: CloseH,
) -> impl IntoElement
where
    SelectH: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    CloseH: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
{
    let is_active = item.state == SelectableState::Active;
    let background = if is_active {
        style.active_background
    } else {
        style.inactive_background
    };
    let text_color = if is_active {
        theme.text
    } else {
        theme.text_muted
    };
    let group_name = format!("project-tab-{}", item.id);

    div()
        .id(("project-tab", index))
        .group(group_name.clone())
        .flex()
        .items_center()
        .gap_2()
        .h(style.item_height)
        .min_w(px(128.0))
        .max_w(px(220.0))
        .border_r_1()
        .border_color(theme.border)
        .bg(background)
        .px_2()
        .text_xs()
        .hover(move |this| this.bg(style.hover_background))
        .on_click(on_select_tab)
        .child(
            Icon::new(IconName::SquareTerminal)
                .size_3()
                .text_color(theme.text_subtle),
        )
        .child(
            div()
                .flex_1()
                .truncate()
                .text_color(text_color)
                .child(item.title),
        )
        .child(tab_status_dot(item.status_tone, theme))
        .child(tab_close_button(index, group_name, theme, on_close_tab))
}

fn tab_status_dot(tone: ProjectTabStatusTone, theme: WorkbenchTheme) -> impl IntoElement {
    let color = match tone {
        ProjectTabStatusTone::Lazy => theme.text_subtle,
        ProjectTabStatusTone::Started => theme.success,
        ProjectTabStatusTone::AgentRunning => theme.accent,
        ProjectTabStatusTone::AgentCompleted => theme.success,
        ProjectTabStatusTone::AgentFailed => theme.danger,
    };

    div().flex_none().w_1p5().h_1p5().rounded_full().bg(color)
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
    div()
        .id(("project-tab-close", index))
        .flex()
        .items_center()
        .justify_center()
        .size_4()
        .rounded_sm()
        .text_color(theme.text_subtle)
        .invisible()
        .group_hover(group_name, |this| this.visible())
        .hover(move |this| this.bg(theme.hover_surface).text_color(theme.text))
        .on_click(on_close_tab)
        .child(Icon::new(IconName::Close).size_3())
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
    div()
        .id(id)
        .flex()
        .items_center()
        .justify_center()
        .size_7()
        .border_l_1()
        .border_color(theme.border)
        .text_color(theme.text_muted)
        .hover(move |this| this.bg(theme.hover_surface).text_color(theme.text))
        .on_click(on_click)
        .child(Icon::new(icon).size_3())
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
