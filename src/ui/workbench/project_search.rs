//! The project panel's Search page: text search over the selected project's
//! files, run on its Host.

use std::{path::PathBuf, sync::Arc, time::Duration};

use gpui::StyledText;
use yttt_protocol::project::{
    ProjectSearchFile, ProjectSearchLine, ProjectSearchQuery, ProjectSearchResults,
};
use yttt_ui::Icon;

use super::*;

/// Typing waits this long for the next key before searching.
const SEARCH_DEBOUNCE: Duration = Duration::from_millis(250);
const MAX_MATCHES: u32 = 2_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SearchOption {
    CaseSensitive,
    WholeWord,
    Regex,
}

impl WorkbenchView {
    pub(super) fn project_search_input(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<InputState> {
        if let Some(input) = &self.project.search.input {
            return input.clone();
        }
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(self.ui_text.get(UiTextKey::ProjectSearchPlaceholder))
                .clean_on_escape()
        });
        let subscription = cx.subscribe_in(&input, window, Self::on_project_search_input_event);
        self.project.search.input = Some(input.clone());
        self.project.search.input_subscription = Some(subscription);
        input
    }

    fn on_project_search_input_event(
        &mut self,
        _input: &Entity<InputState>,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            InputEvent::Change => self.schedule_project_search(SEARCH_DEBOUNCE, window, cx),
            InputEvent::PressEnter { .. } => {
                self.schedule_project_search(Duration::ZERO, window, cx)
            }
            _ => {}
        }
    }

    fn current_project_search_key(&self, cx: &App) -> Option<ProjectSearchKey> {
        let project_id = self.workspace.selected_project_id()?.clone();
        let pattern = self
            .project
            .search
            .input
            .as_ref()?
            .read(cx)
            .value()
            .to_string();
        let search = &self.project.search;
        Some(ProjectSearchKey {
            project_id,
            pattern,
            case_sensitive: search.case_sensitive,
            whole_word: search.whole_word,
            regex: search.regex,
        })
    }

    /// Searches again when the selected project changed under shown results.
    pub(super) fn sync_project_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(key) = self.current_project_search_key(cx) else {
            return;
        };
        let stale = self
            .project
            .search
            .key
            .as_ref()
            .is_some_and(|shown| shown.project_id != key.project_id);
        if stale && !self.project.search.loading {
            self.schedule_project_search(Duration::ZERO, window, cx);
        }
    }

    fn schedule_project_search(
        &mut self,
        delay: Duration,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(key) = self.current_project_search_key(cx) else {
            return;
        };
        let search = &mut self.project.search;
        search.generation = search.generation.wrapping_add(1);
        let generation = search.generation;
        if key.pattern.is_empty() {
            search.task = None;
            search.loading = false;
            search.results = None;
            search.error = None;
            search.key = Some(key);
            cx.notify();
            return;
        }
        let Some(services) = self.project.services.get(&key.project_id).cloned() else {
            return;
        };
        search.loading = true;
        let query = ProjectSearchQuery {
            pattern: key.pattern.clone(),
            case_sensitive: key.case_sensitive,
            whole_word: key.whole_word,
            regex: key.regex,
            max_matches: MAX_MATCHES,
        };
        search.task = Some(cx.spawn_in(window, async move |this, cx| {
            if !delay.is_zero() {
                cx.background_executor().timer(delay).await;
            }
            let result = cx
                .background_spawn(async move { services.search_text(query) })
                .await;
            let _ = this.update(cx, |view, cx| {
                let search = &mut view.project.search;
                if search.generation != generation {
                    return;
                }
                search.loading = false;
                match result {
                    Ok(Some(results)) => {
                        if search.key.as_ref() != Some(&key) {
                            search.collapsed.clear();
                        }
                        search.results = Some(Arc::new(results));
                        search.error = None;
                        search.unsupported = false;
                    }
                    Ok(None) => {
                        search.results = None;
                        search.unsupported = true;
                    }
                    Err(error) => {
                        search.results = None;
                        search.error = Some(error);
                    }
                }
                search.key = Some(key);
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn toggle_project_search_option(
        &mut self,
        option: SearchOption,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let search = &mut self.project.search;
        let flag = match option {
            SearchOption::CaseSensitive => &mut search.case_sensitive,
            SearchOption::WholeWord => &mut search.whole_word,
            SearchOption::Regex => &mut search.regex,
        };
        *flag = !*flag;
        self.schedule_project_search(Duration::ZERO, window, cx);
    }

    pub(super) fn project_search_panel_content(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let appearance = self.theme_runtime();
        let theme = appearance.ui;
        let ui_style = appearance.style;
        let input = self.project_search_input(window, cx);
        if std::mem::take(&mut self.project.pending_project_search_focus) {
            input.update(cx, |input, input_cx| input.focus(window, input_cx));
        }
        self.sync_project_search(window, cx);
        let ssh = self
            .workspace
            .selected_project_id()
            .and_then(|project_id| self.workspace.project(project_id))
            .is_some_and(|project| project.location.local_path().is_none());
        let search = &self.project.search;
        let toggles = div()
            .flex()
            .flex_none()
            .gap(ui_style.spacing.xxs)
            .child(self.search_option_toggle(
                SearchOption::CaseSensitive,
                "Aa",
                search.case_sensitive,
                UiTextKey::ProjectSearchCaseSensitive,
                theme,
                ui_style,
                cx,
            ))
            .child(self.search_option_toggle(
                SearchOption::WholeWord,
                "W",
                search.whole_word,
                UiTextKey::ProjectSearchWholeWord,
                theme,
                ui_style,
                cx,
            ))
            .child(self.search_option_toggle(
                SearchOption::Regex,
                ".*",
                search.regex,
                UiTextKey::ProjectSearchRegex,
                theme,
                ui_style,
                cx,
            ));
        let header = div()
            .debug_selector(|| "project-search-header".to_string())
            .flex()
            .flex_col()
            .flex_none()
            .gap(ui_style.spacing.xs)
            .px(ui_style.spacing.md)
            .pt(ui_style.spacing.md)
            .pb(ui_style.spacing.xs)
            .child(
                div()
                    .debug_selector(|| "project-search-input".to_string())
                    .flex()
                    .items_center()
                    .gap(ui_style.spacing.xs)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .h(ui_style.icon_buttons.toolbar_size)
                            .child(
                                yttt_input(&input, YtttInputKind::Search, theme, ui_style)
                                    .small()
                                    .h(ui_style.icon_buttons.toolbar_size)
                                    .prefix(IconName::Search)
                                    .cleanable(true),
                            ),
                    )
                    .child(toggles),
            );
        let body = if ssh {
            ports_like_message(
                "project-search-ssh",
                self.ui_text
                    .get(UiTextKey::ProjectSearchSshUnavailable)
                    .to_string(),
                theme,
                ui_style,
            )
        } else if search.unsupported {
            ports_like_message(
                "project-search-unsupported",
                self.ui_text
                    .get(UiTextKey::ProjectSearchUnsupported)
                    .to_string(),
                theme,
                ui_style,
            )
        } else if let Some(error) = search.error.clone() {
            ports_like_message("project-search-error", error, theme, ui_style)
        } else if let Some(results) = search.results.clone() {
            self.project_search_results(&results, theme, ui_style, cx)
        } else if search.loading {
            ports_like_message(
                "project-search-loading",
                self.ui_text
                    .get(UiTextKey::ProjectSearchSearching)
                    .to_string(),
                theme,
                ui_style,
            )
        } else {
            div().flex_1()
        };
        div()
            .debug_selector(|| "project-search".to_string())
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .child(header)
            .child(body)
    }

    #[allow(clippy::too_many_arguments)]
    fn search_option_toggle(
        &self,
        option: SearchOption,
        label: &'static str,
        active: bool,
        tooltip: UiTextKey,
        theme: WorkbenchTheme,
        ui_style: UiStyle,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let tooltip = self.ui_text.get(tooltip);
        div()
            .id(SharedString::from(format!("project-search-option-{label}")))
            .flex()
            .items_center()
            .justify_center()
            .size(ui_style.icon_buttons.toolbar_size)
            .rounded_sm()
            .cursor_pointer()
            .text_xs()
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(if active {
                theme.text
            } else {
                theme.text_subtle
            })
            .when(active, |toggle| toggle.bg(theme.ghost_element_selected))
            .hover(move |style| style.bg(theme.ghost_element_hover))
            .child(label)
            .on_click(cx.listener(move |view, _: &ClickEvent, window, cx| {
                view.toggle_project_search_option(option, window, cx);
            }))
            .tooltip(move |window, cx| {
                yttt_ui::primitives::tooltip::yttt_tooltip(
                    tooltip,
                    crate::ui::theme::current_workbench_theme(cx),
                    crate::ui::theme::current_ui_style(cx),
                )
                .build(window, cx)
            })
    }

    fn project_search_results(
        &self,
        results: &Arc<ProjectSearchResults>,
        theme: WorkbenchTheme,
        ui_style: UiStyle,
        cx: &mut Context<Self>,
    ) -> Div {
        if results.files.is_empty() {
            return ports_like_message(
                "project-search-empty",
                self.ui_text
                    .get(UiTextKey::ProjectSearchNoResults)
                    .to_string(),
                theme,
                ui_style,
            );
        }
        let summary = self
            .ui_text
            .get(UiTextKey::ProjectSearchSummary)
            .replace("{matches}", &results.match_count.to_string())
            .replace("{files}", &results.files.len().to_string());
        let summary = if results.truncated {
            format!(
                "{summary} · {}",
                self.ui_text.get(UiTextKey::ProjectSearchTruncated)
            )
        } else {
            summary
        };
        let Some(project_id) = self
            .project
            .search
            .key
            .as_ref()
            .map(|key| key.project_id.clone())
        else {
            return div().flex_1();
        };
        let mut rows = Vec::new();
        for (file_index, file) in results.files.iter().enumerate() {
            let Ok(path) = file.relative_path.to_utf8() else {
                continue;
            };
            let collapsed = self.project.search.collapsed.contains(&path);
            rows.push(
                self.search_file_row(file_index, file, &path, collapsed, theme, ui_style, cx),
            );
            if collapsed {
                continue;
            }
            for (line_index, line) in file.lines.iter().enumerate() {
                rows.push(self.search_line_row(
                    &project_id,
                    file_index,
                    line_index,
                    &path,
                    line,
                    theme,
                    ui_style,
                    cx,
                ));
            }
        }
        let list = div()
            .id("project-search-results")
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .pb(ui_style.spacing.md)
            .children(rows);
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .child(
                div()
                    .debug_selector(|| "project-search-summary".to_string())
                    .flex_none()
                    .px(ui_style.spacing.md)
                    .pb(ui_style.spacing.xs)
                    .text_xs()
                    .text_color(theme.text_subtle)
                    .child(summary),
            )
            .child(list)
    }

    #[allow(clippy::too_many_arguments)]
    fn search_file_row(
        &self,
        file_index: usize,
        file: &ProjectSearchFile,
        path: &str,
        collapsed: bool,
        theme: WorkbenchTheme,
        ui_style: UiStyle,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (directory, name) = match path.rsplit_once('/') {
            Some((directory, name)) => (directory.to_string(), name.to_string()),
            None => (String::new(), path.to_string()),
        };
        let count: usize = file.lines.iter().map(|line| line.matches.len()).sum();
        let toggle_path = path.to_string();
        div()
            .id(SharedString::from(format!(
                "project-search-file-{file_index}"
            )))
            .flex()
            .items_center()
            .min_w_0()
            .gap(ui_style.spacing.xs)
            .mx(ui_style.spacing.xs)
            .px(ui_style.spacing.xs)
            .py(ui_style.spacing.xxs)
            .rounded_sm()
            .cursor_pointer()
            .text_sm()
            .hover(move |style| style.bg(theme.hover_surface))
            .on_click(cx.listener(move |view, _: &ClickEvent, _window, cx| {
                let collapsed = &mut view.project.search.collapsed;
                if !collapsed.remove(&toggle_path) {
                    collapsed.insert(toggle_path.clone());
                }
                cx.notify();
            }))
            .child(
                Icon::new(if collapsed {
                    IconName::ChevronRight
                } else {
                    IconName::ChevronDown
                })
                .size(px(12.0))
                .text_color(theme.icon_muted),
            )
            .child(
                div()
                    .flex_none()
                    .max_w(relative(0.7))
                    .truncate()
                    .text_color(theme.text)
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
                    .text_color(theme.text_subtle)
                    .child(count.to_string()),
            )
            .into_any_element()
    }

    #[allow(clippy::too_many_arguments)]
    fn search_line_row(
        &self,
        project_id: &ProjectId,
        file_index: usize,
        line_index: usize,
        path: &str,
        line: &ProjectSearchLine,
        theme: WorkbenchTheme,
        ui_style: UiStyle,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let reveal = PendingFileReveal {
            project_id: project_id.clone(),
            relative_path: PathBuf::from(path),
            line: line.line,
            column: match_column(line),
        };
        let highlight = HighlightStyle {
            background_color: Some(gpui::Hsla::from(theme.accent).opacity(0.3)),
            ..HighlightStyle::default()
        };
        let highlights = line
            .matches
            .iter()
            .filter(|range| {
                (range.end as usize) <= line.text.len()
                    && line.text.is_char_boundary(range.start as usize)
                    && line.text.is_char_boundary(range.end as usize)
            })
            .map(|range| (range.start as usize..range.end as usize, highlight))
            .collect::<Vec<_>>();
        div()
            .id(SharedString::from(format!(
                "project-search-line-{file_index}-{line_index}"
            )))
            .flex()
            .items_center()
            .min_w_0()
            .gap(ui_style.spacing.sm)
            .mx(ui_style.spacing.xs)
            .pl(ui_style.spacing.xl)
            .pr(ui_style.spacing.xs)
            .py(ui_style.spacing.xxs)
            .rounded_sm()
            .cursor_pointer()
            .text_xs()
            .hover(move |style| style.bg(theme.hover_surface))
            .on_click(cx.listener(move |view, _: &ClickEvent, window, cx| {
                view.open_project_file_at(reveal.clone(), window, cx);
            }))
            .child(
                div()
                    .flex_none()
                    .min_w(px(24.0))
                    .text_color(theme.text_subtle)
                    .child((line.line + 1).to_string()),
            )
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_color(theme.text_muted)
                    .child(StyledText::new(line.text.clone()).with_highlights(highlights)),
            )
            .into_any_element()
    }
}

/// The UTF-16 column of a line's first match, where its cursor goes.
fn match_column(line: &ProjectSearchLine) -> u32 {
    let start = line
        .matches
        .first()
        .map(|range| range.start as usize)
        .unwrap_or(0)
        .min(line.text.len());
    let start = (0..=start)
        .rev()
        .find(|index| line.text.is_char_boundary(*index))
        .unwrap_or(0);
    line.column_offset + line.text[..start].encode_utf16().count() as u32
}

fn ports_like_message(
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

#[cfg(test)]
mod tests {
    use super::*;
    use yttt_protocol::project::ProjectSearchRange;

    #[test]
    fn the_cursor_goes_to_the_first_match_in_utf16_columns() {
        let line = ProjectSearchLine {
            line: 3,
            text: "名前 = port".to_string(),
            column_offset: 4,
            matches: vec![ProjectSearchRange { start: 9, end: 13 }],
        };
        // "名前 = " is 5 UTF-16 units; the text starts 4 units into the line.
        assert_eq!(match_column(&line), 9);
    }
}
