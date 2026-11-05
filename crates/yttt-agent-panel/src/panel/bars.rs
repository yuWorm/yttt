//! The strip above the composer: what the turn is doing, the plan, the files
//! changed, queued prompts and the message being edited. Each part shows
//! only while it has something to say.
use gpui::{
    AnyElement, ClickEvent, Context, FontWeight, IntoElement, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Window, div, prelude::*, px,
};
use gpui_component::{h_flex, shimmer::ShimmerText, v_flex};
use yttt_ui::{Icon, IconName, primitives::spinner::yttt_spinner};

use super::{AgentPanel, AgentPanelEvent};
use crate::{
    event::{Prompt, TodoStatus},
    model::{AssistantChunk, ThreadEntry, ThreadStatus},
    slots::StatusItem,
    thread_view::prompt_text,
};

impl AgentPanel {
    /// The built-in bars that have something to show and are not hidden,
    /// with their orders.
    pub(super) fn render_status_items(&self, cx: &mut Context<Self>) -> Vec<(i32, AnyElement)> {
        StatusItem::ALL
            .into_iter()
            .filter(|item| !self.hidden.contains(item))
            .filter_map(|item| {
                let element = match item {
                    StatusItem::Activity => self.render_activity(cx),
                    StatusItem::Plan => self.render_plan(cx),
                    StatusItem::Changes => self.render_changes(cx),
                    StatusItem::Queue => self.render_queue(cx),
                    StatusItem::Editing => self.render_editing(cx),
                }?;
                Some((item.order(), element))
            })
            .collect()
    }

    fn bar(&self) -> gpui::Div {
        v_flex()
            .w_full()
            .rounded(self.ui_style.radius.card)
            .border_1()
            .border_color(self.theme.border)
            .bg(self.theme.surface)
            .text_size(self.ui_style.text.small)
            .overflow_hidden()
    }

    fn bar_row(&self) -> gpui::Div {
        h_flex()
            .w_full()
            .min_h(px(28.))
            .gap(self.ui_style.spacing.sm)
            .px(self.ui_style.spacing.md)
    }

    fn link(
        &self,
        id: impl Into<SharedString>,
        label: impl Into<SharedString>,
        on_click: impl Fn(&ClickEvent, &mut Window, &mut gpui::App) + 'static,
    ) -> impl IntoElement {
        let theme = self.theme;
        div()
            .id(id.into())
            .flex_shrink_0()
            .px(px(6.))
            .py(px(1.))
            .rounded(px(4.))
            .text_size(self.ui_style.text.xsmall)
            .text_color(theme.text_muted)
            .cursor_pointer()
            .hover(move |this| this.bg(theme.ghost_element_hover).text_color(theme.text))
            .child(label.into())
            .on_click(on_click)
    }

    fn chevron(&self, open: bool) -> impl IntoElement {
        Icon::new(if open {
            IconName::ChevronDown
        } else {
            IconName::ChevronUp
        })
        .size(px(12.))
        .text_color(self.theme.icon_muted)
    }

    /// What the running turn is doing and for how long.
    pub(super) fn render_activity(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let thread = self.thread.read(cx);
        let status = thread.status();
        if status == ThreadStatus::Idle {
            return None;
        }
        let theme = self.theme;
        let label = match (status, thread.entries().last()) {
            (ThreadStatus::AwaitingPermission, _) => "Waiting for your approval",
            (_, Some(ThreadEntry::Assistant { chunks, .. }))
                if matches!(chunks.last(), Some(AssistantChunk::Thought(_))) =>
            {
                "Thinking"
            }
            (_, Some(ThreadEntry::Assistant { .. })) => "Writing",
            (_, Some(ThreadEntry::ToolCall(_))) => "Running tools",
            _ => "Working",
        };
        let elapsed = self
            .turn_started
            .map(|started| format!("{}s", started.elapsed().as_secs()));
        let awaiting = status == ThreadStatus::AwaitingPermission;
        Some(
            h_flex()
                .gap(self.ui_style.spacing.sm)
                .px(px(2.))
                .text_size(self.ui_style.text.xsmall)
                .text_color(theme.text_muted)
                .child(if awaiting {
                    Icon::new(IconName::TriangleAlert)
                        .size(px(12.))
                        .text_color(theme.warning)
                        .into_any_element()
                } else {
                    yttt_spinner(px(12.), theme.accent).into_any_element()
                })
                .child(if awaiting {
                    div()
                        .text_color(theme.warning)
                        .child(label)
                        .into_any_element()
                } else {
                    ShimmerText::new(format!("{label}…")).into_any_element()
                })
                .children(elapsed.map(|elapsed| div().text_color(theme.text_subtle).child(elapsed)))
                .child(div().flex_1())
                .child(div().text_color(theme.text_subtle).child("esc to stop"))
                .into_any_element(),
        )
    }

    /// The latest plan: its current step folded, every step open.
    pub(super) fn render_plan(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let thread = self.thread.read(cx);
        let items = thread.thread().latest_plan()?.to_vec();
        let done = items
            .iter()
            .filter(|item| item.status == TodoStatus::Completed)
            .count();
        if done == items.len() && thread.status() == ThreadStatus::Idle {
            return None;
        }
        let theme = self.theme;
        let current = items
            .iter()
            .find(|item| item.status == TodoStatus::InProgress)
            .or_else(|| items.iter().find(|item| item.status == TodoStatus::Pending))
            .map_or_else(|| "All steps done".to_owned(), |item| item.text.clone());
        let open = self.plan_open;
        Some(
            self.bar()
                .child(
                    self.bar_row()
                        .id("agent-plan")
                        .cursor_pointer()
                        .hover(move |this| this.bg(theme.ghost_element_hover))
                        .child(
                            Icon::new(IconName::LayoutDashboard)
                                .size(px(13.))
                                .text_color(theme.icon_muted),
                        )
                        .child(
                            div()
                                .flex_shrink_0()
                                .text_color(theme.text_muted)
                                .child("Plan"),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_color(theme.text)
                                .child(current),
                        )
                        .children(self.plan_actions.iter().map(|action| {
                            let action_id = action.id.clone();
                            self.link(
                                format!("agent-plan-action-{}", action.id),
                                action.label.clone(),
                                cx.listener(move |_, _, _, cx| {
                                    cx.emit(AgentPanelEvent::PlanAction {
                                        action_id: action_id.clone(),
                                    });
                                    cx.stop_propagation();
                                }),
                            )
                        }))
                        .child(
                            div()
                                .text_size(self.ui_style.text.xsmall)
                                .text_color(theme.text_subtle)
                                .child(format!("{done}/{}", items.len())),
                        )
                        .child(self.chevron(open))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.plan_open = !this.plan_open;
                            cx.notify();
                        })),
                )
                .when(open, |this| {
                    this.child(
                        v_flex()
                            .px(self.ui_style.spacing.md)
                            .pb(self.ui_style.spacing.sm)
                            .gap(px(4.))
                            .children(items.into_iter().enumerate().map(|(index, item)| {
                                let marker = match item.status {
                                    TodoStatus::Completed => Icon::new(IconName::CircleCheck)
                                        .size(px(13.))
                                        .text_color(theme.success)
                                        .into_any_element(),
                                    TodoStatus::InProgress => {
                                        yttt_spinner(px(12.), theme.accent).into_any_element()
                                    }
                                    TodoStatus::Pending => Icon::new(IconName::Square)
                                        .size(px(12.))
                                        .text_color(theme.icon_muted)
                                        .into_any_element(),
                                };
                                h_flex()
                                    .id(SharedString::from(format!("agent-plan-step-{index}")))
                                    .gap(self.ui_style.spacing.sm)
                                    .px(px(4.))
                                    .rounded(px(4.))
                                    .cursor_pointer()
                                    .hover(move |this| this.bg(theme.ghost_element_hover))
                                    .on_click(cx.listener(move |_, _, _, cx| {
                                        cx.emit(AgentPanelEvent::PlanStepClicked { index })
                                    }))
                                    .child(marker)
                                    .child(
                                        div()
                                            .map(|this| match item.status {
                                                TodoStatus::Completed => this
                                                    .line_through()
                                                    .text_color(theme.text_subtle),
                                                TodoStatus::InProgress => this
                                                    .text_color(theme.text)
                                                    .font_weight(FontWeight::MEDIUM),
                                                TodoStatus::Pending => {
                                                    this.text_color(theme.text_muted)
                                                }
                                            })
                                            .child(item.text),
                                    )
                            })),
                    )
                })
                .into_any_element(),
        )
    }

    /// Files the thread changed that are still waiting for a keep or undo.
    pub(super) fn render_changes(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let files = self.thread.read(cx).thread().changed_files();
        if files.is_empty() {
            return None;
        }
        let theme = self.theme;
        let open = self.changes_open;
        let added = files.iter().map(|file| file.added).sum::<usize>();
        let removed = files.iter().map(|file| file.removed).sum::<usize>();
        let all = files
            .iter()
            .map(|file| file.path.clone())
            .collect::<Vec<_>>();
        let undo_all = all.clone();
        let count = match files.len() {
            1 => "1 file changed".to_owned(),
            count => format!("{count} files changed"),
        };
        Some(
            self.bar()
                .child(
                    self.bar_row()
                        .id("agent-changes")
                        .cursor_pointer()
                        .hover(move |this| this.bg(theme.ghost_element_hover))
                        .child(
                            Icon::new(IconName::File)
                                .size(px(13.))
                                .text_color(theme.icon_muted),
                        )
                        .child(div().flex_shrink_0().text_color(theme.text).child(count))
                        .child(
                            h_flex()
                                .flex_1()
                                .gap(px(6.))
                                .text_size(self.ui_style.text.xsmall)
                                .child(div().text_color(theme.success).child(format!("+{added}")))
                                .child(div().text_color(theme.danger).child(format!("−{removed}"))),
                        )
                        .child(self.link(
                            "agent-changes-undo-all",
                            "Undo all",
                            cx.listener(move |this, _, _, cx| {
                                this.resolve_edits(undo_all.clone(), false, cx);
                                cx.stop_propagation();
                            }),
                        ))
                        .child(self.link(
                            "agent-changes-keep-all",
                            "Keep all",
                            cx.listener(move |this, _, _, cx| {
                                this.resolve_edits(all.clone(), true, cx);
                                cx.stop_propagation();
                            }),
                        ))
                        .child(self.chevron(open))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.changes_open = !this.changes_open;
                            cx.notify();
                        })),
                )
                .when(open, |this| {
                    this.child(
                        v_flex()
                            .pb(px(4.))
                            .children(files.into_iter().enumerate().map(|(ix, file)| {
                                let (dir, name) = file
                                    .path
                                    .rsplit_once('/')
                                    .map_or(("", file.path.as_str()), |(dir, name)| (dir, name));
                                let (open_path, keep, undo) =
                                    (file.path.clone(), file.path.clone(), file.path.clone());
                                self.bar_row()
                                    .min_h(px(24.))
                                    .text_size(self.ui_style.text.xsmall)
                                    .child(
                                        h_flex()
                                            .id(SharedString::from(format!("agent-change-{ix}")))
                                            .flex_1()
                                            .min_w_0()
                                            .gap(px(5.))
                                            .cursor_pointer()
                                            .hover(|this| this.underline())
                                            .child(
                                                div()
                                                    .flex_shrink_0()
                                                    .text_color(theme.text)
                                                    .child(name.to_owned()),
                                            )
                                            .child(
                                                div()
                                                    .min_w_0()
                                                    .truncate()
                                                    .text_color(theme.text_subtle)
                                                    .child(dir.to_owned()),
                                            )
                                            .on_click(cx.listener(move |_, _, _, cx| {
                                                cx.emit(AgentPanelEvent::OpenPath {
                                                    path: open_path.clone(),
                                                    line: None,
                                                })
                                            })),
                                    )
                                    .child(
                                        div()
                                            .text_color(theme.success)
                                            .child(format!("+{}", file.added)),
                                    )
                                    .child(
                                        div()
                                            .text_color(theme.danger)
                                            .child(format!("−{}", file.removed)),
                                    )
                                    .child(self.link(
                                        format!("agent-change-undo-{ix}"),
                                        "Undo",
                                        cx.listener(move |this, _, _, cx| {
                                            this.resolve_edits(vec![undo.clone()], false, cx)
                                        }),
                                    ))
                                    .child(self.link(
                                        format!("agent-change-keep-{ix}"),
                                        "Keep",
                                        cx.listener(move |this, _, _, cx| {
                                            this.resolve_edits(vec![keep.clone()], true, cx)
                                        }),
                                    ))
                            })),
                    )
                })
                .into_any_element(),
        )
    }

    /// Prompts waiting for the running turn to end, each removable.
    pub(super) fn render_queue(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.queue.is_empty() {
            return None;
        }
        let theme = self.theme;
        Some(
            self.bar()
                .children(self.queue.iter().enumerate().map(|(ix, prompt)| {
                    let attachments = prompt.attachments.len();
                    self.bar_row()
                        .child(
                            div()
                                .flex_shrink_0()
                                .px(px(6.))
                                .rounded(px(4.))
                                .bg(theme.element_background)
                                .text_size(self.ui_style.text.xsmall)
                                .text_color(theme.text_muted)
                                .child("Queued"),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_color(theme.text)
                                .child(prompt_text(&prompt.segments)),
                        )
                        .when(attachments > 0, |this| {
                            this.child(
                                div()
                                    .text_size(self.ui_style.text.xsmall)
                                    .text_color(theme.text_subtle)
                                    .child(match attachments {
                                        1 => "+1 file".to_owned(),
                                        count => format!("+{count} files"),
                                    }),
                            )
                        })
                        .child(
                            div()
                                .id(SharedString::from(format!("agent-queue-remove-{ix}")))
                                .cursor_pointer()
                                .child(
                                    Icon::new(IconName::Close)
                                        .size(px(12.))
                                        .text_color(theme.icon_muted),
                                )
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if ix < this.queue.len() {
                                        this.queue.remove(ix);
                                    }
                                    cx.notify();
                                })),
                        )
                }))
                .into_any_element(),
        )
    }

    /// Reminds that sending replaces an earlier message.
    pub(super) fn render_editing(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        self.editing.as_ref()?;
        let theme = self.theme;
        Some(
            self.bar_row()
                .rounded(self.ui_style.radius.card)
                .bg(theme.accent.alpha(0.1))
                .text_size(self.ui_style.text.xsmall)
                .text_color(theme.accent)
                .child(
                    Icon::new(IconName::Replace)
                        .size(px(12.))
                        .text_color(theme.accent),
                )
                .child(
                    div()
                        .flex_1()
                        .child("Editing an earlier message — sending reruns from there"),
                )
                .child(self.link(
                    "agent-editing-cancel",
                    "Cancel",
                    cx.listener(|this, _, window, cx| {
                        this.editing = None;
                        this.composer.update(cx, |composer, cx| {
                            composer.set_draft(&Prompt::default(), window, cx)
                        });
                        cx.notify();
                    }),
                ))
                .into_any_element(),
        )
    }
}
