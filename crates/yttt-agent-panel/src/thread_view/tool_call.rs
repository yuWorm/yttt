//! A tool call's card: a header that names what the tool acts on in the form
//! that fits it, a summary that stays readable while the card is folded, the
//! content, and the permission prompt.
use std::time::Duration;

use gpui::{
    AnyElement, Context, Div, IntoElement, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Window, div, prelude::*, px,
};
use gpui_component::{ActiveTheme as _, h_flex, v_flex};
use yttt_ui::{
    Icon, IconName,
    primitives::{
        button::{YtttButtonVariant, yttt_button},
        spinner::yttt_spinner,
    },
    theme::WorkbenchTheme,
};

use super::{ContentCache, ThreadView, ThreadViewEvent, toggle};
use crate::{
    event::{PermissionOptionKind, TodoStatus, ToolCallStatus, ToolContent, ToolKind, ToolTarget},
    model::ToolCall,
    text::split_url,
};

/// Durations shorter than this are noise in the header.
const MIN_SHOWN_DURATION: Duration = Duration::from_millis(100);

impl ThreadView {
    pub(super) fn render_tool_call(
        &self,
        ix: usize,
        call: &ToolCall,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let theme = self.theme;
        let spacing = self.ui_style.spacing;
        let has_body = !call.content.is_empty();
        let open = has_body
            && self.views[ix]
                .open
                .or(self.cards_open)
                .unwrap_or_else(|| opens_itself(call));

        let header = h_flex()
            .id(SharedString::from(format!("tool-call-{ix}")))
            .w_full()
            .min_h(px(30.))
            .gap(spacing.sm)
            .px(spacing.md)
            .text_size(self.ui_style.text.small)
            .child(
                Icon::new(kind_icon(call.kind))
                    .size(px(14.))
                    .text_color(theme.icon_muted),
            )
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .gap(spacing.xs)
                    .overflow_hidden()
                    .children(self.render_subject(ix, call, cx)),
            )
            .children(self.render_summary(ix, call))
            .child(status_indicator(call.status, theme))
            .when(has_body, |this| {
                this.cursor_pointer()
                    .hover(move |this| this.bg(theme.ghost_element_hover))
                    .child(
                        Icon::new(if open {
                            IconName::ChevronDown
                        } else {
                            IconName::ChevronRight
                        })
                        .size(px(12.))
                        .text_color(theme.icon_muted),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        toggle(&mut this.views[ix].open, open);
                        this.list.remeasure_items(ix..ix + 1);
                        cx.notify();
                    }))
            });

        let pending_permission = call
            .permission
            .as_ref()
            .filter(|prompt| prompt.chosen.is_none());

        v_flex()
            .w_full()
            .rounded(self.ui_style.radius.card)
            .border_1()
            .border_color(match call.status {
                ToolCallStatus::AwaitingPermission => theme.warning.alpha(0.6),
                ToolCallStatus::Failed => theme.danger.alpha(0.5),
                _ => theme.border,
            })
            .bg(theme.surface)
            .overflow_hidden()
            .child(header)
            .when(open, |this| {
                this.child(
                    v_flex().border_t_1().border_color(theme.border).children(
                        call.content
                            .iter()
                            .enumerate()
                            .map(|(content_ix, content)| {
                                self.render_content(ix, content_ix, call, content, window, cx)
                            }),
                    ),
                )
            })
            .when(!call.actions.is_empty() && (open || !has_body), |this| {
                this.child(
                    h_flex()
                        .justify_end()
                        .gap(px(2.))
                        .px(spacing.sm)
                        .py(px(4.))
                        .border_t_1()
                        .border_color(theme.border)
                        .children(call.actions.iter().map(|action| {
                            let (tool_call_id, action_id) = (call.id.clone(), action.id.clone());
                            self.text_action(
                                format!("tool-action-{ix}-{}", action.id),
                                action.label.clone(),
                                cx.listener(move |_, _, _, cx| {
                                    cx.emit(ThreadViewEvent::ToolAction {
                                        tool_call_id: tool_call_id.clone(),
                                        action_id: action_id.clone(),
                                    })
                                }),
                            )
                        })),
                )
            })
            .when_some(pending_permission, |this, prompt| {
                this.child(
                    h_flex()
                        .flex_wrap()
                        .gap(spacing.sm)
                        .px(spacing.md)
                        .py(spacing.sm)
                        .border_t_1()
                        .border_color(theme.border)
                        .bg(theme.warning.alpha(0.06))
                        .child(
                            div()
                                .flex_1()
                                .text_size(self.ui_style.text.small)
                                .text_color(theme.text_muted)
                                .child(permission_question(call.kind)),
                        )
                        .children(prompt.options.iter().map(|option| {
                            let variant = match option.kind {
                                PermissionOptionKind::AllowOnce => YtttButtonVariant::Primary,
                                PermissionOptionKind::AllowAlways => YtttButtonVariant::Secondary,
                                PermissionOptionKind::Reject => YtttButtonVariant::Ghost,
                            };
                            let tool_call_id = call.id.clone();
                            let option_id = option.id.clone();
                            yttt_button(
                                SharedString::from(format!("permission-{ix}-{}", option.id.0)),
                                option.label.clone(),
                                variant,
                                theme,
                                self.ui_style,
                            )
                            .on_click(cx.listener(
                                move |_, _, _, cx| {
                                    cx.emit(ThreadViewEvent::PermissionChosen {
                                        tool_call_id: tool_call_id.clone(),
                                        option_id: option_id.clone(),
                                    });
                                },
                            ))
                        })),
                )
            })
    }

    /// What the tool acts on, shaped for its target; the title otherwise.
    fn render_subject(
        &self,
        ix: usize,
        call: &ToolCall,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let theme = self.theme;
        let mono = cx.theme().mono_font_family.clone();
        let verb = || {
            div()
                .flex_shrink_0()
                .text_color(theme.text_muted)
                .child(kind_verb(call.kind))
                .into_any_element()
        };
        let code = |text: &str| {
            div()
                .min_w_0()
                .truncate()
                .px(px(4.))
                .rounded(px(3.))
                .bg(theme.element_background)
                .font_family(mono.clone())
                .text_size(self.ui_style.text.xsmall)
                .text_color(theme.text)
                .child(text.to_owned())
                .into_any_element()
        };
        let muted = |text: String| {
            div()
                .flex_shrink_0()
                .text_color(theme.text_subtle)
                .child(text)
                .into_any_element()
        };
        match &call.target {
            Some(ToolTarget::File { path, lines }) => {
                let mut subject = vec![
                    verb(),
                    self.render_path_link(ix, path, lines.map(|(start, _)| start), cx),
                ];
                if let Some((start, end)) = lines {
                    subject.push(muted(format!("L{start}–{end}")));
                }
                subject
            }
            Some(ToolTarget::Pattern { pattern, scope }) => {
                let mut subject = vec![verb(), code(pattern)];
                if let Some(scope) = scope {
                    subject.push(muted(format!("in {scope}")));
                }
                subject
            }
            Some(ToolTarget::Command { command, .. }) => vec![
                div()
                    .flex_shrink_0()
                    .font_family(mono.clone())
                    .text_color(theme.text_subtle)
                    .child("$")
                    .into_any_element(),
                div()
                    .min_w_0()
                    .truncate()
                    .font_family(mono.clone())
                    .text_size(self.ui_style.text.xsmall)
                    .text_color(theme.text)
                    .child(command.clone())
                    .into_any_element(),
            ],
            Some(ToolTarget::Query(query)) => vec![
                verb(),
                div()
                    .min_w_0()
                    .truncate()
                    .text_color(theme.text)
                    .child(format!("“{query}”"))
                    .into_any_element(),
            ],
            Some(ToolTarget::Url(url)) => {
                let (host, path) = split_url(url);
                let url = url.clone();
                vec![
                    verb(),
                    h_flex()
                        .id(SharedString::from(format!("tool-url-{ix}")))
                        .min_w_0()
                        .overflow_hidden()
                        .cursor_pointer()
                        .hover(|this| this.underline())
                        .child(
                            div()
                                .flex_shrink_0()
                                .text_color(theme.text)
                                .child(host.to_owned()),
                        )
                        .child(
                            div()
                                .min_w_0()
                                .truncate()
                                .text_color(theme.text_subtle)
                                .child(path.to_owned()),
                        )
                        .on_click(move |_, _, cx| {
                            cx.open_url(&url);
                            cx.stop_propagation();
                        })
                        .into_any_element(),
                ]
            }
            Some(ToolTarget::Skill { name }) => vec![verb(), self.render_tag(name)],
            Some(ToolTarget::Mcp { server, tool }) => vec![self.render_tag(server), code(tool)],
            None => vec![
                div()
                    .min_w_0()
                    .truncate()
                    .text_color(theme.text)
                    .child(call.title.clone())
                    .into_any_element(),
            ],
        }
    }

    /// File name first, then its directory, like a breadcrumb; clicking asks
    /// the host to open it.
    pub(super) fn render_path_link(
        &self,
        ix: usize,
        path: &str,
        line: Option<u32>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = self.theme;
        let (dir, name) = path.rsplit_once('/').unwrap_or(("", path));
        let path = path.to_owned();
        h_flex()
            .id(SharedString::from(format!("tool-path-{ix}-{path}")))
            .min_w_0()
            .gap(px(5.))
            .overflow_hidden()
            .cursor_pointer()
            .hover(|this| this.underline())
            .child(
                div()
                    .flex_shrink_0()
                    .text_color(theme.text)
                    .child(name.to_owned()),
            )
            .when(!dir.is_empty(), |this| {
                this.child(
                    div()
                        .min_w_0()
                        .truncate()
                        .text_color(theme.text_subtle)
                        .child(dir.to_owned()),
                )
            })
            .on_click(cx.listener(move |_, _, _, cx| {
                cx.emit(ThreadViewEvent::OpenPath {
                    path: path.clone(),
                    line,
                });
                cx.stop_propagation();
            }))
            .into_any_element()
    }

    pub(super) fn render_tag(&self, text: &str) -> AnyElement {
        div()
            .flex_shrink_0()
            .px(px(6.))
            .rounded(px(4.))
            .bg(self.theme.accent.alpha(0.14))
            .text_color(self.theme.accent)
            .text_size(self.ui_style.text.xsmall)
            .child(text.to_owned())
            .into_any_element()
    }

    /// The gist of the result, readable while the card is folded.
    fn render_summary(&self, ix: usize, call: &ToolCall) -> Option<AnyElement> {
        let theme = self.theme;
        let view = &self.views[ix];
        let mut parts: Vec<(String, gpui::Rgba)> = Vec::new();
        for (content, cache) in call.content.iter().zip(&view.contents) {
            match (content, cache) {
                (ToolContent::Diff { old_text: None, .. }, ContentCache::Diff(diff)) => {
                    parts.push(("new".to_owned(), theme.text_subtle));
                    parts.push((format!("+{}", diff.added), theme.success));
                }
                (ToolContent::Diff { .. }, ContentCache::Diff(diff)) => {
                    parts.push((format!("+{}", diff.added), theme.success));
                    parts.push((format!("−{}", diff.removed), theme.danger));
                }
                (
                    ToolContent::Terminal {
                        exit_code: Some(code),
                        ..
                    },
                    _,
                ) => parts.push((
                    format!("exit {code}"),
                    if *code == 0 {
                        theme.success
                    } else {
                        theme.danger
                    },
                )),
                (ToolContent::File { text, .. }, _)
                    if !matches!(call.target, Some(ToolTarget::File { lines: Some(_), .. })) =>
                {
                    parts.push((plural(text.lines().count(), "line"), theme.text_subtle));
                }
                (ToolContent::SearchResults { files, .. }, _) => {
                    let matches = files.iter().map(|file| file.lines.len()).sum::<usize>();
                    parts.push((
                        format!(
                            "{} · {}",
                            plural(matches, "match"),
                            plural(files.len(), "file")
                        ),
                        theme.text_subtle,
                    ));
                }
                (ToolContent::WebResults(results), _) => {
                    parts.push((plural(results.len(), "result"), theme.text_subtle));
                }
                (ToolContent::Todo(items), _) => {
                    let done = items
                        .iter()
                        .filter(|item| item.status == TodoStatus::Completed)
                        .count();
                    parts.push((format!("{done}/{}", items.len()), theme.text_subtle));
                }
                _ => {}
            }
        }
        let timed = matches!(
            call.kind,
            ToolKind::Execute
                | ToolKind::Fetch
                | ToolKind::WebSearch
                | ToolKind::Mcp
                | ToolKind::Agent
                | ToolKind::Search
        );
        if let Some(elapsed) = view
            .elapsed
            .filter(|elapsed| timed && *elapsed >= MIN_SHOWN_DURATION)
        {
            parts.push((format_duration(elapsed), theme.text_subtle));
        }
        if parts.is_empty() {
            return None;
        }
        Some(
            h_flex()
                .flex_shrink_0()
                .gap(px(6.))
                .text_size(self.ui_style.text.xsmall)
                .children(
                    parts
                        .into_iter()
                        .map(|(text, color)| div().text_color(color).child(text)),
                )
                .into_any_element(),
        )
    }
}

/// Changes, failures and things meant to be seen open themselves; lookups
/// stay folded behind their summary.
fn opens_itself(call: &ToolCall) -> bool {
    if matches!(
        call.status,
        ToolCallStatus::AwaitingPermission | ToolCallStatus::Failed
    ) {
        return true;
    }
    matches!(
        call.kind,
        ToolKind::Edit
            | ToolKind::Write
            | ToolKind::Delete
            | ToolKind::Move
            | ToolKind::Execute
            | ToolKind::Todo
    ) || call.content.iter().any(|content| {
        matches!(
            content,
            ToolContent::Image { .. } | ToolContent::Error(_) | ToolContent::Custom { .. }
        )
    })
}

fn permission_question(kind: ToolKind) -> &'static str {
    match kind {
        ToolKind::Edit | ToolKind::Write => "Apply this change?",
        ToolKind::Delete => "Delete this file?",
        ToolKind::Move => "Move this file?",
        ToolKind::Execute => "Run this command?",
        ToolKind::Fetch | ToolKind::WebSearch => "Allow network access?",
        _ => "Allow this action?",
    }
}

fn kind_verb(kind: ToolKind) -> &'static str {
    match kind {
        ToolKind::Read => "Read",
        ToolKind::Write => "Write",
        ToolKind::Edit => "Edit",
        ToolKind::Delete => "Delete",
        ToolKind::Move => "Move",
        ToolKind::Search => "Search",
        ToolKind::Execute => "Run",
        ToolKind::Think => "Think",
        ToolKind::Fetch => "Fetch",
        ToolKind::WebSearch => "Search web",
        ToolKind::Skill => "Skill",
        ToolKind::Todo => "Plan",
        ToolKind::Mcp | ToolKind::Other => "Tool",
        ToolKind::Agent => "Agent",
    }
}

/// Only the component icon set ships with the app, so these stay within it.
fn kind_icon(kind: ToolKind) -> IconName {
    match kind {
        ToolKind::Read => IconName::FileText,
        ToolKind::Write => IconName::File,
        ToolKind::Edit => IconName::Replace,
        ToolKind::Delete => IconName::Delete,
        ToolKind::Move => IconName::ArrowRight,
        ToolKind::Search => IconName::Search,
        ToolKind::Execute => IconName::SquareTerminal,
        ToolKind::Think => IconName::Asterisk,
        ToolKind::Fetch => IconName::ExternalLink,
        ToolKind::WebSearch => IconName::Globe,
        ToolKind::Skill => IconName::BookOpen,
        ToolKind::Todo => IconName::LayoutDashboard,
        ToolKind::Mcp => IconName::Network,
        ToolKind::Agent => IconName::Bot,
        ToolKind::Other => IconName::Settings,
    }
}

fn status_indicator(status: ToolCallStatus, theme: WorkbenchTheme) -> AnyElement {
    let (icon, color) = match status {
        ToolCallStatus::Running => {
            return yttt_spinner(px(13.), theme.icon_muted).into_any_element();
        }
        ToolCallStatus::Pending => (IconName::Ellipsis, theme.icon_muted),
        ToolCallStatus::AwaitingPermission => (IconName::TriangleAlert, theme.warning),
        ToolCallStatus::Completed => (IconName::Check, theme.success),
        ToolCallStatus::Failed => (IconName::CircleX, theme.danger),
        ToolCallStatus::Rejected => (IconName::Ban, theme.danger),
        ToolCallStatus::Canceled => (IconName::CircleX, theme.icon_muted),
    };
    Icon::new(icon)
        .size(px(14.))
        .text_color(color)
        .into_any_element()
}

fn plural(count: usize, noun: &str) -> String {
    match (count, noun) {
        (1, _) => format!("1 {noun}"),
        (_, "match") => format!("{count} matches"),
        _ => format!("{count} {noun}s"),
    }
}

fn format_duration(elapsed: Duration) -> String {
    if elapsed < Duration::from_secs(60) {
        format!("{:.1}s", elapsed.as_secs_f32())
    } else {
        format!("{}m {}s", elapsed.as_secs() / 60, elapsed.as_secs() % 60)
    }
}
