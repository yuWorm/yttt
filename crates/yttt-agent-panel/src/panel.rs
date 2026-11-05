//! The whole panel: a header, the thread, and above the composer the state
//! of the turn — what is running, the plan, the files changed, queued
//! prompts and the message being edited.
mod bars;

use std::{rc::Rc, time::Instant};

use gpui::{
    AppContext as _, Context, Entity, EventEmitter, IntoElement, ParentElement as _, Render,
    SharedString, Styled as _, Subscription, Task, Window, div, prelude::*, px,
};
use gpui_base::input::Escape;
use gpui_component::{
    h_flex,
    input::{InputEvent, InputState},
    v_flex,
};
use yttt_ui::{
    Icon, IconName,
    primitives::{
        icon_button::{YtttIconButtonKind, yttt_icon_button},
        input::yttt_borderless_input,
    },
    style::UiStyle,
    theme::WorkbenchTheme,
};

use crate::{
    composer::{CandidateSource, Composer, ComposerEvent, SelectorKind, SelectorOption},
    event::{
        Action, MessageId, ModelInfo, NoticeId, PermissionOptionId, Prompt, PromptSegment,
        ThreadEvent, ToolCallId,
    },
    model::{ThreadEntry, ThreadStatus},
    slots::{
        CustomRender, MessageAction, Slot, SlotContext, SlotItem, Slots, StatusItem, merge_ordered,
    },
    thread_view::{ThreadView, ThreadViewEvent},
};

/// What the user asked for. Whoever runs the agent answers with
/// [`ThreadEvent`]s through [`AgentPanel::apply_event`].
#[derive(Clone, Debug, PartialEq)]
pub enum AgentPanelEvent {
    /// A prompt to run. `edit_of` names the message it rewrites, whose turn
    /// and everything after it the agent should drop first.
    Submit {
        prompt: Prompt,
        edit_of: Option<MessageId>,
    },
    PermissionChosen {
        tool_call_id: ToolCallId,
        option_id: PermissionOptionId,
    },
    Stop,
    /// A file the agent touched was clicked; `line` is 1-based.
    OpenPath {
        path: String,
        line: Option<u32>,
    },
    NewThread,
    /// Run the last turn again.
    Retry,
    /// The user accepted these files' changes as they are.
    KeepEdits {
        paths: Vec<String>,
    },
    /// The user wants these files' changes reverted.
    UndoEdits {
        paths: Vec<String>,
    },
    Selected {
        kind: SelectorKind,
        id: String,
    },
    /// A button on a notice.
    NoticeAction {
        notice_id: NoticeId,
        action_id: String,
    },
    /// A button on a tool card.
    ToolAction {
        tool_call_id: ToolCallId,
        action_id: String,
    },
    /// A host message action; `message_id` is the prompt it belongs to and
    /// `text` the prompt's or the turn's replies.
    MessageAction {
        action_id: String,
        message_id: Option<MessageId>,
        text: String,
    },
    /// A button on the plan bar.
    PlanAction {
        action_id: String,
    },
    /// A step in the open plan bar was clicked.
    PlanStepClicked {
        index: usize,
    },
    /// The user renamed the thread; `None` went back to the automatic
    /// title.
    TitleChanged {
        title: Option<String>,
    },
    /// The user picked a model. The agent switches and confirms with
    /// [`ThreadEvent::ActiveModel`].
    ModelSelected {
        id: String,
    },
    FavoriteModel {
        id: String,
        favorite: bool,
    },
    /// The user wants to configure a model, such as entering a key.
    ModelSetup {
        id: String,
    },
    ManageModels,
    /// The user asked for a fresh model list; answer with
    /// [`ThreadEvent::ModelCatalog`].
    RefreshModels,
}

pub struct AgentPanel {
    pub(crate) thread: Entity<ThreadView>,
    composer: Entity<Composer>,
    /// Prompts sent while a turn ran; the first goes out when it ends.
    queue: Vec<Prompt>,
    /// The message the draft rewrites.
    editing: Option<MessageId>,
    plan_open: bool,
    changes_open: bool,
    turn_started: Option<Instant>,
    slots: Slots,
    /// Built-in bars the host hid, usually to show its own instead.
    hidden: std::collections::HashSet<StatusItem>,
    plan_actions: Vec<Action>,
    /// The title being renamed in place, and its focus-loss watch.
    title_editor: Option<(Entity<InputState>, Subscription)>,
    /// Repaints the running turn's clock.
    _ticker: Option<Task<()>>,
    theme: WorkbenchTheme,
    ui_style: UiStyle,
    _subscriptions: [Subscription; 2],
}

impl EventEmitter<AgentPanelEvent> for AgentPanel {}

impl AgentPanel {
    pub fn new(
        source: Rc<dyn CandidateSource>,
        theme: WorkbenchTheme,
        ui_style: UiStyle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let thread = cx.new(|cx| ThreadView::new(theme, ui_style, cx));
        let composer = cx.new(|cx| Composer::new(source, theme, ui_style, window, cx));
        let subscriptions = [
            cx.subscribe(&composer, |this, _, event, cx| match event {
                ComposerEvent::Submit(prompt) => this.send(prompt.clone(), cx),
                ComposerEvent::Stop => cx.emit(AgentPanelEvent::Stop),
                ComposerEvent::Selected { kind, id } => cx.emit(AgentPanelEvent::Selected {
                    kind: *kind,
                    id: id.clone(),
                }),
                ComposerEvent::ModelSelected { id } => {
                    cx.emit(AgentPanelEvent::ModelSelected { id: id.clone() })
                }
                ComposerEvent::FavoriteModel { id, favorite } => {
                    cx.emit(AgentPanelEvent::FavoriteModel {
                        id: id.clone(),
                        favorite: *favorite,
                    })
                }
                ComposerEvent::ModelSetup { id } => {
                    cx.emit(AgentPanelEvent::ModelSetup { id: id.clone() })
                }
                ComposerEvent::ManageModels => cx.emit(AgentPanelEvent::ManageModels),
                ComposerEvent::RefreshModels => cx.emit(AgentPanelEvent::RefreshModels),
            }),
            cx.subscribe_in(&thread, window, |this, _, event, window, cx| match event {
                ThreadViewEvent::PermissionChosen {
                    tool_call_id,
                    option_id,
                } => cx.emit(AgentPanelEvent::PermissionChosen {
                    tool_call_id: tool_call_id.clone(),
                    option_id: option_id.clone(),
                }),
                ThreadViewEvent::OpenPath { path, line } => cx.emit(AgentPanelEvent::OpenPath {
                    path: path.clone(),
                    line: *line,
                }),
                ThreadViewEvent::EditMessage { id } => this.edit_message(id, window, cx),
                ThreadViewEvent::Retry => cx.emit(AgentPanelEvent::Retry),
                ThreadViewEvent::NoticeAction {
                    notice_id,
                    action_id,
                } => cx.emit(AgentPanelEvent::NoticeAction {
                    notice_id: notice_id.clone(),
                    action_id: action_id.clone(),
                }),
                ThreadViewEvent::ToolAction {
                    tool_call_id,
                    action_id,
                } => cx.emit(AgentPanelEvent::ToolAction {
                    tool_call_id: tool_call_id.clone(),
                    action_id: action_id.clone(),
                }),
                ThreadViewEvent::MessageAction {
                    action_id,
                    message_id,
                    text,
                } => cx.emit(AgentPanelEvent::MessageAction {
                    action_id: action_id.clone(),
                    message_id: message_id.clone(),
                    text: text.clone(),
                }),
                ThreadViewEvent::Suggestion(text) => {
                    let prompt = Prompt {
                        segments: vec![PromptSegment::Text(text.clone())],
                        attachments: Vec::new(),
                    };
                    this.composer
                        .update(cx, |composer, cx| composer.set_draft(&prompt, window, cx));
                }
            }),
        ];
        Self {
            thread,
            composer,
            queue: Vec::new(),
            editing: None,
            plan_open: false,
            changes_open: false,
            turn_started: None,
            slots: Slots::default(),
            hidden: Default::default(),
            plan_actions: Vec::new(),
            title_editor: None,
            _ticker: None,
            theme,
            ui_style,
            _subscriptions: subscriptions,
        }
    }

    pub fn apply_event(&mut self, event: ThreadEvent, cx: &mut Context<Self>) {
        match &event {
            ThreadEvent::ModelCatalog { models } => {
                let models = models.clone();
                self.composer
                    .update(cx, |composer, cx| composer.set_models(models, cx));
            }
            ThreadEvent::ActiveModel { id } => {
                let id = id.clone();
                self.composer
                    .update(cx, |composer, cx| composer.set_active_model(&id, cx));
            }
            _ => {}
        }
        let (status, usage) = self.thread.update(cx, |thread, cx| {
            thread.apply(event, cx);
            (thread.status(), thread.thread().usage())
        });
        self.composer.update(cx, |composer, cx| {
            composer.set_status(status, cx);
            composer.set_usage(usage, cx);
        });
        match (status, self.turn_started) {
            (ThreadStatus::Idle, Some(_)) => {
                self.turn_started = None;
                self._ticker = None;
                if !self.queue.is_empty() {
                    let prompt = self.queue.remove(0);
                    cx.emit(AgentPanelEvent::Submit {
                        prompt,
                        edit_of: None,
                    });
                }
            }
            (ThreadStatus::Generating | ThreadStatus::AwaitingPermission, None) => {
                self.turn_started = Some(Instant::now());
                self._ticker = Some(cx.spawn(async move |this, cx| {
                    loop {
                        cx.background_executor()
                            .timer(std::time::Duration::from_secs(1))
                            .await;
                        if this.update(cx, |_, cx| cx.notify()).is_err() {
                            break;
                        }
                    }
                }));
            }
            _ => {}
        }
        self.push_slots(cx);
        cx.notify();
    }

    pub fn status(&self, cx: &gpui::App) -> ThreadStatus {
        self.thread.read(cx).status()
    }

    pub fn clear(&mut self, cx: &mut Context<Self>) {
        self.thread.update(cx, |thread, cx| thread.clear(cx));
        self.composer.update(cx, |composer, cx| {
            composer.set_status(ThreadStatus::Idle, cx);
            composer.set_usage(None, cx);
        });
        self.queue.clear();
        self.editing = None;
        self.turn_started = None;
        self._ticker = None;
        self.push_slots(cx);
        cx.notify();
    }

    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.composer
            .update(cx, |composer, cx| composer.focus(window, cx));
    }

    /// Prompts the empty thread offers to start with.
    pub fn set_suggestions(&self, suggestions: Vec<String>, cx: &mut Context<Self>) {
        self.thread
            .update(cx, |thread, cx| thread.set_suggestions(suggestions, cx));
    }

    /// The choices for the composer's mode or model picker.
    pub fn set_options(
        &self,
        kind: SelectorKind,
        options: Vec<SelectorOption>,
        selected: usize,
        cx: &mut Context<Self>,
    ) {
        self.composer.update(cx, |composer, cx| {
            composer.set_options(kind, options, selected, cx)
        });
    }

    /// The thread's title as shown: the user's, the agent's or the first
    /// prompt's.
    pub fn title(&self, cx: &gpui::App) -> Option<String> {
        self.thread.read(cx).thread().title()
    }

    /// Sets the title as if the user typed it, without reporting it back;
    /// `None` returns to the automatic title.
    pub fn set_title(&mut self, title: Option<String>, cx: &mut Context<Self>) {
        self.thread
            .update(cx, |thread, cx| thread.set_title(title, cx));
        self.push_slots(cx);
        cx.notify();
    }

    /// Replaces the model list, the same as [`ThreadEvent::ModelCatalog`].
    pub fn set_models(&self, models: Vec<ModelInfo>, cx: &mut Context<Self>) {
        self.composer
            .update(cx, |composer, cx| composer.set_models(models, cx));
    }

    /// Marks the model in use, the same as [`ThreadEvent::ActiveModel`].
    pub fn set_active_model(&self, id: &str, cx: &mut Context<Self>) {
        self.composer
            .update(cx, |composer, cx| composer.set_active_model(id, cx));
    }

    pub fn set_favorite_models(&self, ids: Vec<String>, cx: &mut Context<Self>) {
        self.composer
            .update(cx, |composer, cx| composer.set_favorite_models(ids, cx));
    }

    pub fn active_model(&self, cx: &gpui::App) -> Option<ModelInfo> {
        self.composer.read(cx).catalog().active().cloned()
    }

    pub fn models(&self, cx: &gpui::App) -> Vec<ModelInfo> {
        self.composer.read(cx).catalog().models().to_vec()
    }

    /// Opens the model picker, as for a keyboard shortcut.
    pub fn open_model_picker(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.composer
            .update(cx, |composer, cx| composer.open_model_picker(window, cx));
    }

    /// Turns the header title into a text box holding the current title.
    pub fn start_title_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let current = self.title(cx).unwrap_or_default();
        let editor = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(current)
                .placeholder("Thread title")
        });
        editor.update(cx, |state, cx| {
            state.focus(window, cx);
            state.select_all(window, cx);
        });
        let subscription = cx.subscribe_in(&editor, window, |this, _, event, window, cx| {
            if matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                this.finish_title_edit(true, window, cx);
            }
        });
        self.title_editor = Some((editor, subscription));
        cx.notify();
    }

    /// Saves or drops the rename; a blank title goes back to automatic.
    fn finish_title_edit(&mut self, save: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some((editor, _)) = self.title_editor.take() else {
            return;
        };
        if save {
            let typed = editor.read(cx).value().to_string();
            self.thread
                .update(cx, |thread, cx| thread.set_title(Some(typed), cx));
            let title = self
                .thread
                .read(cx)
                .thread()
                .has_user_title()
                .then(|| self.title(cx))
                .flatten();
            cx.emit(AgentPanelEvent::TitleChanged { title });
            self.push_slots(cx);
        }
        self.focus(window, cx);
        cx.notify();
    }

    /// Save or cancel beside the title being renamed. They act on mouse
    /// down without taking focus, so the rename is not first saved by the
    /// text box losing focus.
    fn title_button(
        &self,
        id: &'static str,
        icon: IconName,
        save: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = self.theme;
        div()
            .id(id)
            .flex()
            .flex_shrink_0()
            .items_center()
            .justify_center()
            .size(px(22.))
            .rounded(px(4.))
            .cursor_pointer()
            .hover(move |this| this.bg(theme.ghost_element_hover))
            .child(Icon::new(icon).size(px(13.)).text_color(if save {
                theme.accent
            } else {
                theme.icon_muted
            }))
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(move |this, _, window, cx| {
                    window.prevent_default();
                    this.finish_title_edit(save, window, cx);
                    cx.stop_propagation();
                }),
            )
    }

    /// Adds a host control to `slot`, replacing the item with the same id.
    pub fn insert_slot(&mut self, slot: Slot, item: SlotItem, cx: &mut Context<Self>) {
        self.slots.insert(slot, item);
        self.push_slots(cx);
        cx.notify();
    }

    pub fn remove_slot(&mut self, slot: Slot, id: &str, cx: &mut Context<Self>) {
        self.slots.remove(slot, id);
        self.push_slots(cx);
        cx.notify();
    }

    /// Shows or hides a built-in bar above the composer.
    pub fn set_status_item_visible(
        &mut self,
        item: StatusItem,
        visible: bool,
        cx: &mut Context<Self>,
    ) {
        if visible {
            self.hidden.remove(&item);
        } else {
            self.hidden.insert(item);
        }
        cx.notify();
    }

    /// Buttons on the plan bar; clicks report [`AgentPanelEvent::PlanAction`].
    pub fn set_plan_actions(&mut self, actions: Vec<Action>, cx: &mut Context<Self>) {
        self.plan_actions = actions;
        cx.notify();
    }

    /// Draws custom tool content of `kind` with `render`.
    pub fn set_custom_renderer(
        &self,
        kind: impl Into<String>,
        render: CustomRender,
        cx: &mut Context<Self>,
    ) {
        let kind = kind.into();
        self.thread.update(cx, |thread, cx| {
            thread.set_custom_renderer(kind, render, cx)
        });
    }

    /// Host buttons on prompts and turn ends; clicks report
    /// [`AgentPanelEvent::MessageAction`].
    pub fn set_message_actions(&self, actions: Vec<MessageAction>, cx: &mut Context<Self>) {
        self.thread
            .update(cx, |thread, cx| thread.set_message_actions(actions, cx));
    }

    /// What slot renderers see of the thread.
    pub fn slot_context(&self, cx: &gpui::App) -> SlotContext {
        let thread = self.thread.read(cx).thread();
        SlotContext {
            status: thread.status(),
            usage: thread.usage(),
            title: thread.title(),
            plan: thread.latest_plan().map(<[_]>::to_vec),
            changed_files: thread.changed_files(),
        }
    }

    /// The composer renders its footer slots itself, from this copy.
    fn push_slots(&self, cx: &mut Context<Self>) {
        let (slots, context) = (self.slots.clone(), self.slot_context(cx));
        self.composer
            .update(cx, |composer, cx| composer.set_slots(slots, context, cx));
    }

    /// Opens or folds every tool card; `None` lets each card decide.
    pub fn set_cards_open(&self, open: Option<bool>, cx: &mut Context<Self>) {
        self.thread
            .update(cx, |thread, cx| thread.set_cards_open(open, cx));
    }

    pub fn scroll_thread_to_top(&self, cx: &mut Context<Self>) {
        self.thread
            .update(cx, |thread, cx| thread.scroll_to_top(cx));
    }

    /// Scrolls the thread down; false once its end is already showing.
    pub fn scroll_thread_down(&self, distance: gpui::Pixels, cx: &mut Context<Self>) -> bool {
        self.thread
            .update(cx, |thread, cx| thread.scroll_down(distance, cx))
    }

    /// Sends now when the thread is free, queues behind the running turn
    /// otherwise.
    fn send(&mut self, prompt: Prompt, cx: &mut Context<Self>) {
        if self.status(cx) == ThreadStatus::Idle && self.queue.is_empty() {
            cx.emit(AgentPanelEvent::Submit {
                prompt,
                edit_of: self.editing.take(),
            });
        } else {
            self.queue.push(prompt);
        }
        cx.notify();
    }

    fn edit_message(&mut self, id: &MessageId, window: &mut Window, cx: &mut Context<Self>) {
        let prompt = self
            .thread
            .read(cx)
            .entries()
            .iter()
            .find_map(|entry| match entry {
                ThreadEntry::User {
                    id: entry_id,
                    content,
                    attachments,
                } if entry_id == id => Some(Prompt {
                    segments: content.clone(),
                    attachments: attachments.clone(),
                }),
                _ => None,
            });
        let Some(prompt) = prompt else {
            return;
        };
        self.editing = Some(id.clone());
        self.composer
            .update(cx, |composer, cx| composer.set_draft(&prompt, window, cx));
        cx.notify();
    }

    fn resolve_edits(&mut self, paths: Vec<String>, keep: bool, cx: &mut Context<Self>) {
        self.thread
            .update(cx, |thread, cx| thread.resolve_edits(&paths, cx));
        self.push_slots(cx);
        cx.emit(if keep {
            AgentPanelEvent::KeepEdits { paths }
        } else {
            AgentPanelEvent::UndoEdits { paths }
        });
        cx.notify();
    }
}

impl Render for AgentPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.theme;
        let spacing = self.ui_style.spacing;
        let context = self.slot_context(cx);
        let title = context
            .title
            .clone()
            .unwrap_or_else(|| "New thread".to_owned());
        let slots = self.slots.clone();
        let header_start = slots.render(Slot::HeaderStart, &context, window, cx);
        let header_end = slots.render(Slot::HeaderEnd, &context, window, cx);
        let status_bars = merge_ordered(
            self.render_status_items(cx),
            slots.render_ordered(Slot::StatusBar, &context, window, cx),
        );
        v_flex()
            .size_full()
            .bg(theme.panel_background)
            .text_color(theme.text)
            .child(
                h_flex()
                    .flex_shrink_0()
                    .h(px(36.))
                    .capture_action(cx.listener(|this, _: &Escape, window, cx| {
                        if this.title_editor.is_some() {
                            this.finish_title_edit(false, window, cx);
                            cx.stop_propagation();
                        }
                    }))
                    .gap(spacing.sm)
                    .pl(spacing.lg)
                    .pr(spacing.sm)
                    .border_b_1()
                    .border_color(theme.border)
                    .text_size(self.ui_style.text.small)
                    .child(
                        Icon::new(IconName::Bot)
                            .size(px(15.))
                            .text_color(theme.icon_muted),
                    )
                    .map(|header| match &self.title_editor {
                        // Renamed in place: the text stays where the title was,
                        // and save and cancel stand in for the header buttons.
                        Some((editor, _)) => header
                            .child(
                                div().flex_1().min_w_0().px(px(4.)).child(
                                    yttt_borderless_input(editor)
                                        .text_size(self.ui_style.text.small)
                                        .p_0(),
                                ),
                            )
                            .child(self.title_button("agent-title-save", IconName::Check, true, cx))
                            .child(self.title_button(
                                "agent-title-cancel",
                                IconName::Close,
                                false,
                                cx,
                            )),
                        None => header
                            .child(
                                div()
                                    .id("agent-title")
                                    .min_w_0()
                                    .truncate()
                                    .px(px(4.))
                                    .rounded(px(4.))
                                    .cursor_pointer()
                                    .hover(move |this| this.bg(theme.ghost_element_hover))
                                    .child(SharedString::from(title))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.start_title_edit(window, cx)
                                    })),
                            )
                            .children(header_start)
                            .child(div().flex_1())
                            .children(header_end)
                            .child(yttt_icon_button(
                                "agent-new-thread",
                                IconName::Plus,
                                YtttIconButtonKind::Toolbar,
                                theme,
                                self.ui_style,
                                cx.listener(|this, _, window, cx| {
                                    this.clear(cx);
                                    this.focus(window, cx);
                                    cx.emit(AgentPanelEvent::NewThread);
                                }),
                            )),
                    }),
            )
            .child(div().flex_1().min_h_0().child(self.thread.clone()))
            .child(
                v_flex()
                    .flex_shrink_0()
                    .gap(spacing.xs)
                    .px(spacing.lg)
                    .pt(spacing.sm)
                    .pb(spacing.lg)
                    .children(status_bars)
                    .child(self.composer.clone()),
            )
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use gpui::{TestAppContext, VisualTestContext};

    use super::*;
    use crate::{
        demo::DemoCandidates,
        event::{StopReason, ToolCallFields, ToolCallStatus, ToolContent, ToolKind},
    };

    fn panel(
        cx: &mut TestAppContext,
    ) -> (
        Entity<AgentPanel>,
        Rc<RefCell<Vec<AgentPanelEvent>>>,
        &mut VisualTestContext,
    ) {
        cx.update(yttt_ui::init);
        let slot = Rc::new(RefCell::new(None));
        let panel_slot = slot.clone();
        let (_root, cx) = cx.add_window_view(|window, cx| {
            let panel = cx.new(|cx| {
                AgentPanel::new(
                    Rc::new(DemoCandidates),
                    WorkbenchTheme::zed_defaults(true),
                    UiStyle::default(),
                    window,
                    cx,
                )
            });
            *panel_slot.borrow_mut() = Some(panel.clone());
            yttt_ui::window::yttt_root(panel, window, cx)
        });
        let panel = slot.borrow_mut().take().expect("window built the panel");
        let events = Rc::new(RefCell::new(Vec::new()));
        let sink = events.clone();
        cx.update(|window, cx| {
            cx.subscribe(&panel, move |_, event: &AgentPanelEvent, _| {
                sink.borrow_mut().push(event.clone())
            })
            .detach();
            panel.update(cx, |panel, cx| panel.focus(window, cx));
        });
        (panel, events, cx)
    }

    fn user_message(id: &str, text: &str) -> ThreadEvent {
        ThreadEvent::UserMessage {
            id: id.into(),
            content: vec![PromptSegment::Text(text.to_owned())],
            attachments: Vec::new(),
        }
    }

    fn submitted(events: &RefCell<Vec<AgentPanelEvent>>) -> Vec<(String, Option<MessageId>)> {
        events
            .borrow()
            .iter()
            .filter_map(|event| match event {
                AgentPanelEvent::Submit { prompt, edit_of } => Some((
                    crate::thread_view::prompt_text(&prompt.segments),
                    edit_of.clone(),
                )),
                _ => None,
            })
            .collect()
    }

    #[gpui::test]
    fn a_prompt_sent_mid_turn_waits_for_the_turn_to_end(cx: &mut TestAppContext) {
        let (panel, events, cx) = panel(cx);
        panel.update(cx, |panel, cx| {
            panel.apply_event(user_message("u1", "first"), cx)
        });
        cx.simulate_input("next");
        cx.simulate_keystrokes("enter");
        assert!(submitted(&events).is_empty());
        panel.read_with(cx, |panel, _| assert_eq!(panel.queue.len(), 1));

        panel.update(cx, |panel, cx| {
            panel.apply_event(
                ThreadEvent::Stop {
                    reason: StopReason::EndTurn,
                },
                cx,
            )
        });
        cx.run_until_parked();
        assert_eq!(submitted(&events), [("next".to_owned(), None)]);
        panel.read_with(cx, |panel, _| assert!(panel.queue.is_empty()));
    }

    #[gpui::test]
    fn editing_a_message_resends_it_in_place(cx: &mut TestAppContext) {
        let (panel, events, cx) = panel(cx);
        panel.update(cx, |panel, cx| {
            panel.apply_event(user_message("u1", "hello"), cx);
            panel.apply_event(
                ThreadEvent::Stop {
                    reason: StopReason::EndTurn,
                },
                cx,
            );
        });
        panel.update_in(cx, |panel, window, cx| {
            panel.edit_message(&"u1".into(), window, cx)
        });
        cx.simulate_input(" again");
        cx.simulate_keystrokes("enter");
        assert_eq!(
            submitted(&events),
            [("hello again".to_owned(), Some("u1".into()))]
        );
        panel.read_with(cx, |panel, _| assert!(panel.editing.is_none()));
    }

    #[gpui::test]
    fn keeping_a_file_takes_it_off_the_changes(cx: &mut TestAppContext) {
        let (panel, events, cx) = panel(cx);
        panel.update(cx, |panel, cx| {
            panel.apply_event(
                ThreadEvent::ToolCall {
                    id: "e1".into(),
                    kind: ToolKind::Edit,
                    title: "Edit".to_owned(),
                    status: ToolCallStatus::Running,
                    target: None,
                },
                cx,
            );
            panel.apply_event(
                ThreadEvent::ToolCallUpdate {
                    id: "e1".into(),
                    fields: ToolCallFields {
                        status: Some(ToolCallStatus::Completed),
                        content: Some(vec![ToolContent::Diff {
                            path: "a.rs".to_owned(),
                            old_text: Some("a\n".to_owned()),
                            new_text: "b\n".to_owned(),
                        }]),
                        ..Default::default()
                    },
                },
                cx,
            );
        });
        let changed = |panel: &Entity<AgentPanel>, cx: &mut VisualTestContext| {
            panel.read_with(cx, |panel, cx| {
                panel.thread.read(cx).thread().changed_files().len()
            })
        };
        assert_eq!(changed(&panel, cx), 1);
        panel.update(cx, |panel, cx| {
            panel.resolve_edits(vec!["a.rs".to_owned()], true, cx)
        });
        assert_eq!(changed(&panel, cx), 0);
        assert!(events.borrow().contains(&AgentPanelEvent::KeepEdits {
            paths: vec!["a.rs".to_owned()],
        }));
    }

    #[gpui::test]
    fn a_hidden_built_in_bar_leaves_the_others_in_order(cx: &mut TestAppContext) {
        let (panel, _, cx) = panel(cx);
        panel.update(cx, |panel, cx| {
            panel.apply_event(user_message("u1", "go"), cx);
            panel.apply_event(
                ThreadEvent::ToolCall {
                    id: "p".into(),
                    kind: ToolKind::Todo,
                    title: "Plan".to_owned(),
                    status: ToolCallStatus::Completed,
                    target: None,
                },
                cx,
            );
            panel.apply_event(
                ThreadEvent::ToolCallUpdate {
                    id: "p".into(),
                    fields: ToolCallFields {
                        content: Some(vec![ToolContent::Todo(vec![crate::event::TodoItem {
                            text: "step".to_owned(),
                            status: crate::event::TodoStatus::Pending,
                        }])]),
                        ..Default::default()
                    },
                },
                cx,
            );
        });
        let orders = |panel: &Entity<AgentPanel>, cx: &mut VisualTestContext| {
            panel.update(cx, |panel, cx| {
                panel
                    .render_status_items(cx)
                    .into_iter()
                    .map(|(order, _)| order)
                    .collect::<Vec<_>>()
            })
        };
        assert_eq!(
            orders(&panel, cx),
            [StatusItem::Activity.order(), StatusItem::Plan.order()]
        );
        panel.update(cx, |panel, cx| {
            panel.set_status_item_visible(StatusItem::Plan, false, cx)
        });
        assert_eq!(orders(&panel, cx), [StatusItem::Activity.order()]);
    }

    #[gpui::test]
    fn renaming_the_title_reports_it_and_a_blank_one_goes_back(cx: &mut TestAppContext) {
        let (panel, events, cx) = panel(cx);
        panel.update(cx, |panel, cx| {
            panel.apply_event(user_message("u1", "fix the build"), cx)
        });
        panel.update_in(cx, |panel, window, cx| panel.start_title_edit(window, cx));
        cx.simulate_input("Build fix");
        cx.simulate_keystrokes("enter");
        assert_eq!(
            panel.read_with(cx, |panel, cx| panel.title(cx)).as_deref(),
            Some("Build fix")
        );
        assert!(events.borrow().contains(&AgentPanelEvent::TitleChanged {
            title: Some("Build fix".to_owned()),
        }));

        panel.update_in(cx, |panel, window, cx| panel.start_title_edit(window, cx));
        cx.simulate_keystrokes("backspace enter");
        assert_eq!(
            panel.read_with(cx, |panel, cx| panel.title(cx)).as_deref(),
            Some("fix the build")
        );
        assert!(
            events
                .borrow()
                .contains(&AgentPanelEvent::TitleChanged { title: None })
        );
    }

    #[gpui::test]
    fn escape_keeps_the_title(cx: &mut TestAppContext) {
        let (panel, events, cx) = panel(cx);
        panel.update(cx, |panel, cx| {
            panel.apply_event(user_message("u1", "fix the build"), cx)
        });
        panel.update_in(cx, |panel, window, cx| panel.start_title_edit(window, cx));
        cx.simulate_input("Nope");
        cx.simulate_keystrokes("escape");
        assert_eq!(
            panel.read_with(cx, |panel, cx| panel.title(cx)).as_deref(),
            Some("fix the build")
        );
        assert!(
            !events
                .borrow()
                .iter()
                .any(|event| matches!(event, AgentPanelEvent::TitleChanged { .. }))
        );
    }

    #[gpui::test]
    fn the_agent_keeps_the_model_list_and_choice_in_sync(cx: &mut TestAppContext) {
        let (panel, _, cx) = panel(cx);
        panel.update(cx, |panel, cx| {
            panel.apply_event(
                ThreadEvent::ModelCatalog {
                    models: crate::demo::demo_models(false),
                },
                cx,
            );
            panel.apply_event(ThreadEvent::ActiveModel { id: "fast".into() }, cx);
        });
        panel.read_with(cx, |panel, cx| {
            assert_eq!(panel.models(cx).len(), 5);
            assert_eq!(
                panel.active_model(cx).map(|m| m.id),
                Some("fast".to_owned())
            );
        });
    }
}
