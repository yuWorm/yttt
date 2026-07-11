use gpui::{App, ClickEvent, IntoElement, Window, div, prelude::*, rgb};
use gpui_component::{StyledExt, TitleBar};

use crate::ui::theme::WorkbenchTheme;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TitlebarInfo {
    pub project_name: String,
    pub compact_path: Option<String>,
    pub git_branch: Option<String>,
    pub git_counters: Option<String>,
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
    if path.chars().count() <= MAX_LEN {
        return path.to_string();
    }

    let mut parts = path.rsplit('/').filter(|part| !part.is_empty());
    let tail = parts.next().unwrap_or(path);
    let parent = parts.next();

    match parent {
        Some(parent) => format!(".../{parent}/{tail}"),
        None => format!(".../{tail}"),
    }
}

pub fn workbench_titlebar<BranchH, DiffH>(
    info: TitlebarInfo,
    theme: WorkbenchTheme,
    on_branch_click: BranchH,
    on_diff_click: DiffH,
) -> impl IntoElement
where
    BranchH: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    DiffH: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
{
    TitleBar::new()
        .bg(theme.titlebar_background)
        .border_color(theme.border)
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .size_full()
                .px_3()
                .text_sm()
                .text_color(theme.text)
                .bg(theme.titlebar_background)
                .child(div().font_semibold().child(info.project_name))
                .children(info.compact_path.map(|path| {
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(titlebar_separator(theme))
                        .child(titlebar_meta(path, theme))
                }))
                .children(info.git_branch.map(|branch| {
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(titlebar_separator(theme))
                        .child(
                            div()
                                .id("titlebar-git-branch")
                                .debug_selector(|| "titlebar-git-branch".to_string())
                                .cursor_pointer()
                                .rounded_sm()
                                .px_1()
                                .text_xs()
                                .text_color(theme.text_muted)
                                .hover(move |this| {
                                    this.bg(theme.hover_surface).text_color(theme.text)
                                })
                                .on_click(on_branch_click)
                                .child(format!("⎇ {branch}")),
                        )
                }))
                .children(info.git_counters.map(|counters| {
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(titlebar_separator(theme))
                        .child(
                            div()
                                .id("titlebar-git-changes")
                                .debug_selector(|| "titlebar-git-changes".to_string())
                                .cursor_pointer()
                                .rounded_sm()
                                .border_1()
                                .border_color(theme.border)
                                .px_1()
                                .text_xs()
                                .text_color(rgb(0xc8d3df))
                                .hover(move |this| {
                                    this.bg(theme.hover_surface).border_color(theme.accent)
                                })
                                .on_click(on_diff_click)
                                .child(counters),
                        )
                })),
        )
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
