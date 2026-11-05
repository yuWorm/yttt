//! The prompt box. Mentions are atomic tokens in a [`TextareaState`]; that
//! state has no completion menu of its own (completion belongs to the code
//! editor mode, which has no tokens), so `@` and `/` candidates are this
//! view's popover, and it takes the arrow, Tab and Escape keys while the
//! popover is open. It always takes plain Enter, to send.
//!
//! Attachments come from pasting an image, dropping files or the attach
//! button. Up and Down in an empty box walk back through sent prompts.
mod footer;
mod model_picker;
mod popover;
pub mod trigger;

use std::{path::Path, rc::Rc};

use gpui::{
    AppContext as _, ClickEvent, ClipboardEntry, Context, Entity, EventEmitter, ExternalPaths,
    Focusable as _, IntoElement, ParentElement as _, PathPromptOptions, Render, SharedString,
    Styled as _, Subscription, Window, div, prelude::*, px,
};
use gpui_base::input::{
    Enter, Escape, IndentInline, InlineToken, InputContent, MoveDown, MoveUp, Paste,
};
use gpui_component::{
    h_flex,
    input::{InputEvent, InputState, Textarea, TextareaState},
    v_flex,
};
use yttt_ui::{
    Icon, IconName,
    primitives::icon_button::{YtttIconButtonKind, yttt_icon_button_style},
    style::UiStyle,
    theme::WorkbenchTheme,
};

use self::trigger::{TokenRange, Trigger, TriggerKind, detect_trigger, prompt_segments};
use crate::{
    event::{Attachment, AttachmentKind, Prompt, PromptSegment},
    model::{ThreadStatus, Usage},
    models::ModelCatalog,
    slots::{Slot, SlotContext, Slots},
};

const MAX_CANDIDATES: usize = 8;
const MAX_HISTORY: usize = 50;

/// Something to mention or a command to run. A mention's `id` names the
/// resource (`file:///…`); a command's `label` is its name without `/`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub id: String,
    pub label: String,
    pub detail: Option<String>,
}

/// Supplies `@` and `/` candidates for a query, best first.
pub trait CandidateSource {
    fn candidates(&self, kind: TriggerKind, query: &str) -> Vec<Candidate>;
}

/// One choice in the mode or model menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelectorOption {
    pub id: String,
    pub label: String,
    pub detail: Option<String>,
}

/// A picker the host fills with options, beside the model picker.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectorKind {
    Mode,
}

/// Which menu hangs above the prompt box.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OpenMenu {
    Selector(SelectorKind),
    Models,
}

#[derive(Clone, Debug, Default)]
struct Selector {
    options: Vec<SelectorOption>,
    selected: usize,
}

pub enum ComposerEvent {
    /// Sent whatever the thread is doing; the panel queues it if busy.
    Submit(Prompt),
    Stop,
    Selected {
        kind: SelectorKind,
        id: String,
    },
    /// The user picked a model; the agent confirms with `ActiveModel`.
    ModelSelected {
        id: String,
    },
    FavoriteModel {
        id: String,
        favorite: bool,
    },
    /// The user wants to configure a model that needs setup.
    ModelSetup {
        id: String,
    },
    ManageModels,
    /// The user asked for a fresh model list.
    RefreshModels,
}

pub struct Composer {
    input: Entity<TextareaState>,
    source: Rc<dyn CandidateSource>,
    popover: Option<Popover>,
    /// The trigger the user dismissed with Escape; it stays closed until a
    /// different trigger opens.
    dismissed: Option<usize>,
    attachments: Vec<Attachment>,
    /// Sent prompts, oldest first, and which one Up last recalled.
    history: Vec<Prompt>,
    recalled: Option<usize>,
    modes: Selector,
    catalog: ModelCatalog,
    model_filter: Entity<InputState>,
    /// The picker row Up, Down and Enter act on.
    model_highlight: usize,
    open_menu: Option<OpenMenu>,
    /// The host's footer controls and what they see of the thread.
    slots: Slots,
    slot_context: SlotContext,
    usage: Option<Usage>,
    status: ThreadStatus,
    theme: WorkbenchTheme,
    ui_style: UiStyle,
    _subscriptions: [Subscription; 2],
}

struct Popover {
    trigger: Trigger,
    candidates: Vec<Candidate>,
    selected: usize,
}

impl EventEmitter<ComposerEvent> for Composer {}

impl Composer {
    pub fn new(
        source: Rc<dyn CandidateSource>,
        theme: WorkbenchTheme,
        ui_style: UiStyle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(1, 8)
                .placeholder("Message the agent — @ to mention, / for commands")
        });
        let model_filter = cx.new(|cx| InputState::new(window, cx).placeholder("Select a model…"));
        let subscriptions = [
            cx.subscribe(&input, |this, _, event, cx| {
                if let InputEvent::Change = event {
                    this.refresh_popover(cx);
                }
            }),
            cx.subscribe(&model_filter, |this, _, event, cx| {
                if let InputEvent::Change = event {
                    this.reset_model_highlight(cx);
                    cx.notify();
                }
            }),
        ];
        Self {
            input,
            source,
            popover: None,
            dismissed: None,
            attachments: Vec::new(),
            history: Vec::new(),
            recalled: None,
            modes: Selector::default(),
            catalog: ModelCatalog::default(),
            model_filter,
            model_highlight: 0,
            open_menu: None,
            slots: Slots::default(),
            slot_context: SlotContext::default(),
            usage: None,
            status: ThreadStatus::Idle,
            theme,
            ui_style,
            _subscriptions: subscriptions,
        }
    }

    pub fn set_status(&mut self, status: ThreadStatus, cx: &mut Context<Self>) {
        if self.status != status {
            self.status = status;
            cx.notify();
        }
    }

    pub fn set_usage(&mut self, usage: Option<Usage>, cx: &mut Context<Self>) {
        if self.usage != usage {
            self.usage = usage;
            cx.notify();
        }
    }

    /// The choices for the mode or model menu; `selected` indexes them.
    pub fn set_options(
        &mut self,
        kind: SelectorKind,
        options: Vec<SelectorOption>,
        selected: usize,
        cx: &mut Context<Self>,
    ) {
        let selected = selected.min(options.len().saturating_sub(1));
        *self.selector_mut(kind) = Selector { options, selected };
        cx.notify();
    }

    /// The host's footer controls, and the thread snapshot they render from.
    pub fn set_slots(&mut self, slots: Slots, context: SlotContext, cx: &mut Context<Self>) {
        self.slots = slots;
        self.slot_context = context;
        cx.notify();
    }

    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.input.update(cx, |state, cx| state.focus(window, cx));
    }

    /// Replaces the draft, mentions restored as tokens.
    pub fn set_draft(&mut self, prompt: &Prompt, window: &mut Window, cx: &mut Context<Self>) {
        let mut text = String::new();
        let mut tokens = Vec::new();
        for segment in &prompt.segments {
            match segment {
                PromptSegment::Text(segment) => text.push_str(segment),
                PromptSegment::Mention { id, label } => {
                    let start = text.len();
                    text.push('@');
                    text.push_str(label);
                    tokens.push((
                        start..text.len(),
                        InlineToken::new(id.clone(), format!("@{label}")).with_label(label.clone()),
                    ));
                }
            }
        }
        let end = text.len();
        let mut content = InputContent::new(text.clone());
        for (range, token) in tokens {
            content = match content.with_token(range, token) {
                Ok(content) => content,
                // A mention that will not tokenize stays as its plain text.
                Err(_) => InputContent::new(text.clone()),
            };
        }
        self.attachments = prompt.attachments.clone();
        self.input.update(cx, |state, cx| {
            state.set_value(content, window, cx);
            state.set_selected_range(end..end, cx);
        });
        self.focus(window, cx);
        cx.notify();
    }

    fn selector_mut(&mut self, kind: SelectorKind) -> &mut Selector {
        match kind {
            SelectorKind::Mode => &mut self.modes,
        }
    }

    fn selector(&self, kind: SelectorKind) -> &Selector {
        match kind {
            SelectorKind::Mode => &self.modes,
        }
    }

    pub fn catalog(&self) -> &ModelCatalog {
        &self.catalog
    }

    /// Replaces the model list, as the agent reported it.
    pub fn set_models(&mut self, models: Vec<crate::event::ModelInfo>, cx: &mut Context<Self>) {
        self.catalog.set_models(models);
        self.reset_model_highlight(cx);
        cx.notify();
    }

    /// The model the agent says it uses.
    pub fn set_active_model(&mut self, id: &str, cx: &mut Context<Self>) {
        self.catalog.set_active(id);
        cx.notify();
    }

    pub fn set_favorite_models(&mut self, ids: Vec<String>, cx: &mut Context<Self>) {
        self.catalog.set_favorites(ids);
        cx.notify();
    }

    /// Opens the model picker with the search box focused.
    pub fn open_model_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.popover = None;
        self.open_menu = Some(OpenMenu::Models);
        self.model_filter.update(cx, |state, cx| {
            state.set_value("", window, cx);
            state.focus(window, cx);
        });
        self.reset_model_highlight(cx);
        cx.notify();
    }

    /// Closes whichever menu is open and hands focus back to the prompt.
    pub(super) fn close_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.open_menu.take() == Some(OpenMenu::Models) {
            self.focus(window, cx);
        }
        cx.notify();
    }

    fn refresh_popover(&mut self, cx: &mut Context<Self>) {
        let state = self.input.read(cx);
        let text = state.value();
        let tokens = state
            .tokens()
            .iter()
            .map(|span| span.range())
            .collect::<Vec<_>>();
        let trigger = detect_trigger(&text, state.cursor(), &tokens);
        let Some(trigger) = trigger else {
            self.dismissed = None;
            self.popover = None;
            cx.notify();
            return;
        };
        if self.dismissed == Some(trigger.range.start) {
            return;
        }
        self.dismissed = None;
        let mut candidates = self.source.candidates(trigger.kind, &trigger.query);
        candidates.truncate(MAX_CANDIDATES);
        let selected = self
            .popover
            .as_ref()
            .filter(|popover| popover.trigger.range.start == trigger.range.start)
            .map_or(0, |popover| popover.selected)
            .min(candidates.len().saturating_sub(1));
        self.popover = (!candidates.is_empty()).then_some(Popover {
            trigger,
            candidates,
            selected,
        });
        cx.notify();
    }

    fn move_selection(&mut self, delta: isize, cx: &mut Context<Self>) -> bool {
        let Some(popover) = self.popover.as_mut() else {
            return false;
        };
        let count = popover.candidates.len() as isize;
        popover.selected = (popover.selected as isize + delta).rem_euclid(count) as usize;
        cx.notify();
        true
    }

    fn accept(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(popover) = self.popover.take() else {
            return;
        };
        let Some(candidate) = popover.candidates.into_iter().nth(ix) else {
            return;
        };
        let range = popover.trigger.range;
        self.input
            .update(cx, |state, cx| match popover.trigger.kind {
                TriggerKind::Mention => {
                    let token = InlineToken::new(candidate.id, format!("@{}", candidate.label))
                        .with_label(candidate.label);
                    if state
                        .replace_range_with_token(range, token, window, cx)
                        .is_ok()
                    {
                        state.insert(" ", window, cx);
                    }
                }
                TriggerKind::Command => {
                    state.set_selected_range(range, cx);
                    state.replace(format!("/{} ", candidate.label), window, cx);
                }
            });
        self.focus(window, cx);
        cx.notify();
    }

    fn draft(&self, cx: &gpui::App) -> Prompt {
        let content = self.input.read(cx).content();
        let tokens = content
            .tokens()
            .iter()
            .map(|span| TokenRange {
                range: span.range(),
                id: span.token().id().to_string(),
                label: span.token().label().to_string(),
            })
            .collect::<Vec<_>>();
        Prompt {
            segments: prompt_segments(content.text(), &tokens),
            attachments: self.attachments.clone(),
        }
    }

    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.popover.is_some() {
            return;
        }
        let prompt = self.draft(cx);
        if prompt.is_empty() {
            return;
        }
        self.attachments.clear();
        self.recalled = None;
        if self.history.last() != Some(&prompt) {
            self.history.push(prompt.clone());
            if self.history.len() > MAX_HISTORY {
                self.history.remove(0);
            }
        }
        self.input.update(cx, |state, cx| state.clean(window, cx));
        cx.emit(ComposerEvent::Submit(prompt));
        cx.notify();
    }

    /// Walks the sent prompts while the box is empty or still shows the one
    /// recalled last; anything typed in between stops the walk.
    fn recall(&mut self, older: bool, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let draft = self.draft(cx);
        let walking = match self.recalled {
            Some(ix) => self.history.get(ix) == Some(&draft),
            None => draft.is_empty() && older,
        };
        if !walking || self.history.is_empty() {
            return false;
        }
        let next = match (self.recalled, older) {
            (None, _) => Some(self.history.len() - 1),
            (Some(ix), true) => Some(ix.saturating_sub(1)),
            (Some(ix), false) => (ix + 1 < self.history.len()).then_some(ix + 1),
        };
        self.recalled = next;
        let prompt = next
            .and_then(|ix| self.history.get(ix).cloned())
            .unwrap_or_default();
        self.set_draft(&prompt, window, cx);
        true
    }

    fn add_paths(&mut self, paths: &[std::path::PathBuf], cx: &mut Context<Self>) {
        for path in paths {
            let path = path.to_string_lossy().into_owned();
            let attached = self.attachments.iter().any(|attachment| {
                matches!(&attachment.kind, AttachmentKind::File { path: existing } if *existing == path)
            });
            if attached {
                continue;
            }
            let name = Path::new(&path)
                .file_name()
                .map_or_else(|| path.clone(), |name| name.to_string_lossy().into_owned());
            self.attachments.push(Attachment {
                name,
                kind: AttachmentKind::File { path },
            });
        }
        cx.notify();
    }

    /// Takes a pasted image as an attachment; text pastes go to the box.
    fn paste_image(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(item) = cx.read_from_clipboard() else {
            return false;
        };
        let images = item
            .entries()
            .iter()
            .filter_map(|entry| match entry {
                ClipboardEntry::Image(image) => Some(image.clone()),
                _ => None,
            })
            .collect::<Vec<_>>();
        if images.is_empty() {
            return false;
        }
        for image in images {
            let extension = format!("{:?}", image.format()).to_ascii_lowercase();
            self.attachments.push(Attachment {
                name: format!("Pasted image {}.{extension}", self.attachments.len() + 1),
                kind: AttachmentKind::Image {
                    mime: format!("image/{extension}"),
                    data: image.bytes().to_vec(),
                },
            });
        }
        cx.notify();
        true
    }

    fn on_attach(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("Attach".into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = paths.await {
                this.update(cx, |this, cx| this.add_paths(&paths, cx)).ok();
            }
        })
        .detach();
    }

    fn on_send_or_stop(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.status != ThreadStatus::Idle && self.draft(cx).is_empty() {
            cx.emit(ComposerEvent::Stop);
        } else {
            self.submit(window, cx);
        }
    }

    fn on_mention_button(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.input.update(cx, |state, cx| {
            let before = &state.value()[..state.cursor()];
            let needs_space = before
                .chars()
                .next_back()
                .is_some_and(|c| !c.is_whitespace());
            state.insert(if needs_space { " @" } else { "@" }, window, cx);
        });
        self.focus(window, cx);
    }

    fn icon_action(
        &self,
        id: impl Into<SharedString>,
        content: impl IntoElement,
        on_click: impl Fn(&ClickEvent, &mut Window, &mut gpui::App) + 'static,
    ) -> impl IntoElement {
        let style = yttt_icon_button_style(YtttIconButtonKind::Toolbar, self.theme, self.ui_style);
        div()
            .id(id.into())
            .flex()
            .flex_shrink_0()
            .items_center()
            .justify_center()
            .size(style.size)
            .rounded(style.radius)
            .bg(style.background)
            .text_color(style.text)
            .cursor_pointer()
            .hover(move |this| this.bg(style.hover_background).text_color(style.hover_text))
            .active(move |this| {
                this.bg(style.active_background)
                    .text_color(style.active_text)
            })
            .on_click(on_click)
            .child(content)
    }
}

impl Render for Composer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.theme;
        let spacing = self.ui_style.spacing;
        let focused = self.input.focus_handle(cx).is_focused(window);
        let busy = self.status != ThreadStatus::Idle;
        let empty = self.draft(cx).is_empty();
        let (slots, slot_context) = (self.slots.clone(), self.slot_context.clone());
        let footer_start = slots.render(Slot::ComposerFooterStart, &slot_context, window, cx);
        let footer_end = slots.render(Slot::ComposerFooterEnd, &slot_context, window, cx);

        v_flex()
            .relative()
            .w_full()
            .key_context("AgentComposer")
            .capture_action(cx.listener(|this, _: &MoveUp, window, cx| {
                if this.open_menu == Some(OpenMenu::Models) {
                    this.move_model_highlight(-1, cx);
                    cx.stop_propagation();
                } else if this.move_selection(-1, cx) || this.recall(true, window, cx) {
                    cx.stop_propagation();
                }
            }))
            .capture_action(cx.listener(|this, _: &MoveDown, window, cx| {
                if this.open_menu == Some(OpenMenu::Models) {
                    this.move_model_highlight(1, cx);
                    cx.stop_propagation();
                } else if this.move_selection(1, cx) || this.recall(false, window, cx) {
                    cx.stop_propagation();
                }
            }))
            // Plain Enter never reaches the textarea: left unhandled it would
            // fall through as a typed newline. Shift-Enter still inserts one.
            .capture_action(cx.listener(|this, action: &Enter, window, cx| {
                if action.shift {
                    return;
                }
                if this.open_menu == Some(OpenMenu::Models) {
                    this.pick_highlighted_model(window, cx);
                    cx.stop_propagation();
                    return;
                }
                match this.popover.as_ref().map(|popover| popover.selected) {
                    Some(selected) => this.accept(selected, window, cx),
                    None => this.submit(window, cx),
                }
                cx.stop_propagation();
            }))
            .capture_action(cx.listener(|this, _: &IndentInline, window, cx| {
                if let Some(selected) = this.popover.as_ref().map(|popover| popover.selected) {
                    this.accept(selected, window, cx);
                    cx.stop_propagation();
                }
            }))
            .capture_action(cx.listener(|this, _: &Escape, window, cx| {
                if let Some(popover) = this.popover.take() {
                    this.dismissed = Some(popover.trigger.range.start);
                } else if this.open_menu.is_some() {
                    this.close_menu(window, cx);
                } else {
                    if this.status == ThreadStatus::Idle {
                        return;
                    }
                    cx.emit(ComposerEvent::Stop);
                }
                cx.notify();
                cx.stop_propagation();
            }))
            .capture_action(cx.listener(|this, _: &Paste, _, cx| {
                if this.paste_image(cx) {
                    cx.stop_propagation();
                }
            }))
            .on_drop(cx.listener(|this, paths: &ExternalPaths, window, cx| {
                this.add_paths(paths.paths(), cx);
                this.focus(window, cx);
            }))
            .when_some(self.popover.as_ref(), |this, popover| {
                this.child(self.render_popover(popover, cx))
            })
            .when_some(self.open_menu, |this, menu| match menu {
                OpenMenu::Selector(kind) => {
                    this.child(self.render_menu(kind, cx).into_any_element())
                }
                OpenMenu::Models => this.child(self.render_model_picker(cx).into_any_element()),
            })
            .child(
                v_flex()
                    .w_full()
                    .rounded(self.ui_style.radius.card)
                    .border_1()
                    .border_color(if focused {
                        theme.border_focused
                    } else {
                        theme.border
                    })
                    .bg(theme.editor_background)
                    .drag_over::<ExternalPaths>(move |this, _, _, _| {
                        this.border_color(theme.accent).bg(theme.accent.alpha(0.06))
                    })
                    .when(!self.attachments.is_empty(), |this| {
                        this.child(self.render_attachments(cx))
                    })
                    .child(
                        div().px(spacing.md).pt(spacing.sm).child(
                            Textarea::new(&self.input)
                                .appearance(false)
                                .text_size(self.ui_style.text.small),
                        ),
                    )
                    .child(
                        h_flex()
                            .gap(spacing.xs)
                            .px(spacing.sm)
                            .pb(spacing.sm)
                            .child(self.icon_action(
                                "composer-mention",
                                div().text_size(self.ui_style.text.small).child("@"),
                                cx.listener(Self::on_mention_button),
                            ))
                            .child(self.icon_action(
                                "composer-attach",
                                Icon::new(IconName::Plus).size(px(14.)),
                                cx.listener(Self::on_attach),
                            ))
                            .children(self.render_selector_button(SelectorKind::Mode, cx))
                            .children(self.render_model_button(cx))
                            .children(footer_start)
                            .child(div().flex_1())
                            .children(footer_end)
                            .children(self.render_usage())
                            .child(self.render_send_button(busy, empty, cx)),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use gpui::{TestAppContext, VisualTestContext};

    use super::*;

    struct Files;

    impl CandidateSource for Files {
        fn candidates(&self, kind: TriggerKind, query: &str) -> Vec<Candidate> {
            let names: &[&str] = match kind {
                TriggerKind::Mention => &["main.rs", "model.rs"],
                TriggerKind::Command => &["compact", "clear"],
            };
            trigger::rank(query, names, |name| name)
                .into_iter()
                .map(|name| Candidate {
                    id: format!("file:///repo/{name}"),
                    label: (*name).to_owned(),
                    detail: None,
                })
                .collect()
        }
    }

    fn composer(cx: &mut TestAppContext) -> (Entity<Composer>, &mut VisualTestContext) {
        cx.update(yttt_ui::init);
        let (composer, cx) = cx.add_window_view(|window, cx| {
            Composer::new(
                Rc::new(Files),
                WorkbenchTheme::zed_defaults(true),
                UiStyle::default(),
                window,
                cx,
            )
        });
        composer.update_in(cx, |composer, window, cx| composer.focus(window, cx));
        (composer, cx)
    }

    fn submitted(
        composer: &Entity<Composer>,
        cx: &mut VisualTestContext,
    ) -> Rc<RefCell<Vec<Prompt>>> {
        let sent = Rc::new(RefCell::new(Vec::new()));
        let sink = sent.clone();
        cx.update(|_, cx| {
            cx.subscribe(composer, move |_, event, _| {
                if let ComposerEvent::Submit(prompt) = event {
                    sink.borrow_mut().push(prompt.clone());
                }
            })
            .detach();
        });
        sent
    }

    fn value(composer: &Entity<Composer>, cx: &mut VisualTestContext) -> String {
        composer.read_with(cx, |composer, cx| {
            composer.input.read(cx).value().to_string()
        })
    }

    #[gpui::test]
    fn accepting_a_mention_inserts_a_token_and_sends_it(cx: &mut TestAppContext) {
        let (composer, cx) = composer(cx);
        let sent = submitted(&composer, cx);
        cx.simulate_input("fix @mo");
        composer.read_with(cx, |composer, _| {
            let popover = composer.popover.as_ref().expect("popover opens");
            assert_eq!(popover.candidates[0].label, "model.rs");
        });
        cx.simulate_keystrokes("enter");
        composer.read_with(cx, |composer, cx| {
            assert!(composer.popover.is_none());
            let state = composer.input.read(cx);
            assert_eq!(state.value().as_ref(), "fix @model.rs ");
            assert_eq!(state.tokens().len(), 1);
        });
        cx.simulate_input("now");
        cx.simulate_keystrokes("enter");
        assert_eq!(
            sent.borrow()[0].segments,
            [
                PromptSegment::Text("fix ".to_owned()),
                PromptSegment::Mention {
                    id: "file:///repo/model.rs".to_owned(),
                    label: "model.rs".to_owned(),
                },
                PromptSegment::Text(" now".to_owned()),
            ]
        );
        assert_eq!(value(&composer, cx), "");
    }

    #[gpui::test]
    fn arrows_move_the_selection_and_escape_dismisses(cx: &mut TestAppContext) {
        let (composer, cx) = composer(cx);
        cx.simulate_input("/c");
        cx.simulate_keystrokes("down");
        composer.read_with(cx, |composer, _| {
            assert_eq!(composer.popover.as_ref().map(|p| p.selected), Some(1));
        });
        cx.simulate_keystrokes("escape");
        composer.read_with(cx, |composer, _| assert!(composer.popover.is_none()));
        cx.simulate_input("o");
        composer.read_with(cx, |composer, _| assert!(composer.popover.is_none()));
    }

    #[gpui::test]
    fn accepting_a_command_replaces_the_query(cx: &mut TestAppContext) {
        let (composer, cx) = composer(cx);
        cx.simulate_input("/com");
        cx.simulate_keystrokes("tab");
        assert_eq!(value(&composer, cx), "/compact ");
    }

    #[gpui::test]
    fn a_busy_thread_still_takes_prompts_to_queue(cx: &mut TestAppContext) {
        let (composer, cx) = composer(cx);
        let sent = submitted(&composer, cx);
        composer.update(cx, |composer, cx| {
            composer.set_status(ThreadStatus::Generating, cx)
        });
        cx.simulate_input("hello");
        cx.simulate_keystrokes("enter");
        assert_eq!(sent.borrow().len(), 1);
        assert_eq!(value(&composer, cx), "");
    }

    #[gpui::test]
    fn escape_stops_a_busy_thread(cx: &mut TestAppContext) {
        let (composer, cx) = composer(cx);
        let stopped = Rc::new(RefCell::new(0));
        let sink = stopped.clone();
        cx.update(|_, cx| {
            cx.subscribe(&composer, move |_, event, _| {
                if let ComposerEvent::Stop = event {
                    *sink.borrow_mut() += 1;
                }
            })
            .detach();
        });
        cx.simulate_keystrokes("escape");
        assert_eq!(*stopped.borrow(), 0);
        composer.update(cx, |composer, cx| {
            composer.set_status(ThreadStatus::Generating, cx)
        });
        cx.simulate_keystrokes("escape");
        assert_eq!(*stopped.borrow(), 1);
    }

    #[gpui::test]
    fn up_recalls_sent_prompts_with_their_mentions(cx: &mut TestAppContext) {
        let (composer, cx) = composer(cx);
        cx.simulate_input("first");
        cx.simulate_keystrokes("enter");
        cx.simulate_input("see @ma");
        cx.simulate_keystrokes("enter enter");
        cx.simulate_keystrokes("up");
        assert_eq!(value(&composer, cx), "see @main.rs");
        composer.read_with(cx, |composer, cx| {
            assert_eq!(composer.input.read(cx).tokens().len(), 1)
        });
        cx.simulate_keystrokes("up");
        assert_eq!(value(&composer, cx), "first");
        cx.simulate_keystrokes("down");
        assert_eq!(value(&composer, cx), "see @main.rs");
        cx.simulate_keystrokes("down");
        assert_eq!(value(&composer, cx), "");
    }

    #[gpui::test]
    fn dropped_files_attach_once_and_send_with_the_prompt(cx: &mut TestAppContext) {
        let (composer, cx) = composer(cx);
        let sent = submitted(&composer, cx);
        composer.update(cx, |composer, cx| {
            let paths = [std::path::PathBuf::from("/repo/shot.png")];
            composer.add_paths(&paths, cx);
            composer.add_paths(&paths, cx);
        });
        cx.simulate_keystrokes("enter");
        let sent = sent.borrow();
        assert_eq!(sent[0].attachments.len(), 1);
        assert_eq!(sent[0].attachments[0].name, "shot.png");
        assert!(sent[0].attachments[0].kind.is_image());
        composer.read_with(cx, |composer, _| assert!(composer.attachments.is_empty()));
    }

    fn with_models(composer: &Entity<Composer>, cx: &mut VisualTestContext) {
        composer.update(cx, |composer, cx| {
            composer.set_models(crate::demo::demo_models(false), cx)
        });
    }

    fn model_events(
        composer: &Entity<Composer>,
        cx: &mut VisualTestContext,
    ) -> Rc<RefCell<Vec<String>>> {
        let picked = Rc::new(RefCell::new(Vec::new()));
        let sink = picked.clone();
        cx.update(|_, cx| {
            cx.subscribe(composer, move |_, event, _| {
                if let ComposerEvent::ModelSelected { id } = event {
                    sink.borrow_mut().push(id.clone());
                }
            })
            .detach();
        });
        picked
    }

    #[gpui::test]
    fn the_model_picker_filters_and_picks_with_the_keyboard(cx: &mut TestAppContext) {
        let (composer, cx) = composer(cx);
        with_models(&composer, cx);
        let picked = model_events(&composer, cx);
        composer.update_in(cx, |composer, window, cx| {
            composer.open_model_picker(window, cx)
        });
        cx.simulate_input("cloud");
        cx.simulate_keystrokes("down enter");
        assert_eq!(picked.borrow().as_slice(), ["fast"]);
        composer.read_with(cx, |composer, _| {
            assert_eq!(
                composer.catalog.active().map(|m| m.id.as_str()),
                Some("fast")
            );
            assert!(composer.open_menu.is_none());
        });
        // Back in the prompt box: typing goes to the draft again.
        cx.simulate_input("hi");
        assert_eq!(value(&composer, cx), "hi");
    }

    #[gpui::test]
    fn the_highlight_skips_models_that_cannot_run(cx: &mut TestAppContext) {
        let (composer, cx) = composer(cx);
        with_models(&composer, cx);
        let picked = model_events(&composer, cx);
        composer.update_in(cx, |composer, window, cx| {
            composer.open_model_picker(window, cx)
        });
        // Large, Fast, Reasoner are usable; Partner Pro and Local 8B are not.
        cx.simulate_keystrokes("up enter");
        assert_eq!(picked.borrow().as_slice(), ["reasoner"]);
    }

    #[gpui::test]
    fn escape_closes_the_model_picker_without_picking(cx: &mut TestAppContext) {
        let (composer, cx) = composer(cx);
        with_models(&composer, cx);
        let picked = model_events(&composer, cx);
        composer.update_in(cx, |composer, window, cx| {
            composer.open_model_picker(window, cx)
        });
        cx.simulate_keystrokes("escape");
        assert!(picked.borrow().is_empty());
        composer.read_with(cx, |composer, _| assert!(composer.open_menu.is_none()));
    }
}
