use gpui::{App, ClickEvent, IntoElement, Window, div, prelude::*};
use gpui_component::{Icon, IconName, StyledExt, TitleBar, tooltip::Tooltip};

use crate::ui::{
    components::workbench_icon_button,
    primitives::icon_button::YtttIconButtonKind,
    theme::{UiStyle, WorkbenchTheme},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TitlebarInfo {
    pub project_name: String,
    pub compact_path: Option<String>,
    pub git_branch: Option<String>,
    pub git_counters: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TitlebarMetricInfo {
    pub value: String,
    pub tooltip: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TitlebarApplicationPerformanceInfo {
    pub projects: TitlebarMetricInfo,
    pub terminals: TitlebarMetricInfo,
    pub tabs: TitlebarMetricInfo,
    pub editors: TitlebarMetricInfo,
    pub cpu: TitlebarMetricInfo,
    pub memory: TitlebarMetricInfo,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TitlebarSystemPerformanceInfo {
    pub cpu: TitlebarMetricInfo,
    pub memory: TitlebarMetricInfo,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TitlebarPerformanceInfo {
    pub application: Option<TitlebarApplicationPerformanceInfo>,
    pub system: Option<TitlebarSystemPerformanceInfo>,
}

impl TitlebarInfo {
    pub fn parts(&self) -> Vec<String> {
        let mut parts = vec![self.project_name.clone()];
        if let Some(path) = &self.compact_path {
            parts.push(path.clone());
        }
        if let Some(branch) = &self.git_branch {
            parts.push(branch.clone());
        }
        if let Some(counters) = &self.git_counters {
            parts.push(counters.clone());
        }
        parts
    }
}

pub fn compact_path_for_titlebar(path: &str) -> String {
    const MAX_LEN: usize = 48;
    let path = if let Some(path) = path.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{path}")
    } else {
        path.strip_prefix(r"\\?\").unwrap_or(path).to_string()
    };
    if path.chars().count() <= MAX_LEN {
        return path;
    }

    let separator = if path.rfind('\\') > path.rfind('/') {
        '\\'
    } else {
        '/'
    };
    let mut parts = path.rsplit(['/', '\\']).filter(|part| !part.is_empty());
    let tail = parts.next().unwrap_or(&path);
    let parent = parts.next();

    match parent {
        Some(parent) => format!("...{separator}{parent}{separator}{tail}"),
        None => format!("...{separator}{tail}"),
    }
}

pub fn workbench_titlebar<BranchH, DiffH, CommandH, SettingsH>(
    info: TitlebarInfo,
    performance: Option<TitlebarPerformanceInfo>,
    theme: WorkbenchTheme,
    ui_style: UiStyle,
    command_tooltip: &'static str,
    settings_tooltip: &'static str,
    on_branch_click: BranchH,
    on_diff_click: DiffH,
    on_command_click: CommandH,
    on_settings_click: SettingsH,
) -> impl IntoElement
where
    BranchH: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    DiffH: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    CommandH: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    SettingsH: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
{
    TitleBar::new()
        .bg(theme.titlebar_background)
        .border_color(theme.border)
        .child(
            div()
                .flex()
                .items_center()
                .gap(ui_style.spacing.md)
                .size_full()
                .px(ui_style.spacing.lg)
                .text_sm()
                .text_color(theme.text)
                .child(div().font_semibold().child(info.project_name))
                .children(info.compact_path.map(|path| {
                    div()
                        .flex()
                        .items_center()
                        .gap(ui_style.spacing.md)
                        .child(titlebar_separator(theme))
                        .child(titlebar_meta(path, theme))
                }))
                .children(info.git_branch.map(|branch| {
                    div()
                        .flex()
                        .items_center()
                        .gap(ui_style.spacing.md)
                        .child(titlebar_separator(theme))
                        .child(
                            div()
                                .id("titlebar-git-branch")
                                .debug_selector(|| "titlebar-git-branch".to_string())
                                .cursor_pointer()
                                .occlude()
                                .rounded(ui_style.radius.compact)
                                .px(ui_style.spacing.xs)
                                .text_xs()
                                .text_color(theme.text_muted)
                                .hover(move |this| {
                                    this.bg(ui_style.hover_background(theme))
                                        .text_color(theme.text)
                                })
                                .on_click(on_branch_click)
                                .child(format!("⎇ {branch}")),
                        )
                }))
                .children(info.git_counters.map(|counters| {
                    div()
                        .flex()
                        .items_center()
                        .gap(ui_style.spacing.md)
                        .child(titlebar_separator(theme))
                        .child(
                            div()
                                .id("titlebar-git-changes")
                                .debug_selector(|| "titlebar-git-changes".to_string())
                                .cursor_pointer()
                                .occlude()
                                .rounded(ui_style.radius.compact)
                                .border(ui_style.border.hairline)
                                .border_color(theme.border)
                                .px(ui_style.spacing.xs)
                                .text_xs()
                                .text_color(theme.text_muted)
                                .hover(move |this| {
                                    this.bg(ui_style.hover_background(theme))
                                        .border_color(theme.accent)
                                })
                                .on_click(on_diff_click)
                                .child(counters),
                        )
                }))
                .child(div().flex_1())
                .children(
                    performance
                        .map(|metrics| titlebar_performance_metrics(metrics, theme, ui_style)),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .h_full()
                        .occlude()
                        .child(
                            workbench_icon_button(
                                "titlebar-command-palette",
                                IconName::Search,
                                YtttIconButtonKind::Toolbar,
                                theme,
                                ui_style,
                                on_command_click,
                            )
                            .debug_selector(|| "titlebar-command-palette".to_string())
                            .tooltip(move |window, cx| {
                                Tooltip::new(command_tooltip).build(window, cx)
                            }),
                        )
                        .child(
                            workbench_icon_button(
                                "titlebar-settings",
                                IconName::Settings,
                                YtttIconButtonKind::Toolbar,
                                theme,
                                ui_style,
                                on_settings_click,
                            )
                            .debug_selector(|| "titlebar-settings".to_string())
                            .tooltip(move |window, cx| {
                                Tooltip::new(settings_tooltip).build(window, cx)
                            }),
                        ),
                ),
        )
}

fn titlebar_performance_metrics(
    metrics: TitlebarPerformanceInfo,
    theme: WorkbenchTheme,
    ui_style: UiStyle,
) -> impl IntoElement {
    div()
        .id("titlebar-performance-metrics")
        .debug_selector(|| "titlebar-performance-metrics".to_string())
        .flex()
        .items_center()
        .gap(ui_style.spacing.md)
        .h_full()
        .px(ui_style.spacing.md)
        .mr(ui_style.spacing.xs)
        .border_l(ui_style.border.hairline)
        .border_color(theme.border)
        .children(
            metrics
                .application
                .map(|metrics| titlebar_application_performance_metrics(metrics, theme, ui_style)),
        )
        .children(
            metrics
                .system
                .map(|metrics| titlebar_system_performance_metrics(metrics, theme, ui_style)),
        )
}

fn titlebar_application_performance_metrics(
    metrics: TitlebarApplicationPerformanceInfo,
    theme: WorkbenchTheme,
    ui_style: UiStyle,
) -> impl IntoElement {
    div()
        .id("titlebar-application-performance-metrics")
        .debug_selector(|| "titlebar-application-performance-metrics".to_string())
        .flex()
        .items_center()
        .gap(ui_style.spacing.md)
        .child(titlebar_performance_metric(
            "titlebar-performance-projects",
            IconName::Folder,
            metrics.projects,
            theme,
            ui_style,
        ))
        .child(titlebar_performance_metric(
            "titlebar-performance-terminals",
            IconName::SquareTerminal,
            metrics.terminals,
            theme,
            ui_style,
        ))
        .child(titlebar_performance_metric(
            "titlebar-performance-tabs",
            IconName::GalleryVerticalEnd,
            metrics.tabs,
            theme,
            ui_style,
        ))
        .child(titlebar_performance_metric(
            "titlebar-performance-editors",
            IconName::File,
            metrics.editors,
            theme,
            ui_style,
        ))
        .child(titlebar_performance_metric(
            "titlebar-performance-cpu",
            IconName::Cpu,
            metrics.cpu,
            theme,
            ui_style,
        ))
        .child(titlebar_performance_metric(
            "titlebar-performance-memory",
            IconName::MemoryStick,
            metrics.memory,
            theme,
            ui_style,
        ))
}

fn titlebar_system_performance_metrics(
    metrics: TitlebarSystemPerformanceInfo,
    theme: WorkbenchTheme,
    ui_style: UiStyle,
) -> impl IntoElement {
    div()
        .id("titlebar-system-performance-metrics")
        .debug_selector(|| "titlebar-system-performance-metrics".to_string())
        .flex()
        .items_center()
        .gap(ui_style.spacing.md)
        .child(
            Icon::new(IconName::Globe)
                .size_3()
                .text_color(theme.text_subtle),
        )
        .child(titlebar_performance_metric(
            "titlebar-system-cpu",
            IconName::Cpu,
            metrics.cpu,
            theme,
            ui_style,
        ))
        .child(titlebar_performance_metric(
            "titlebar-system-memory",
            IconName::MemoryStick,
            metrics.memory,
            theme,
            ui_style,
        ))
}

fn titlebar_performance_metric(
    id: &'static str,
    icon: IconName,
    metric: TitlebarMetricInfo,
    theme: WorkbenchTheme,
    ui_style: UiStyle,
) -> impl IntoElement {
    let tooltip = metric.tooltip;
    div()
        .id(id)
        .debug_selector(move || id.to_string())
        .flex()
        .items_center()
        .gap(ui_style.spacing.xs)
        .whitespace_nowrap()
        .text_xs()
        .text_color(theme.text_muted)
        .child(Icon::new(icon).size_3().text_color(theme.text_subtle))
        .child(metric.value)
        .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
}

fn titlebar_meta(value: String, theme: WorkbenchTheme) -> impl IntoElement {
    div()
        .truncate()
        .text_xs()
        .text_color(theme.text_muted)
        .child(value)
}

fn titlebar_separator(theme: WorkbenchTheme) -> impl IntoElement {
    div().text_xs().text_color(theme.text_subtle).child("—")
}
