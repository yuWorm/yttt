//! Messages and the thread's own lines: prompts, replies, thoughts, notices,
//! the end of each turn and the empty thread.
use std::time::Duration;

use gpui::{
    AnyElement, ClipboardItem, Context, Div, FontWeight, HighlightStyle, IntoElement, ObjectFit,
    ParentElement as _, SharedString, StatefulInteractiveElement as _, StyledImage as _,
    StyledText, div, img, prelude::*, px,
};
use gpui_component::{h_flex, shimmer::ShimmerText, text::TextView, v_flex};
use yttt_ui::{Icon, IconName, primitives::spinner::yttt_spinner};

use super::{ThreadView, ThreadViewEvent, toggle};
use crate::{
    event::{Attachment, AttachmentKind, MessageId, NoticeKind, PromptSegment, StopReason},
    model::{AssistantChunk, Notice, ThreadEntry, ThreadStatus},
    slots::MessagePlacement,
};

const ATTACHMENT_THUMBNAIL: f32 = 56.;

/// A prompt's text, mentions written the way the composer showed them.
pub(crate) fn prompt_text(content: &[PromptSegment]) -> String {
    content
        .iter()
        .map(|segment| match segment {
            PromptSegment::Text(text) => text.clone(),
            PromptSegment::Mention { label, .. } => format!("@{label}"),
        })
        .collect()
}

impl ThreadView {
    /// A prompt as sent, mentions highlighted inline so they wrap with the
    /// text, attachments below, and copy and edit on hover.
    pub(super) fn render_user(
        &self,
        ix: usize,
        id: &MessageId,
        content: &[PromptSegment],
        attachments: &[Attachment],
        cx: &mut Context<Self>,
    ) -> Div {
        let theme = self.theme;
        let ui_style = self.ui_style;
        let mention = HighlightStyle {
            color: Some(theme.accent.into()),
            background_color: Some(theme.accent.alpha(0.14).into()),
            font_weight: Some(FontWeight::MEDIUM),
            ..Default::default()
        };
        let mut text = String::new();
        let mut highlights = Vec::new();
        for segment in content {
            match segment {
                PromptSegment::Text(segment) => text.push_str(segment),
                PromptSegment::Mention { label, .. } => {
                    let start = text.len();
                    text.push('@');
                    text.push_str(label);
                    highlights.push((start..text.len(), mention));
                }
            }
        }
        let group = SharedString::from(format!("user-message-{ix}"));
        let copied = prompt_text(content);
        let editable = self.thread.status() == ThreadStatus::Idle;
        let id = id.clone();
        v_flex()
            .group(group.clone())
            .relative()
            .w_full()
            .child(
                v_flex()
                    .w_full()
                    .gap(ui_style.spacing.sm)
                    .rounded(ui_style.radius.card)
                    .border_1()
                    .border_color(theme.border)
                    .bg(theme.surface_elevated)
                    .px(ui_style.spacing.md)
                    .py(ui_style.spacing.sm)
                    .text_size(ui_style.text.small)
                    .text_color(theme.text)
                    .when(!text.is_empty(), |this| {
                        this.child(StyledText::new(text).with_highlights(highlights))
                    })
                    .when(!attachments.is_empty(), |this| {
                        this.child(
                            h_flex().flex_wrap().gap(ui_style.spacing.sm).children(
                                attachments
                                    .iter()
                                    .map(|attachment| self.render_attachment(attachment)),
                            ),
                        )
                    }),
            )
            // Over the bubble's corner, so hover actions take no space.
            .child(
                h_flex()
                    .absolute()
                    .top(px(4.))
                    .right(px(4.))
                    .gap(px(2.))
                    .p(px(1.))
                    .rounded(px(5.))
                    .bg(theme.surface_elevated)
                    .invisible()
                    .group_hover(group, |this| this.visible())
                    .child(
                        self.text_action(format!("user-copy-{ix}"), "Copy", move |_, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(copied.clone()))
                        }),
                    )
                    .when(editable, |this| {
                        let id = id.clone();
                        this.child(self.text_action(
                            format!("user-edit-{ix}"),
                            "Edit",
                            cx.listener(move |_, _, _, cx| {
                                cx.emit(ThreadViewEvent::EditMessage { id: id.clone() })
                            }),
                        ))
                    })
                    .children(self.host_message_actions(
                        ix,
                        MessagePlacement::User,
                        Some(id),
                        prompt_text(content),
                        cx,
                    )),
            )
    }

    /// The host's actions for one placement, each reporting the message it
    /// sits on.
    fn host_message_actions(
        &self,
        ix: usize,
        placement: MessagePlacement,
        message_id: Option<MessageId>,
        text: String,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        self.message_actions
            .iter()
            .filter(|action| action.placement == placement)
            .map(|action| {
                let (action_id, message_id, text) =
                    (action.id.clone(), message_id.clone(), text.clone());
                self.text_action(
                    format!("message-action-{ix}-{}", action.id),
                    action.label.clone(),
                    cx.listener(move |_, _, _, cx| {
                        cx.emit(ThreadViewEvent::MessageAction {
                            action_id: action_id.clone(),
                            message_id: message_id.clone(),
                            text: text.clone(),
                        })
                    }),
                )
                .into_any_element()
            })
            .collect()
    }

    /// An attachment as a thumbnail when it is an image, a file chip
    /// otherwise.
    pub(crate) fn render_attachment(&self, attachment: &Attachment) -> AnyElement {
        render_attachment_preview(attachment, self.theme, self.ui_style)
    }

    /// A small ghost text button for message actions.
    pub(super) fn text_action(
        &self,
        id: String,
        label: impl Into<SharedString>,
        on_click: impl Fn(&gpui::ClickEvent, &mut gpui::Window, &mut gpui::App) + 'static,
    ) -> impl IntoElement {
        let theme = self.theme;
        div()
            .id(SharedString::from(id))
            .px(px(6.))
            .py(px(1.))
            .rounded(px(4.))
            .text_size(self.ui_style.text.xsmall)
            .text_color(theme.text_subtle)
            .cursor_pointer()
            .hover(move |this| this.bg(theme.ghost_element_hover).text_color(theme.text))
            .child(label.into())
            .on_click(on_click)
    }

    pub(super) fn render_assistant(
        &self,
        ix: usize,
        chunks: &[AssistantChunk],
        cx: &mut Context<Self>,
    ) -> Div {
        let streaming = self.is_streaming(ix);
        let last = chunks.len().saturating_sub(1);
        v_flex()
            .w_full()
            .gap(self.ui_style.spacing.sm)
            .children(chunks.iter().enumerate().map(|(chunk_ix, chunk)| {
                match chunk {
                    AssistantChunk::Text(_) => TextView::new(&self.views[ix].chunks[chunk_ix].text)
                        .text_size(self.ui_style.text.small)
                        .text_color(self.theme.text)
                        .into_any_element(),
                    AssistantChunk::Thought(_) => {
                        self.render_thought(ix, chunk_ix, streaming && chunk_ix == last, cx)
                    }
                }
            }))
    }

    /// Open while it streams, folded once anything follows, unless the user
    /// chose otherwise.
    fn render_thought(
        &self,
        ix: usize,
        chunk_ix: usize,
        streaming: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let chunk = &self.views[ix].chunks[chunk_ix];
        let open = chunk.open.unwrap_or(streaming);
        let theme = self.theme;
        let label = if streaming {
            ShimmerText::new("Thinking…").into_any_element()
        } else {
            let label = match chunk.elapsed {
                Some(elapsed) if elapsed >= Duration::from_secs(1) => {
                    format!("Thought for {}s", elapsed.as_secs())
                }
                _ => "Thought".to_owned(),
            };
            div().child(label).into_any_element()
        };
        v_flex()
            .w_full()
            .child(
                h_flex()
                    .id(SharedString::from(format!("thought-{ix}-{chunk_ix}")))
                    .gap(self.ui_style.spacing.xs)
                    .cursor_pointer()
                    .text_size(self.ui_style.text.small)
                    .text_color(theme.text_muted)
                    .hover(move |this| this.text_color(theme.text))
                    .child(
                        Icon::new(IconName::Asterisk)
                            .size(px(14.))
                            .text_color(theme.icon_muted),
                    )
                    .child(label)
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
                        toggle(&mut this.views[ix].chunks[chunk_ix].open, open);
                        this.list.remeasure_items(ix..ix + 1);
                        cx.notify();
                    })),
            )
            .when(open, |this| {
                this.child(
                    div()
                        .mt(self.ui_style.spacing.xs)
                        .ml(px(6.))
                        .pl(self.ui_style.spacing.md)
                        .border_l_2()
                        .border_color(theme.border)
                        .text_size(self.ui_style.text.small)
                        .text_color(theme.text_muted)
                        .child(TextView::new(&chunk.text)),
                )
            })
            .into_any_element()
    }

    /// An error, warning or retry in the thread; the turn's end, not the
    /// notice, offers the retry.
    pub(super) fn render_notice(
        &self,
        ix: usize,
        notice: &Notice,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = self.theme;
        let spacing = self.ui_style.spacing;
        let text = self.ui_style.text;
        if notice.kind == NoticeKind::Compaction {
            let line = || div().flex_1().h(px(1.)).bg(theme.border);
            return h_flex()
                .gap(spacing.sm)
                .py(spacing.xs)
                .text_size(text.xsmall)
                .text_color(theme.text_subtle)
                .child(line())
                .child(format!("Context compacted · {}", notice.message))
                .child(line())
                .into_any_element();
        }
        let (color, background) = match notice.kind {
            NoticeKind::Error => (theme.danger, theme.danger.alpha(0.08)),
            NoticeKind::Warning | NoticeKind::Retrying => {
                (theme.warning, theme.warning.alpha(0.08))
            }
            NoticeKind::Info | NoticeKind::Compaction => {
                (theme.text_muted, theme.element_background)
            }
        };
        let marker = match notice.kind {
            NoticeKind::Retrying => yttt_spinner(px(13.), color).into_any_element(),
            kind => Icon::new(match kind {
                NoticeKind::Error => IconName::CircleX,
                NoticeKind::Warning => IconName::TriangleAlert,
                _ => IconName::Info,
            })
            .size(px(14.))
            .text_color(color)
            .into_any_element(),
        };
        h_flex()
            .items_start()
            .gap(spacing.sm)
            .px(spacing.md)
            .py(spacing.sm)
            .rounded(self.ui_style.radius.card)
            .bg(background)
            .text_size(text.small)
            .child(div().pt(px(2.)).child(marker))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_color(color)
                    .child(notice.message.clone()),
            )
            .children(notice.actions.iter().map(|action| {
                let (notice_id, action_id) = (notice.id.clone(), action.id.clone());
                self.text_action(
                    format!("notice-action-{ix}-{}", action.id),
                    action.label.clone(),
                    cx.listener(move |_, _, _, cx| {
                        cx.emit(ThreadViewEvent::NoticeAction {
                            notice_id: notice_id.clone(),
                            action_id: action_id.clone(),
                        })
                    }),
                )
            }))
            .into_any_element()
    }

    /// A quiet line that closes a turn: how it ended, how long it took and
    /// how much it did, with a retry on the newest one.
    pub(super) fn render_turn_end(
        &self,
        ix: usize,
        reason: &StopReason,
        tool_calls: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = self.theme;
        let elapsed = self.views[ix].elapsed.map(format_elapsed);
        let (icon, color, label) = match reason {
            StopReason::EndTurn => (
                IconName::Check,
                theme.text_subtle,
                elapsed.map_or_else(
                    || "Done".to_owned(),
                    |elapsed| format!("Worked for {elapsed}"),
                ),
            ),
            StopReason::Cancelled => (IconName::CircleX, theme.text_subtle, "Stopped".to_owned()),
            StopReason::MaxTokens => (
                IconName::TriangleAlert,
                theme.warning,
                "Stopped at the output limit".to_owned(),
            ),
            StopReason::Error { .. } => (IconName::CircleX, theme.danger, "Failed".to_owned()),
        };
        let tools = match tool_calls {
            0 => None,
            1 => Some("1 tool call".to_owned()),
            count => Some(format!("{count} tool calls")),
        };
        let latest =
            ix + 1 == self.thread.entries().len() && self.thread.status() == ThreadStatus::Idle;
        let replies = self.thread.entries()[..ix]
            .iter()
            .rev()
            .take_while(|entry| !matches!(entry, ThreadEntry::User { .. }))
            .filter_map(|entry| match entry {
                ThreadEntry::Assistant { chunks, .. } => Some(chunks),
                _ => None,
            })
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .flat_map(|chunks| {
                chunks.iter().filter_map(|chunk| match chunk {
                    AssistantChunk::Text(text) => Some(text.as_str()),
                    AssistantChunk::Thought(_) => None,
                })
            })
            .collect::<Vec<_>>()
            .join("\n\n");
        h_flex()
            .gap(px(6.))
            .text_size(self.ui_style.text.xsmall)
            .text_color(theme.text_subtle)
            .child(Icon::new(icon).size(px(12.)).text_color(color))
            .child(div().text_color(color).child(label))
            .children(tools.map(|tools| div().child(format!("· {tools}"))))
            .child(div().flex_1())
            .children(self.host_message_actions(
                ix,
                MessagePlacement::TurnEnd,
                self.thread.turn_message(ix).cloned(),
                replies.clone(),
                cx,
            ))
            .when(!replies.is_empty(), |this| {
                this.child(
                    self.text_action(format!("turn-copy-{ix}"), "Copy", move |_, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(replies.clone()))
                    }),
                )
            })
            .when(latest, |this| {
                this.child(self.text_action(
                    format!("turn-retry-{ix}"),
                    "Retry",
                    cx.listener(|_, _, _, cx| cx.emit(ThreadViewEvent::Retry)),
                ))
            })
            .into_any_element()
    }

    pub(super) fn render_empty(&self, cx: &mut Context<Self>) -> Div {
        let theme = self.theme;
        let ui_style = self.ui_style;
        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap(ui_style.spacing.sm)
            .px(ui_style.spacing.xl)
            .text_color(theme.text_muted)
            .child(
                Icon::new(IconName::Bot)
                    .size(px(28.))
                    .text_color(theme.icon_muted),
            )
            .child(
                div()
                    .text_size(ui_style.text.default)
                    .child("Ask the agent"),
            )
            .child(
                div()
                    .text_size(ui_style.text.small)
                    .text_color(theme.text_subtle)
                    .child("Type @ to mention a file, / for a command"),
            )
            .when(!self.suggestions.is_empty(), |this| {
                this.child(
                    v_flex()
                        .mt(ui_style.spacing.md)
                        .w_full()
                        .max_w(px(360.))
                        .gap(px(6.))
                        .children(self.suggestions.iter().enumerate().map(|(ix, suggestion)| {
                            let prompt = suggestion.clone();
                            h_flex()
                                .id(SharedString::from(format!("suggestion-{ix}")))
                                .gap(ui_style.spacing.sm)
                                .px(ui_style.spacing.md)
                                .py(ui_style.spacing.sm)
                                .rounded(ui_style.radius.card)
                                .border_1()
                                .border_color(theme.border)
                                .text_size(ui_style.text.small)
                                .text_color(theme.text)
                                .cursor_pointer()
                                .hover(move |this| this.bg(theme.ghost_element_hover))
                                .child(
                                    Icon::new(IconName::ArrowRight)
                                        .size(px(12.))
                                        .text_color(theme.icon_muted),
                                )
                                .child(div().flex_1().child(suggestion.clone()))
                                .on_click(cx.listener(move |_, _, _, cx| {
                                    cx.emit(ThreadViewEvent::Suggestion(prompt.clone()))
                                }))
                        })),
                )
            })
    }
}

/// Shared with the composer, which shows attachments before they are sent.
pub(crate) fn render_attachment_preview(
    attachment: &Attachment,
    theme: yttt_ui::theme::WorkbenchTheme,
    ui_style: yttt_ui::style::UiStyle,
) -> AnyElement {
    if attachment.kind.is_image() {
        let source: gpui::ImageSource = match &attachment.kind {
            AttachmentKind::Image { mime, data } => match super::image_format(mime) {
                Some(format) => {
                    std::sync::Arc::new(gpui::Image::from_bytes(format, data.clone())).into()
                }
                None => return file_chip(&attachment.name, theme, ui_style),
            },
            AttachmentKind::File { path } => std::path::PathBuf::from(path).into(),
        };
        return div()
            .size(px(ATTACHMENT_THUMBNAIL))
            .rounded(px(6.))
            .border_1()
            .border_color(theme.border)
            .overflow_hidden()
            .child(img(source).size_full().object_fit(ObjectFit::Cover))
            .into_any_element();
    }
    file_chip(&attachment.name, theme, ui_style)
}

fn file_chip(
    name: &str,
    theme: yttt_ui::theme::WorkbenchTheme,
    ui_style: yttt_ui::style::UiStyle,
) -> AnyElement {
    h_flex()
        .gap(px(5.))
        .px(px(8.))
        .py(px(4.))
        .rounded(px(6.))
        .border_1()
        .border_color(theme.border)
        .bg(theme.element_background)
        .text_size(ui_style.text.xsmall)
        .text_color(theme.text)
        .child(
            Icon::new(IconName::File)
                .size(px(12.))
                .text_color(theme.icon_muted),
        )
        .child(name.to_owned())
        .into_any_element()
}

fn format_elapsed(elapsed: Duration) -> String {
    let secs = elapsed.as_secs();
    if secs < 60 {
        format!("{secs}s")
    } else {
        format!("{}m {}s", secs / 60, secs % 60)
    }
}
