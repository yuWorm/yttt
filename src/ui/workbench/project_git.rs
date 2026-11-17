//! The project panel's Git page: the selected project's changed files; one
//! opens the diff panel on that file.

use crate::runtime::git_status::GitFileStatus;
use yttt_ui::Icon;

use super::*;

impl WorkbenchView {
    pub(super) fn project_git_panel_content(&self, cx: &mut Context<Self>) -> Div {
        let appearance = self.theme_runtime();
        let theme = appearance.ui;
        let ui_style = appearance.style;
        let Some(project_id) = self.workspace.selected_project_id() else {
            return div().flex_1();
        };
        let Some(status) = self.project.project_git_statuses.get(project_id) else {
            return git_message(
                "project-git-unavailable",
                self.ui_text
                    .get(UiTextKey::ProjectGitUnavailable)
                    .to_string(),
                theme,
                ui_style,
            );
        };
        let changes = status
            .file_statuses()
            .iter()
            .filter(|(_, status)| **status != GitFileStatus::Ignored)
            .filter_map(|(path, status)| Some((path.to_str()?.to_string(), *status)))
            .collect::<Vec<_>>();
        let branch = status.branch.clone().unwrap_or_default();
        let open_label = self.ui_text.get(UiTextKey::ProjectGitOpenDiff);
        let header = div()
            .debug_selector(|| "project-git-header".to_string())
            .flex()
            .items_center()
            .justify_between()
            .flex_none()
            .gap(ui_style.spacing.sm)
            .px(ui_style.spacing.md)
            .pt(ui_style.spacing.md)
            .pb(ui_style.spacing.xs)
            .text_xs()
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme.text_muted)
                    .child(branch),
            )
            .child(
                div()
                    .id("project-git-open-diff")
                    .flex_none()
                    .px(ui_style.spacing.xs)
                    .rounded_sm()
                    .cursor_pointer()
                    .text_color(theme.text_subtle)
                    .hover(move |style| style.bg(theme.ghost_element_hover).text_color(theme.text))
                    .child(format!("{open_label} ({})", changes.len()))
                    .on_click(cx.listener(|view, _: &ClickEvent, _window, cx| {
                        if let Err(error) = view.open_git_diff_panel() {
                            view.load_error = Some(error.to_string());
                        }
                        cx.notify();
                    })),
            );
        if changes.is_empty() {
            return div()
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .child(header)
                .child(git_message(
                    "project-git-clean",
                    self.ui_text.get(UiTextKey::ProjectGitClean).to_string(),
                    theme,
                    ui_style,
                ));
        }
        let rows = changes
            .into_iter()
            .enumerate()
            .map(|(index, (path, status))| {
                self.git_change_row(index, path, status, theme, ui_style, cx)
            })
            .collect::<Vec<_>>();
        div()
            .debug_selector(|| "project-git".to_string())
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .child(header)
            .child(
                div()
                    .id("project-git-changes")
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .pb(ui_style.spacing.md)
                    .children(rows),
            )
    }

    fn git_change_row(
        &self,
        index: usize,
        path: String,
        status: GitFileStatus,
        theme: WorkbenchTheme,
        ui_style: UiStyle,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (directory, name) = match path.rsplit_once('/') {
            Some((directory, name)) => (directory.to_string(), name.to_string()),
            None => (String::new(), path.clone()),
        };
        let (letter, color) = match status {
            GitFileStatus::Added => ("A", theme.success),
            GitFileStatus::Modified => ("M", theme.warning),
            GitFileStatus::Deleted => ("D", theme.danger),
            GitFileStatus::Untracked => ("U", theme.success),
            GitFileStatus::Ignored => ("I", theme.text_subtle),
        };
        let deleted = status == GitFileStatus::Deleted;
        div()
            .id(SharedString::from(format!("project-git-change-{index}")))
            .flex()
            .items_center()
            .min_w_0()
            .gap(ui_style.spacing.sm)
            .mx(ui_style.spacing.xs)
            .px(ui_style.spacing.sm)
            .py(ui_style.spacing.xxs)
            .rounded_sm()
            .cursor_pointer()
            .text_sm()
            .hover(move |style| style.bg(theme.hover_surface))
            .on_click(cx.listener(move |view, _: &ClickEvent, _window, cx| {
                if let Err(error) = view.open_git_diff_panel_for_file(path.clone()) {
                    view.load_error = Some(error.to_string());
                }
                cx.notify();
            }))
            .child(
                Icon::new(IconName::File)
                    .size(px(13.0))
                    .text_color(theme.icon_muted),
            )
            .child(
                div()
                    .flex_none()
                    .max_w(relative(0.7))
                    .truncate()
                    .text_color(theme.text)
                    .when(deleted, |name| name.line_through())
                    .child(name),
            )
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .truncate()
                    .text_xs()
                    .text_color(theme.text_subtle)
                    .child(directory),
            )
            .child(
                div()
                    .flex_none()
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(color)
                    .child(letter),
            )
            .into_any_element()
    }
}

fn git_message(
    selector: &'static str,
    message: String,
    theme: WorkbenchTheme,
    ui_style: UiStyle,
) -> Div {
    div()
        .debug_selector(move || selector.to_string())
        .flex()
        .flex_1()
        .items_center()
        .justify_center()
        .px(ui_style.spacing.xl)
        .text_center()
        .text_sm()
        .text_color(theme.text_subtle)
        .child(message)
}
