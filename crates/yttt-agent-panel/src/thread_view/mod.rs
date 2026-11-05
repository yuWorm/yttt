//! The scrolling thread. Each entry keeps its rendered state (markdown text,
//! diffs, collapse overrides) across events, so a streamed delta only appends
//! to one text view instead of rebuilding the thread.
mod message;
mod tool_call;
mod tool_content;

pub(crate) use message::{prompt_text, render_attachment_preview};

use std::{
    collections::{HashMap, HashSet},
    hash::{DefaultHasher, Hash as _, Hasher as _},
    ops::Range,
    sync::Arc,
    time::{Duration, Instant},
};

use gpui::{
    App, Context, Entity, EventEmitter, FollowMode, HighlightStyle, IntoElement, ListAlignment,
    ListState, ParentElement as _, Render, Styled as _, Window, div, list, prelude::*, px,
};
use gpui_component::{
    ActiveTheme as _, Rope, diff::DiffState, highlighter::SyntaxHighlighter, text::TextViewState,
};
use yttt_ui::{style::UiStyle, theme::WorkbenchTheme};

use crate::{
    diff::{line_stats, unified_patch},
    event::{
        ImageSource, MessageId, NoticeId, PermissionOptionId, ThreadEvent, ToolCallId, ToolContent,
    },
    model::{AssistantChunk, EntryChange, Thread, ThreadEntry, ThreadStatus},
    slots::{CustomRender, MessageAction},
    text::language_for_path,
};

/// How far past the viewport the list lays out entries ahead of scrolling.
const OVERDRAW: f32 = 1024.;

pub enum ThreadViewEvent {
    PermissionChosen {
        tool_call_id: ToolCallId,
        option_id: PermissionOptionId,
    },
    /// The user clicked a file the agent touched; `line` is 1-based.
    OpenPath { path: String, line: Option<u32> },
    /// The user wants to rewrite a prompt they sent.
    EditMessage { id: MessageId },
    /// The user asked to run the last turn again.
    Retry,
    /// The user picked one of the empty thread's suggestions.
    Suggestion(String),
    NoticeAction {
        notice_id: NoticeId,
        action_id: String,
    },
    ToolAction {
        tool_call_id: ToolCallId,
        action_id: String,
    },
    /// A host message action; `message_id` is the prompt it sits on or the
    /// prompt that started the turn, and `text` the prompt's or the turn's
    /// replies.
    MessageAction {
        action_id: String,
        message_id: Option<MessageId>,
        text: String,
    },
}

pub struct ThreadView {
    thread: Thread,
    list: ListState,
    views: Vec<EntryView>,
    /// Overrides each card's own choice to open, until the user toggles it.
    cards_open: Option<bool>,
    /// Prompts the empty thread offers.
    suggestions: Vec<String>,
    custom_renderers: HashMap<String, CustomRender>,
    message_actions: Vec<MessageAction>,
    /// When the running turn's prompt arrived.
    turn_started: Option<Instant>,
    theme: WorkbenchTheme,
    ui_style: UiStyle,
}

/// Rendered state for one entry, index-aligned with [`Thread::entries`].
#[derive(Default)]
struct EntryView {
    chunks: Vec<ChunkView>,
    /// Index-aligned with the tool call's content.
    contents: Vec<ContentCache>,
    /// Content the user asked to see in full, by index.
    expanded: HashSet<usize>,
    /// Set once the user toggles a tool card; automatic opening stops then.
    open: Option<bool>,
    /// When the panel first saw the call unfinished, and how long it took;
    /// for a turn's end, how long the turn took.
    started: Option<Instant>,
    elapsed: Option<Duration>,
}

/// Content that is costly to rebuild each frame.
#[derive(Default)]
enum ContentCache {
    #[default]
    None,
    Diff(DiffView),
    Image {
        key: u64,
        image: Arc<gpui::Image>,
    },
    /// Syntax highlights for a file's text.
    Code {
        key: u64,
        styles: Vec<(Range<usize>, HighlightStyle)>,
    },
}

struct ChunkView {
    text: Entity<TextViewState>,
    len: usize,
    open: Option<bool>,
    started: Instant,
    /// How long a thought streamed, once it stopped.
    elapsed: Option<Duration>,
}

struct DiffView {
    state: Entity<DiffState>,
    patch: String,
    rows: usize,
    added: usize,
    removed: usize,
}

impl EventEmitter<ThreadViewEvent> for ThreadView {}

impl ThreadView {
    pub fn new(theme: WorkbenchTheme, ui_style: UiStyle, cx: &mut Context<Self>) -> Self {
        let list = ListState::new(0, ListAlignment::Top, px(OVERDRAW));
        list.set_follow_mode(FollowMode::Tail);
        // Scrolling shows and hides the jump-to-latest button.
        let this = cx.weak_entity();
        list.set_scroll_handler(move |_, _, cx| {
            this.update(cx, |_, cx| cx.notify()).ok();
        });
        Self {
            thread: Thread::default(),
            list,
            views: Vec::new(),
            cards_open: None,
            suggestions: Vec::new(),
            custom_renderers: HashMap::new(),
            message_actions: Vec::new(),
            turn_started: None,
            theme,
            ui_style,
        }
    }

    pub fn status(&self) -> ThreadStatus {
        self.thread.status()
    }

    pub fn entries(&self) -> &[ThreadEntry] {
        self.thread.entries()
    }

    pub fn is_empty(&self) -> bool {
        self.thread.entries().is_empty()
    }

    pub fn thread(&self) -> &Thread {
        &self.thread
    }

    /// The user's title; `None` goes back to the automatic one.
    pub fn set_title(&mut self, title: Option<String>, cx: &mut Context<Self>) {
        self.thread.set_title(title);
        cx.notify();
    }

    /// Marks the open edits to `paths` as handled.
    pub fn resolve_edits(&mut self, paths: &[String], cx: &mut Context<Self>) {
        self.thread.resolve_edits(paths);
        cx.notify();
    }

    /// Draws custom tool content of `kind` with `render` from now on.
    pub fn set_custom_renderer(
        &mut self,
        kind: impl Into<String>,
        render: CustomRender,
        cx: &mut Context<Self>,
    ) {
        self.custom_renderers.insert(kind.into(), render);
        self.list.remeasure();
        cx.notify();
    }

    pub fn set_message_actions(&mut self, actions: Vec<MessageAction>, cx: &mut Context<Self>) {
        self.message_actions = actions;
        self.list.remeasure();
        cx.notify();
    }

    pub fn set_suggestions(&mut self, suggestions: Vec<String>, cx: &mut Context<Self>) {
        self.suggestions = suggestions;
        cx.notify();
    }

    pub fn apply(&mut self, event: ThreadEvent, cx: &mut Context<Self>) {
        // Whatever was last stops streaming once something follows it or the
        // turn ends, so it re-measures along with the entries the event hit.
        let previous_last = self.thread.entries().len().checked_sub(1);
        let EntryChange {
            removed,
            updated,
            inserted,
        } = self.thread.apply(event);
        if let Some(range) = removed {
            self.views.drain(range.clone());
            self.list.splice(range, 0);
        }
        for ix in updated {
            self.sync(ix, cx);
            self.list.remeasure_items(ix..ix + 1);
        }
        if let Some(ix) = inserted {
            self.views.insert(ix, EntryView::default());
            self.sync(ix, cx);
            self.list.splice(ix..ix, 1);
        }
        self.finish_thoughts();
        if let Some(ix) = previous_last.filter(|ix| *ix < self.list.item_count()) {
            self.list.remeasure_items(ix..ix + 1);
        }
        cx.notify();
    }

    /// Opens or folds every tool card; `None` lets each card decide.
    pub fn set_cards_open(&mut self, open: Option<bool>, cx: &mut Context<Self>) {
        self.cards_open = open;
        self.list.remeasure();
        cx.notify();
    }

    pub fn scroll_to_top(&mut self, cx: &mut Context<Self>) {
        self.list.scroll_to(gpui::ListOffset::default());
        cx.notify();
    }

    /// Scrolls down by `distance`; false once the end is already showing.
    pub fn scroll_down(&mut self, distance: gpui::Pixels, cx: &mut Context<Self>) -> bool {
        if self.list.is_scrolled_to_end().unwrap_or(true) {
            return false;
        }
        self.list.scroll_by(distance);
        cx.notify();
        true
    }

    pub fn clear(&mut self, cx: &mut Context<Self>) {
        self.thread.clear();
        self.views.clear();
        self.turn_started = None;
        self.list.reset(0);
        cx.notify();
    }

    /// Stamps how long each thought took once something follows it.
    fn finish_thoughts(&mut self) {
        let len = self.thread.entries().len();
        for ix in len.saturating_sub(2)..len {
            let ThreadEntry::Assistant { chunks, .. } = &self.thread.entries()[ix] else {
                continue;
            };
            let streaming_chunk = self.is_streaming(ix).then(|| chunks.len() - 1);
            for (chunk_ix, chunk_view) in self.views[ix].chunks.iter_mut().enumerate() {
                if chunk_view.elapsed.is_none()
                    && Some(chunk_ix) != streaming_chunk
                    && matches!(chunks.get(chunk_ix), Some(AssistantChunk::Thought(_)))
                {
                    chunk_view.elapsed = Some(chunk_view.started.elapsed());
                }
            }
        }
    }

    fn sync(&mut self, ix: usize, cx: &mut Context<Self>) {
        let view = &mut self.views[ix];
        match &self.thread.entries()[ix] {
            ThreadEntry::User { .. } => self.turn_started = Some(Instant::now()),
            ThreadEntry::Notice(_) => {}
            ThreadEntry::TurnEnd { .. } => {
                view.elapsed = self.turn_started.take().map(|started| started.elapsed());
            }
            ThreadEntry::Assistant { chunks, .. } => {
                for (chunk_ix, chunk) in chunks.iter().enumerate() {
                    let text = match chunk {
                        AssistantChunk::Text(text) | AssistantChunk::Thought(text) => text,
                    };
                    match view.chunks.get_mut(chunk_ix) {
                        Some(chunk_view) if text.len() > chunk_view.len => {
                            let delta = text[chunk_view.len..].to_owned();
                            chunk_view.len = text.len();
                            chunk_view
                                .text
                                .update(cx, |state, cx| state.push_str(&delta, cx));
                        }
                        Some(_) => {}
                        None => view.chunks.push(ChunkView {
                            text: cx.new(|cx| TextViewState::markdown(text, cx)),
                            len: text.len(),
                            open: None,
                            started: Instant::now(),
                            elapsed: None,
                        }),
                    }
                }
            }
            ThreadEntry::ToolCall(call) => {
                if call.status.is_finished() {
                    if let (Some(started), None) = (view.started, view.elapsed) {
                        view.elapsed = Some(started.elapsed());
                    }
                } else if view.started.is_none() {
                    view.started = Some(Instant::now());
                }
                view.contents
                    .resize_with(call.content.len(), ContentCache::default);
                for (content, slot) in call.content.iter().zip(&mut view.contents) {
                    match content {
                        ToolContent::Diff {
                            path,
                            old_text,
                            new_text,
                        } => {
                            let patch = unified_patch(path, old_text.as_deref(), new_text);
                            if !matches!(slot, ContentCache::Diff(diff) if diff.patch == patch) {
                                *slot = ContentCache::Diff(DiffView::new(
                                    patch,
                                    old_text.as_deref(),
                                    new_text,
                                    cx,
                                ));
                            }
                        }
                        ToolContent::Image {
                            source: ImageSource::Bytes { mime, data },
                            ..
                        } => {
                            let key = content_key(data);
                            if !matches!(slot, ContentCache::Image { key: cached, .. } if *cached == key)
                            {
                                *slot = match image_format(mime) {
                                    Some(format) => ContentCache::Image {
                                        key,
                                        image: Arc::new(gpui::Image::from_bytes(
                                            format,
                                            data.clone(),
                                        )),
                                    },
                                    None => ContentCache::None,
                                };
                            }
                        }
                        ToolContent::File { path, text, .. } => {
                            let key = content_key(text.as_bytes());
                            if !matches!(slot, ContentCache::Code { key: cached, .. } if *cached == key)
                            {
                                *slot = ContentCache::Code {
                                    key,
                                    styles: highlight(path, text, cx),
                                };
                            }
                        }
                        _ => *slot = ContentCache::None,
                    }
                }
            }
        }
    }

    fn is_streaming(&self, ix: usize) -> bool {
        self.thread.status() != ThreadStatus::Idle && ix + 1 == self.thread.entries().len()
    }

    fn render_entry(
        &mut self,
        ix: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let body = match &self.thread.entries()[ix] {
            ThreadEntry::User {
                id,
                content,
                attachments,
            } => {
                let (id, content, attachments) = (id.clone(), content.clone(), attachments.clone());
                self.render_user(ix, &id, &content, &attachments, cx)
                    .into_any_element()
            }
            ThreadEntry::Notice(notice) => {
                let notice = notice.clone();
                self.render_notice(ix, &notice, cx)
            }
            ThreadEntry::TurnEnd { reason, tool_calls } => {
                let (reason, tool_calls) = (reason.clone(), *tool_calls);
                self.render_turn_end(ix, &reason, tool_calls, cx)
            }
            ThreadEntry::Assistant { chunks, .. } => {
                let chunks = chunks.clone();
                self.render_assistant(ix, &chunks, cx).into_any_element()
            }
            ThreadEntry::ToolCall(call) => {
                let call = call.clone();
                self.render_tool_call(ix, &call, window, cx)
                    .into_any_element()
            }
        };
        div()
            .px(self.ui_style.spacing.lg)
            .pt(self.ui_style.spacing.md)
            .when(ix + 1 == self.thread.entries().len(), |this| {
                this.pb(self.ui_style.spacing.lg)
            })
            .child(body)
            .into_any_element()
    }
}

impl DiffView {
    fn new(patch: String, old: Option<&str>, new: &str, cx: &mut Context<ThreadView>) -> Self {
        let files = gpui_component::diff::DiffFile::parse(&patch).unwrap_or_default();
        // The two `---`/`+++` header lines do not render as rows.
        let rows = patch.lines().count().saturating_sub(2);
        let (added, removed) = line_stats(old, new);
        Self {
            state: cx.new(|cx| DiffState::new(files, cx)),
            patch,
            rows,
            added,
            removed,
        }
    }
}

impl Render for ThreadView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.is_empty() {
            return self.render_empty(cx).into_any_element();
        }
        let behind =
            !self.list.is_following_tail() && self.list.is_scrolled_to_end() == Some(false);
        let theme = self.theme;
        div()
            .relative()
            .size_full()
            .child(
                list(
                    self.list.clone(),
                    cx.processor(|this, ix, window, cx| this.render_entry(ix, window, cx)),
                )
                .size_full(),
            )
            .when(behind, |this| {
                this.child(
                    div()
                        .absolute()
                        .bottom(px(12.))
                        .left_0()
                        .right_0()
                        .flex()
                        .justify_center()
                        .child(
                            div()
                                .id("jump-to-latest")
                                .flex()
                                .items_center()
                                .justify_center()
                                .size(px(30.))
                                .rounded(px(15.))
                                .border_1()
                                .border_color(theme.border)
                                .bg(theme.surface_elevated)
                                .shadow_md()
                                .cursor_pointer()
                                .hover(move |this| this.bg(theme.element_hover))
                                .child(
                                    yttt_ui::Icon::new(yttt_ui::IconName::ArrowDown)
                                        .size(px(14.))
                                        .text_color(theme.icon),
                                )
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.list.scroll_to_end();
                                    this.list.set_follow_mode(FollowMode::Tail);
                                    cx.notify();
                                })),
                        ),
                )
            })
            .into_any_element()
    }
}

pub(crate) fn toggle(open: &mut Option<bool>, shown: bool) {
    *open = Some(!shown);
}

/// Files longer than this stay plain rather than parse on the UI thread.
const MAX_HIGHLIGHTED_BYTES: usize = 64 * 1024;

fn highlight(path: &str, text: &str, cx: &App) -> Vec<(Range<usize>, HighlightStyle)> {
    let Some(language) = language_for_path(path).filter(|_| text.len() <= MAX_HIGHLIGHTED_BYTES)
    else {
        return Vec::new();
    };
    let mut highlighter = SyntaxHighlighter::new(language);
    highlighter.update(None, &Rope::from_str(text), None);
    highlighter.styles(&(0..text.len()), cx.theme().highlight_theme.as_ref())
}

fn content_key(bytes: &[u8]) -> u64 {
    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    hasher.finish()
}

fn image_format(mime: &str) -> Option<gpui::ImageFormat> {
    Some(match mime {
        "image/png" => gpui::ImageFormat::Png,
        "image/jpeg" | "image/jpg" => gpui::ImageFormat::Jpeg,
        "image/webp" => gpui::ImageFormat::Webp,
        "image/gif" => gpui::ImageFormat::Gif,
        "image/svg+xml" => gpui::ImageFormat::Svg,
        "image/bmp" => gpui::ImageFormat::Bmp,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use gpui::TestAppContext;

    use super::*;
    use crate::event::{MessageId, ToolCallFields, ToolCallStatus, ToolKind};

    fn view(cx: &mut TestAppContext) -> Entity<ThreadView> {
        cx.update(|cx| {
            let theme = WorkbenchTheme::zed_defaults(true);
            cx.new(|cx| ThreadView::new(theme, UiStyle::default(), cx))
        })
    }

    fn diff_state(view: &ThreadView) -> Entity<DiffState> {
        match &view.views[0].contents[0] {
            ContentCache::Diff(diff) => diff.state.clone(),
            _ => panic!("content 0 is not a diff"),
        }
    }

    fn text(delta: &str) -> ThreadEvent {
        ThreadEvent::AgentText {
            message_id: MessageId::from("m1"),
            delta: delta.to_owned(),
        }
    }

    #[gpui::test]
    fn streaming_appends_to_the_same_text_view(cx: &mut TestAppContext) {
        let view = view(cx);
        view.update(cx, |view, cx| view.apply(text("Hel"), cx));
        let first = view.read_with(cx, |view, _| view.views[0].chunks[0].text.clone());
        view.update(cx, |view, cx| view.apply(text("lo"), cx));
        view.read_with(cx, |view, _| {
            let chunk = &view.views[0].chunks[0];
            assert_eq!(chunk.text, first);
            assert_eq!(chunk.len, "Hello".len());
            assert_eq!(view.list.item_count(), 1);
        });
    }

    #[gpui::test]
    fn a_diff_view_is_kept_until_its_patch_changes(cx: &mut TestAppContext) {
        let view = view(cx);
        let diff = |new: &str| ToolContent::Diff {
            path: "a.rs".to_owned(),
            old_text: Some("a\n".to_owned()),
            new_text: new.to_owned(),
        };
        let update = |content: Vec<ToolContent>, status| ThreadEvent::ToolCallUpdate {
            id: "t1".into(),
            fields: ToolCallFields {
                status,
                content: Some(content),
                ..Default::default()
            },
        };
        view.update(cx, |view, cx| {
            view.apply(
                ThreadEvent::ToolCall {
                    id: "t1".into(),
                    kind: ToolKind::Edit,
                    title: "Edit a.rs".to_owned(),
                    status: ToolCallStatus::Running,
                    target: None,
                },
                cx,
            );
            view.apply(update(vec![diff("b\n")], None), cx);
        });
        let first = view.read_with(cx, |view, _| diff_state(view));
        view.update(cx, |view, cx| {
            view.apply(
                update(vec![diff("b\n")], Some(ToolCallStatus::Completed)),
                cx,
            )
        });
        let kept = view.read_with(cx, |view, _| diff_state(view));
        assert_eq!(kept, first);
        view.update(cx, |view, cx| {
            view.apply(update(vec![diff("c\n")], None), cx)
        });
        let replaced = view.read_with(cx, |view, _| diff_state(view));
        assert_ne!(replaced, first);
    }
}
