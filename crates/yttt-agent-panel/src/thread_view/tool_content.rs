//! One renderer per kind of tool result. Long results show their most useful
//! part (a file's head, a terminal's tail) with a row to show the rest.
use gpui::{
    AnyElement, Context, FontWeight, HighlightStyle, ImageSource as GpuiImageSource, IntoElement,
    ObjectFit, ParentElement as _, SharedString, StatefulInteractiveElement as _, StyledImage as _,
    StyledText, Window, div, img, prelude::*, px,
};
use gpui_component::{ActiveTheme as _, diff::Diff, h_flex, text::TextView, v_flex};
use yttt_ui::{Icon, IconName, primitives::spinner::yttt_spinner};

use super::{ContentCache, DiffView, ThreadView};
use crate::{
    event::{FileMatches, ImageSource, TodoItem, TodoStatus, ToolContent, ToolTarget, WebResult},
    model::ToolCall,
    text::{
        AnsiColor, StyledOutput, clip_styles, find_matches, head_lines, parse_ansi, split_url,
        tail_lines,
    },
};

/// Taller diffs scroll inside the card.
const MAX_DIFF_ROWS: usize = 16;
const MAX_TERMINAL_LINES: usize = 24;
const MAX_FILE_LINES: usize = 30;
const MAX_JSON_LINES: usize = 20;
const MAX_PAGE_LINES: usize = 12;
const MAX_MATCH_FILES: usize = 6;
const MAX_MATCHES_PER_FILE: usize = 5;
const MAX_IMAGE_HEIGHT: f32 = 260.;

impl ThreadView {
    pub(super) fn render_content(
        &self,
        ix: usize,
        content_ix: usize,
        call: &ToolCall,
        content: &ToolContent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let expanded = self.views[ix].expanded.contains(&content_ix);
        match content {
            ToolContent::Markdown(markdown) => self
                .padded()
                .child(TextView::markdown(
                    SharedString::from(format!("tool-markdown-{ix}-{content_ix}")),
                    markdown.clone(),
                ))
                .into_any_element(),
            ToolContent::Diff { path, .. } => match &self.views[ix].contents[content_ix] {
                ContentCache::Diff(diff) => {
                    // The header already names the file the card is about.
                    let named = matches!(&call.target, Some(ToolTarget::File { path: target, .. }) if target == path);
                    self.render_diff(ix, (!named).then_some(path.as_str()), diff, window, cx)
                }
                _ => div().into_any_element(),
            },
            ToolContent::Terminal {
                command, output, ..
            } => {
                // The header already shows the command when it is the target.
                let show_command = !matches!(call.target, Some(ToolTarget::Command { .. }));
                self.render_terminal(
                    ix,
                    content_ix,
                    show_command.then_some(command),
                    output,
                    expanded,
                    cx,
                )
            }
            ToolContent::File {
                text, first_line, ..
            } => self.render_file(ix, content_ix, text, *first_line, expanded, cx),
            ToolContent::SearchResults { query, files } => {
                self.render_matches(ix, content_ix, query.as_deref(), files, expanded, cx)
            }
            ToolContent::WebResults(results) => self.render_web_results(ix, content_ix, results),
            ToolContent::WebPage {
                url,
                title,
                excerpt,
            } => self.render_web_page(ix, content_ix, url, title, excerpt, expanded, cx),
            ToolContent::Image { source, caption } => {
                self.render_image(ix, content_ix, source, caption.as_deref())
            }
            ToolContent::Json { label, text } => {
                self.render_json(ix, content_ix, label, text, expanded, cx)
            }
            ToolContent::Todo(items) => self.render_todo(items),
            ToolContent::Custom { kind, data } => match self.custom_renderers.get(kind).cloned() {
                Some(render) => self
                    .padded()
                    .child(render(data, window, cx))
                    .into_any_element(),
                // Without a renderer the payload still reads, as JSON.
                None => self.render_json(ix, content_ix, kind, data, expanded, cx),
            },
            ToolContent::Error(message) => self.render_error(message),
        }
    }

    fn padded(&self) -> gpui::Div {
        div()
            .px(self.ui_style.spacing.md)
            .py(self.ui_style.spacing.sm)
            .text_size(self.ui_style.text.small)
            .text_color(self.theme.text)
    }

    /// A quiet row that shows or hides the rest of a long result.
    fn render_more(
        &self,
        ix: usize,
        content_ix: usize,
        expanded: bool,
        hidden: usize,
        what: &str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = self.theme;
        let label = if expanded {
            "Show less".to_owned()
        } else {
            format!("Show {hidden} {what}")
        };
        div()
            .id(SharedString::from(format!("tool-more-{ix}-{content_ix}")))
            .px(self.ui_style.spacing.md)
            .py(px(4.))
            .text_size(self.ui_style.text.xsmall)
            .text_color(theme.text_subtle)
            .cursor_pointer()
            .hover(move |this| this.text_color(theme.text).bg(theme.ghost_element_hover))
            .child(label)
            .on_click(cx.listener(move |this, _, _, cx| {
                let expanded = &mut this.views[ix].expanded;
                if !expanded.remove(&content_ix) {
                    expanded.insert(content_ix);
                }
                this.list.remeasure_items(ix..ix + 1);
                cx.notify();
            }))
            .into_any_element()
    }

    /// The diff view scrolls rows inside itself, so the card gives it a
    /// height for the rows it has, matching the view's own row metrics.
    fn render_diff(
        &self,
        ix: usize,
        path: Option<&str>,
        diff: &DiffView,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = self.theme;
        let spacing = self.ui_style.spacing;
        let row_height = window.rem_size() * (f32::from(cx.theme().mono_font_size) / 16.) * 1.6;
        let rows = diff.rows.min(MAX_DIFF_ROWS);
        v_flex()
            .when_some(path, |this, path| {
                this.child(
                    h_flex()
                        .gap(spacing.sm)
                        .px(spacing.md)
                        .py(spacing.xs)
                        .text_size(self.ui_style.text.xsmall)
                        .bg(theme.toolbar_background)
                        .child(
                            Icon::new(IconName::File)
                                .size(px(13.))
                                .text_color(theme.icon_muted),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .overflow_hidden()
                                .child(self.render_path_link(ix, path, None, cx)),
                        )
                        .child(
                            div()
                                .text_color(theme.success)
                                .child(format!("+{}", diff.added)),
                        )
                        .child(
                            div()
                                .text_color(theme.danger)
                                .child(format!("−{}", diff.removed)),
                        ),
                )
            })
            .when(rows > 0, |this| {
                this.child(
                    div()
                        .w_full()
                        .h(row_height * rows as f32)
                        .child(Diff::new(&diff.state).header_visible(false).size_full()),
                )
            })
            .into_any_element()
    }

    /// Colored like the terminal printed it; the tail matters most, so the
    /// head is what folds away.
    fn render_terminal(
        &self,
        ix: usize,
        content_ix: usize,
        command: Option<&String>,
        output: &str,
        expanded: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = self.theme;
        let spacing = self.ui_style.spacing;
        let parsed = parse_ansi(output);
        let (tail, hidden) = tail_lines(&parsed, MAX_TERMINAL_LINES);
        let shown = if expanded { parsed } else { tail };
        let mono = cx.theme().mono_font_family.clone();
        v_flex()
            .bg(theme.terminal_background)
            .when(hidden > 0, |this| {
                this.child(self.render_more(ix, content_ix, expanded, hidden, "earlier lines", cx))
            })
            .child(
                v_flex()
                    .px(spacing.md)
                    .py(spacing.sm)
                    .gap(spacing.xs)
                    .font_family(mono)
                    .text_size(self.ui_style.text.xsmall)
                    .when_some(command, |this, command| {
                        this.child(
                            h_flex()
                                .gap(spacing.xs)
                                .child(div().text_color(theme.text_subtle).child("$"))
                                .child(div().text_color(theme.text).child(command.clone())),
                        )
                    })
                    .when(!shown.text.is_empty(), |this| {
                        this.child(
                            div()
                                .text_color(theme.text_muted)
                                .child(self.styled_output(shown)),
                        )
                    }),
            )
            .into_any_element()
    }

    fn styled_output(&self, output: StyledOutput) -> StyledText {
        let theme = self.theme;
        let highlights = output.styles.into_iter().map(move |(range, style)| {
            let color = style.color.map(|color| match color {
                AnsiColor::Red => theme.danger,
                AnsiColor::Green => theme.success,
                AnsiColor::Yellow => theme.warning,
                AnsiColor::Blue | AnsiColor::Magenta | AnsiColor::Cyan => theme.accent,
                AnsiColor::Black => theme.text_subtle,
                AnsiColor::White => theme.text,
            });
            (
                range,
                HighlightStyle {
                    color: color.map(Into::into),
                    font_weight: style.bold.then_some(FontWeight::BOLD),
                    ..Default::default()
                },
            )
        });
        StyledText::new(output.text).with_highlights(highlights.collect::<Vec<_>>())
    }

    /// A file's head as numbered, highlighted lines.
    fn render_file(
        &self,
        ix: usize,
        content_ix: usize,
        text: &str,
        first_line: u32,
        expanded: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = self.theme;
        let styles = match &self.views[ix].contents[content_ix] {
            ContentCache::Code { styles, .. } => styles.as_slice(),
            _ => &[],
        };
        let mut offset = 0;
        let lines = text
            .trim_end_matches('\n')
            .split('\n')
            .map(|line| {
                let range = offset..offset + line.len();
                offset = range.end + 1;
                (line, range)
            })
            .collect::<Vec<_>>();
        let hidden = lines.len().saturating_sub(MAX_FILE_LINES);
        let shown = if expanded {
            lines.len()
        } else {
            MAX_FILE_LINES
        };
        v_flex()
            .child(
                v_flex()
                    .px(self.ui_style.spacing.md)
                    .py(self.ui_style.spacing.sm)
                    .bg(theme.editor_background)
                    .font_family(cx.theme().mono_font_family.clone())
                    .text_size(self.ui_style.text.xsmall)
                    .children(lines.iter().take(shown).enumerate().map(
                        |(line_ix, (line, range))| {
                            h_flex()
                                .gap(self.ui_style.spacing.sm)
                                .child(
                                    div()
                                        .w(px(36.))
                                        .flex_shrink_0()
                                        .text_right()
                                        .text_color(theme.text_subtle)
                                        .child((first_line as usize + line_ix).to_string()),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .truncate()
                                        .text_color(theme.text)
                                        .child(
                                            StyledText::new((*line).to_owned()).with_highlights(
                                                clip_styles(styles, range.clone()),
                                            ),
                                        ),
                                )
                        },
                    )),
            )
            .when(hidden > 0, |this| {
                this.child(self.render_more(ix, content_ix, expanded, hidden, "more lines", cx))
            })
            .into_any_element()
    }

    /// Matches grouped by file, the query highlighted in each line.
    fn render_matches(
        &self,
        ix: usize,
        content_ix: usize,
        query: Option<&str>,
        files: &[FileMatches],
        expanded: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = self.theme;
        let spacing = self.ui_style.spacing;
        let mono = cx.theme().mono_font_family.clone();
        let highlight = HighlightStyle {
            color: Some(theme.text.into()),
            background_color: Some(theme.warning.alpha(0.25).into()),
            ..Default::default()
        };
        let total = files.iter().map(|file| file.lines.len()).sum::<usize>();
        let mut shown_count = 0;
        let groups = files
            .iter()
            .take(if expanded {
                usize::MAX
            } else {
                MAX_MATCH_FILES
            })
            .map(|file| {
                let lines = &file.lines[..if expanded {
                    file.lines.len()
                } else {
                    file.lines.len().min(MAX_MATCHES_PER_FILE)
                }];
                shown_count += lines.len();
                v_flex()
                    .gap(px(2.))
                    .child(
                        h_flex()
                            .gap(spacing.sm)
                            .text_size(self.ui_style.text.xsmall)
                            .child(
                                Icon::new(IconName::FileText)
                                    .size(px(12.))
                                    .text_color(theme.icon_muted),
                            )
                            .child(div().flex_1().min_w_0().overflow_hidden().child(
                                self.render_path_link(
                                    ix,
                                    &file.path,
                                    file.lines.first().map(|line| line.number),
                                    cx,
                                ),
                            ))
                            .child(
                                div()
                                    .text_color(theme.text_subtle)
                                    .child(file.lines.len().to_string()),
                            ),
                    )
                    .children(lines.iter().map(|line| {
                        let ranges =
                            query.map_or_else(Vec::new, |query| find_matches(&line.text, query));
                        h_flex()
                            .gap(spacing.sm)
                            .font_family(mono.clone())
                            .text_size(self.ui_style.text.xsmall)
                            .child(
                                div()
                                    .w(px(36.))
                                    .flex_shrink_0()
                                    .text_right()
                                    .text_color(theme.text_subtle)
                                    .child(line.number.to_string()),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_color(theme.text_muted)
                                    .child(
                                        StyledText::new(line.text.trim_end().to_owned())
                                            .with_highlights(
                                                ranges.into_iter().map(|range| (range, highlight)),
                                            ),
                                    ),
                            )
                    }))
            })
            .collect::<Vec<_>>();
        let hidden = total - shown_count;
        v_flex()
            .child(
                v_flex()
                    .px(spacing.md)
                    .py(spacing.sm)
                    .gap(spacing.sm)
                    .children(groups),
            )
            .when(hidden > 0 || expanded, |this| {
                this.child(self.render_more(ix, content_ix, expanded, hidden, "more matches", cx))
            })
            .into_any_element()
    }

    fn render_web_results(
        &self,
        ix: usize,
        content_ix: usize,
        results: &[WebResult],
    ) -> AnyElement {
        let theme = self.theme;
        let spacing = self.ui_style.spacing;
        v_flex()
            .px(spacing.md)
            .py(spacing.sm)
            .gap(spacing.md)
            .children(results.iter().enumerate().map(|(result_ix, result)| {
                let url = result.url.clone();
                let (host, _) = split_url(&result.url);
                v_flex()
                    .gap(px(2.))
                    .child(
                        h_flex()
                            .gap(spacing.xs)
                            .text_size(self.ui_style.text.xsmall)
                            .text_color(theme.text_subtle)
                            .child(Icon::new(IconName::Globe).size(px(11.)))
                            .child(host.to_owned()),
                    )
                    .child(
                        div()
                            .id(SharedString::from(format!(
                                "web-result-{ix}-{content_ix}-{result_ix}"
                            )))
                            .text_size(self.ui_style.text.small)
                            .text_color(theme.accent)
                            .cursor_pointer()
                            .hover(|this| this.underline())
                            .child(result.title.clone())
                            .on_click(move |_, _, cx| cx.open_url(&url)),
                    )
                    .child(
                        div()
                            .text_size(self.ui_style.text.xsmall)
                            .text_color(theme.text_muted)
                            .line_clamp(2)
                            .child(result.snippet.clone()),
                    )
            }))
            .into_any_element()
    }

    #[allow(clippy::too_many_arguments)]
    fn render_web_page(
        &self,
        ix: usize,
        content_ix: usize,
        url: &str,
        title: &str,
        excerpt: &str,
        expanded: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = self.theme;
        let (head, hidden) = head_lines(excerpt, MAX_PAGE_LINES);
        let shown = if expanded { excerpt } else { head };
        let link = url.to_owned();
        v_flex()
            .child(
                v_flex()
                    .px(self.ui_style.spacing.md)
                    .pt(self.ui_style.spacing.sm)
                    .child(
                        div()
                            .text_size(self.ui_style.text.small)
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme.text)
                            .child(title.to_owned()),
                    )
                    .child(
                        div()
                            .id(SharedString::from(format!("web-page-{ix}-{content_ix}")))
                            .truncate()
                            .text_size(self.ui_style.text.xsmall)
                            .text_color(theme.text_subtle)
                            .cursor_pointer()
                            .hover(|this| this.underline())
                            .child(url.to_owned())
                            .on_click(move |_, _, cx| cx.open_url(&link)),
                    ),
            )
            .child(
                self.padded()
                    .text_color(theme.text_muted)
                    .child(TextView::markdown(
                        SharedString::from(format!("web-page-text-{ix}-{content_ix}-{expanded}")),
                        shown.to_owned(),
                    )),
            )
            .when(hidden > 0, |this| {
                this.child(self.render_more(ix, content_ix, expanded, hidden, "more lines", cx))
            })
            .into_any_element()
    }

    fn render_image(
        &self,
        ix: usize,
        content_ix: usize,
        source: &ImageSource,
        caption: Option<&str>,
    ) -> AnyElement {
        let theme = self.theme;
        let source: Option<GpuiImageSource> = match (source, &self.views[ix].contents[content_ix]) {
            (ImageSource::Bytes { .. }, ContentCache::Image { image, .. }) => {
                Some(image.clone().into())
            }
            (ImageSource::Bytes { .. }, _) => None,
            (ImageSource::Path(path), _) => Some(std::path::PathBuf::from(path).into()),
            (ImageSource::Url(url), _) => Some(url.clone().into()),
        };
        v_flex()
            .px(self.ui_style.spacing.md)
            .py(self.ui_style.spacing.sm)
            .gap(self.ui_style.spacing.xs)
            .child(match source {
                Some(source) => div()
                    .self_start()
                    .max_w_full()
                    .rounded(px(6.))
                    .border_1()
                    .border_color(theme.border)
                    .overflow_hidden()
                    .child(
                        img(source)
                            .max_w_full()
                            .max_h(px(MAX_IMAGE_HEIGHT))
                            .object_fit(ObjectFit::Contain),
                    )
                    .into_any_element(),
                None => self.render_error("Unsupported image format"),
            })
            .when_some(caption, |this, caption| {
                this.child(
                    div()
                        .text_size(self.ui_style.text.xsmall)
                        .text_color(theme.text_subtle)
                        .child(caption.to_owned()),
                )
            })
            .into_any_element()
    }

    /// Pretty-printed when it parses as JSON, shown as given otherwise.
    fn render_json(
        &self,
        ix: usize,
        content_ix: usize,
        label: &str,
        text: &str,
        expanded: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = self.theme;
        let pretty = serde_json::from_str::<serde_json::Value>(text)
            .and_then(|value| serde_json::to_string_pretty(&value))
            .unwrap_or_else(|_| text.to_owned());
        let (head, hidden) = head_lines(&pretty, MAX_JSON_LINES);
        let shown = if expanded { pretty.as_str() } else { head };
        v_flex()
            .px(self.ui_style.spacing.md)
            .py(self.ui_style.spacing.sm)
            .gap(px(4.))
            .child(
                div()
                    .text_size(self.ui_style.text.xsmall)
                    .text_color(theme.text_subtle)
                    .child(label.to_owned()),
            )
            .child(
                div()
                    .px(self.ui_style.spacing.sm)
                    .py(px(6.))
                    .rounded(px(4.))
                    .bg(theme.terminal_background)
                    .font_family(cx.theme().mono_font_family.clone())
                    .text_size(self.ui_style.text.xsmall)
                    .text_color(theme.text_muted)
                    .child(shown.to_owned()),
            )
            .when(hidden > 0, |this| {
                this.child(self.render_more(ix, content_ix, expanded, hidden, "more lines", cx))
            })
            .into_any_element()
    }

    fn render_todo(&self, items: &[TodoItem]) -> AnyElement {
        let theme = self.theme;
        v_flex()
            .px(self.ui_style.spacing.md)
            .py(self.ui_style.spacing.sm)
            .gap(px(6.))
            .text_size(self.ui_style.text.small)
            .children(items.iter().map(|item| {
                let marker = match item.status {
                    TodoStatus::Completed => Icon::new(IconName::CircleCheck)
                        .size(px(14.))
                        .text_color(theme.success)
                        .into_any_element(),
                    TodoStatus::InProgress => {
                        yttt_spinner(px(13.), theme.accent).into_any_element()
                    }
                    TodoStatus::Pending => Icon::new(IconName::Square)
                        .size(px(13.))
                        .text_color(theme.icon_muted)
                        .into_any_element(),
                };
                h_flex()
                    .gap(self.ui_style.spacing.sm)
                    .child(div().flex_shrink_0().child(marker))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .map(|this| match item.status {
                                TodoStatus::Completed => {
                                    this.line_through().text_color(theme.text_subtle)
                                }
                                TodoStatus::InProgress => {
                                    this.text_color(theme.text).font_weight(FontWeight::MEDIUM)
                                }
                                TodoStatus::Pending => this.text_color(theme.text_muted),
                            })
                            .child(item.text.clone()),
                    )
            }))
            .into_any_element()
    }

    fn render_error(&self, message: &str) -> AnyElement {
        let theme = self.theme;
        h_flex()
            .items_start()
            .gap(self.ui_style.spacing.sm)
            .px(self.ui_style.spacing.md)
            .py(self.ui_style.spacing.sm)
            .bg(theme.danger.alpha(0.08))
            .text_size(self.ui_style.text.small)
            .text_color(theme.danger)
            .child(
                div()
                    .pt(px(2.))
                    .child(Icon::new(IconName::TriangleAlert).size(px(13.))),
            )
            .child(div().flex_1().min_w_0().child(message.to_owned()))
            .into_any_element()
    }
}
