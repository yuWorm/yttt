//! The thread as the panel shows it, folded from [`ThreadEvent`]s. Pure data,
//! so every transition is testable without a window.
use std::collections::HashSet;

use crate::{
    diff::line_stats,
    event::{
        Action, Attachment, MessageId, NoticeId, NoticeKind, PermissionOption, PermissionOptionId,
        PermissionOptionKind, PromptSegment, StopReason, ThreadEvent, TodoItem, ToolCallId,
        ToolCallStatus, ToolContent, ToolKind, ToolTarget,
    },
};

#[derive(Clone, Debug, PartialEq)]
pub enum ThreadEntry {
    User {
        id: MessageId,
        content: Vec<PromptSegment>,
        attachments: Vec<Attachment>,
    },
    Assistant {
        id: MessageId,
        chunks: Vec<AssistantChunk>,
    },
    ToolCall(ToolCall),
    Notice(Notice),
    /// Closes a turn: why it stopped and how many tools it ran.
    TurnEnd {
        reason: StopReason,
        tool_calls: usize,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AssistantChunk {
    Text(String),
    Thought(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct ToolCall {
    pub id: ToolCallId,
    pub kind: ToolKind,
    pub title: String,
    pub status: ToolCallStatus,
    pub target: Option<ToolTarget>,
    pub content: Vec<ToolContent>,
    pub permission: Option<PermissionPrompt>,
    /// Added and removed lines for each diff in `content`, by index.
    pub diff_stats: Vec<(usize, usize, usize)>,
    pub actions: Vec<Action>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PermissionPrompt {
    pub options: Vec<PermissionOption>,
    pub chosen: Option<PermissionOptionId>,
}

impl PermissionPrompt {
    pub fn chosen_option(&self) -> Option<&PermissionOption> {
        let chosen = self.chosen.as_ref()?;
        self.options.iter().find(|option| &option.id == chosen)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notice {
    pub id: NoticeId,
    pub kind: NoticeKind,
    pub message: String,
    pub actions: Vec<Action>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ThreadStatus {
    #[default]
    Idle,
    Generating,
    AwaitingPermission,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Usage {
    pub used_tokens: u64,
    pub limit_tokens: u64,
}

/// A file the thread changed, its diffs' line counts summed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChangedFile {
    pub path: String,
    pub added: usize,
    pub removed: usize,
}

/// Which entries an event touched, so views redo only those. Indices are
/// after the change.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EntryChange {
    pub removed: Option<std::ops::Range<usize>>,
    pub updated: Vec<usize>,
    pub inserted: Option<usize>,
}

impl EntryChange {
    pub fn none() -> Self {
        Self::default()
    }

    pub fn inserted(ix: usize) -> Self {
        Self {
            inserted: Some(ix),
            ..Self::default()
        }
    }

    pub fn updated(ixs: Vec<usize>) -> Self {
        Self {
            updated: ixs,
            ..Self::default()
        }
    }

    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

#[derive(Clone, Debug, Default)]
pub struct Thread {
    entries: Vec<ThreadEntry>,
    status: ThreadStatus,
    usage: Option<Usage>,
    /// Edits the user kept or undid, by tool call and content index.
    resolved_edits: HashSet<(ToolCallId, usize)>,
    /// A title the user typed, and one the agent chose.
    user_title: Option<String>,
    agent_title: Option<String>,
}

impl Thread {
    pub fn entries(&self) -> &[ThreadEntry] {
        &self.entries
    }

    pub fn status(&self) -> ThreadStatus {
        self.status
    }

    pub fn usage(&self) -> Option<Usage> {
        self.usage
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// The title the user typed, else the agent's, else the first prompt's
    /// opening line.
    pub fn title(&self) -> Option<String> {
        self.user_title
            .clone()
            .or_else(|| self.agent_title.clone())
            .or_else(|| self.derived_title())
    }

    /// Sets the user's title; `None` or a blank one goes back to automatic.
    pub fn set_title(&mut self, title: Option<String>) {
        self.user_title = title
            .map(|title| title.trim().to_owned())
            .filter(|title| !title.is_empty());
    }

    pub fn has_user_title(&self) -> bool {
        self.user_title.is_some()
    }

    fn derived_title(&self) -> Option<String> {
        const MAX_CHARS: usize = 60;
        let content = self.entries.iter().find_map(|entry| match entry {
            ThreadEntry::User { content, .. } => Some(content),
            _ => None,
        })?;
        let text = content
            .iter()
            .map(|segment| match segment {
                PromptSegment::Text(text) => text.clone(),
                PromptSegment::Mention { label, .. } => format!("@{label}"),
            })
            .collect::<String>();
        let line = text.lines().find(|line| !line.trim().is_empty())?.trim();
        Some(match line.char_indices().nth(MAX_CHARS) {
            Some((end, _)) => format!("{}…", &line[..end]),
            None => line.to_owned(),
        })
    }

    /// The prompt that started the turn `ix` belongs to.
    pub fn turn_message(&self, ix: usize) -> Option<&MessageId> {
        self.entries
            .get(..=ix)?
            .iter()
            .rev()
            .find_map(|entry| match entry {
                ThreadEntry::User { id, .. } => Some(id),
                _ => None,
            })
    }

    /// The most recent task list any tool call reported.
    pub fn latest_plan(&self) -> Option<&[TodoItem]> {
        self.entries.iter().rev().find_map(|entry| match entry {
            ThreadEntry::ToolCall(call) => call.content.iter().find_map(|content| match content {
                ToolContent::Todo(items) => Some(items.as_slice()),
                _ => None,
            }),
            _ => None,
        })
    }

    /// Files changed by completed edits the user has neither kept nor
    /// undone, in the order they were first changed.
    pub fn changed_files(&self) -> Vec<ChangedFile> {
        let mut files: Vec<ChangedFile> = Vec::new();
        for (_, _, path, added, removed) in self.open_edits() {
            match files.iter_mut().find(|file| file.path == path) {
                Some(file) => {
                    file.added += added;
                    file.removed += removed;
                }
                None => files.push(ChangedFile {
                    path: path.to_owned(),
                    added,
                    removed,
                }),
            }
        }
        files
    }

    /// Takes the open edits to `paths` off the changed-files list, whether
    /// the user kept or undid them.
    pub fn resolve_edits(&mut self, paths: &[String]) {
        let resolved = self
            .open_edits()
            .filter(|(_, _, path, _, _)| paths.iter().any(|resolved| resolved == path))
            .map(|(call, content_ix, ..)| (call.clone(), content_ix))
            .collect::<Vec<_>>();
        self.resolved_edits.extend(resolved);
    }

    fn open_edits(&self) -> impl Iterator<Item = (&ToolCallId, usize, &str, usize, usize)> {
        self.entries
            .iter()
            .filter_map(|entry| match entry {
                ThreadEntry::ToolCall(call) if call.status == ToolCallStatus::Completed => {
                    Some(call)
                }
                _ => None,
            })
            .flat_map(|call| {
                call.diff_stats
                    .iter()
                    .filter_map(move |&(content_ix, added, removed)| {
                        match call.content.get(content_ix)? {
                            ToolContent::Diff { path, .. } => {
                                Some((&call.id, content_ix, path.as_str(), added, removed))
                            }
                            _ => None,
                        }
                    })
            })
            .filter(|(id, content_ix, ..)| {
                !self.resolved_edits.contains(&((*id).clone(), *content_ix))
            })
    }

    pub fn apply(&mut self, event: ThreadEvent) -> EntryChange {
        match event {
            ThreadEvent::UserMessage {
                id,
                content,
                attachments,
            } => {
                self.status = ThreadStatus::Generating;
                self.push(ThreadEntry::User {
                    id,
                    content,
                    attachments,
                })
            }
            ThreadEvent::AgentText { message_id, delta } => {
                self.append_chunk(message_id, AssistantChunk::Text(delta))
            }
            ThreadEvent::AgentThinking { message_id, delta } => {
                self.append_chunk(message_id, AssistantChunk::Thought(delta))
            }
            ThreadEvent::ToolCall {
                id,
                kind,
                title,
                status,
                target,
            } => self.push(ThreadEntry::ToolCall(ToolCall {
                id,
                kind,
                title,
                status,
                target,
                content: Vec::new(),
                permission: None,
                diff_stats: Vec::new(),
                actions: Vec::new(),
            })),
            ThreadEvent::ToolCallUpdate { id, fields } => self.update_tool_call(&id, |call| {
                if let Some(title) = fields.title {
                    call.title = title;
                }
                if let Some(status) = fields.status {
                    call.status = status;
                }
                if let Some(target) = fields.target {
                    call.target = Some(target);
                }
                if let Some(actions) = fields.actions {
                    call.actions = actions;
                }
                if let Some(content) = fields.content {
                    call.diff_stats = content
                        .iter()
                        .enumerate()
                        .filter_map(|(ix, content)| match content {
                            ToolContent::Diff {
                                old_text, new_text, ..
                            } => {
                                let (added, removed) = line_stats(old_text.as_deref(), new_text);
                                Some((ix, added, removed))
                            }
                            _ => None,
                        })
                        .collect();
                    call.content = content;
                }
            }),
            ThreadEvent::PermissionRequest {
                tool_call_id,
                options,
            } => {
                let change = self.update_tool_call(&tool_call_id, |call| {
                    call.status = ToolCallStatus::AwaitingPermission;
                    call.permission = Some(PermissionPrompt {
                        options,
                        chosen: None,
                    });
                });
                if !change.is_empty() {
                    self.status = ThreadStatus::AwaitingPermission;
                }
                change
            }
            ThreadEvent::PermissionResolved {
                tool_call_id,
                option_id,
            } => {
                let change = self.update_tool_call(&tool_call_id, |call| {
                    let Some(prompt) = call.permission.as_mut() else {
                        return;
                    };
                    prompt.chosen = Some(option_id);
                    call.status = match prompt.chosen_option().map(|option| option.kind) {
                        Some(PermissionOptionKind::Reject) => ToolCallStatus::Rejected,
                        _ => ToolCallStatus::Running,
                    };
                });
                if !change.is_empty() && self.status == ThreadStatus::AwaitingPermission {
                    self.status = ThreadStatus::Generating;
                }
                change
            }
            ThreadEvent::Stop { reason } => self.stop(reason),
            ThreadEvent::Notice {
                id,
                kind,
                message,
                actions,
            } => {
                let existing = self.entries.iter().position(
                    |entry| matches!(entry, ThreadEntry::Notice(notice) if notice.id == id),
                );
                let notice = Notice {
                    id,
                    kind,
                    message,
                    actions,
                };
                match existing {
                    Some(ix) => {
                        self.entries[ix] = ThreadEntry::Notice(notice);
                        EntryChange::updated(vec![ix])
                    }
                    None => self.push(ThreadEntry::Notice(notice)),
                }
            }
            ThreadEvent::ClearNotice { id } => {
                match self.entries.iter().position(
                    |entry| matches!(entry, ThreadEntry::Notice(notice) if notice.id == id),
                ) {
                    Some(ix) => {
                        self.entries.remove(ix);
                        EntryChange {
                            removed: Some(ix..ix + 1),
                            ..EntryChange::default()
                        }
                    }
                    None => EntryChange::none(),
                }
            }
            ThreadEvent::Truncate { from } => {
                let Some(start) = self
                    .entries
                    .iter()
                    .position(|entry| matches!(entry, ThreadEntry::User { id, .. } if *id == from))
                else {
                    return EntryChange::none();
                };
                let end = self.entries.len();
                self.entries.truncate(start);
                self.status = ThreadStatus::Idle;
                EntryChange {
                    removed: Some(start..end),
                    ..EntryChange::default()
                }
            }
            ThreadEvent::Usage {
                used_tokens,
                limit_tokens,
            } => {
                self.usage = Some(Usage {
                    used_tokens,
                    limit_tokens,
                });
                EntryChange::none()
            }
            ThreadEvent::Title { title } => {
                self.agent_title = title.filter(|title| !title.trim().is_empty());
                EntryChange::none()
            }
            // The model list belongs to the panel's picker, not the thread.
            ThreadEvent::ModelCatalog { .. } | ThreadEvent::ActiveModel { .. } => {
                EntryChange::none()
            }
        }
    }

    /// Ends the turn: a cancel cancels the unfinished cards, and a turn that
    /// was running gets its closing entry.
    fn stop(&mut self, reason: StopReason) -> EntryChange {
        let was_running = self.status != ThreadStatus::Idle;
        self.status = ThreadStatus::Idle;
        let mut change = EntryChange::none();
        if reason == StopReason::Cancelled {
            for (ix, entry) in self.entries.iter_mut().enumerate() {
                if let ThreadEntry::ToolCall(call) = entry
                    && !call.status.is_finished()
                {
                    call.status = ToolCallStatus::Canceled;
                    change.updated.push(ix);
                }
            }
        }
        if was_running {
            let tool_calls = self
                .entries
                .iter()
                .rev()
                .take_while(|entry| !matches!(entry, ThreadEntry::User { .. }))
                .filter(|entry| matches!(entry, ThreadEntry::ToolCall(_)))
                .count();
            change.inserted = self
                .push(ThreadEntry::TurnEnd { reason, tool_calls })
                .inserted;
        }
        change
    }

    fn push(&mut self, entry: ThreadEntry) -> EntryChange {
        self.entries.push(entry);
        EntryChange::inserted(self.entries.len() - 1)
    }

    /// Streams into the last chunk of the same kind when the message is
    /// still the newest entry; anything else in between starts a new entry.
    fn append_chunk(&mut self, message_id: MessageId, chunk: AssistantChunk) -> EntryChange {
        let last = self.entries.len().wrapping_sub(1);
        if let Some(ThreadEntry::Assistant { id, chunks }) = self.entries.last_mut()
            && *id == message_id
        {
            match (chunks.last_mut(), chunk) {
                (Some(AssistantChunk::Text(text)), AssistantChunk::Text(delta))
                | (Some(AssistantChunk::Thought(text)), AssistantChunk::Thought(delta)) => {
                    text.push_str(&delta)
                }
                (_, chunk) => chunks.push(chunk),
            }
            return EntryChange::updated(vec![last]);
        }
        self.push(ThreadEntry::Assistant {
            id: message_id,
            chunks: vec![chunk],
        })
    }

    fn update_tool_call(&mut self, id: &ToolCallId, f: impl FnOnce(&mut ToolCall)) -> EntryChange {
        let Some((ix, call)) = self
            .entries
            .iter_mut()
            .enumerate()
            .rev()
            .find_map(|(ix, entry)| match entry {
                ThreadEntry::ToolCall(call) if &call.id == id => Some((ix, call)),
                _ => None,
            })
        else {
            return EntryChange::none();
        };
        f(call);
        EntryChange::updated(vec![ix])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::ToolCallFields;

    fn text(message: &str, delta: &str) -> ThreadEvent {
        ThreadEvent::AgentText {
            message_id: message.into(),
            delta: delta.to_owned(),
        }
    }

    fn thinking(message: &str, delta: &str) -> ThreadEvent {
        ThreadEvent::AgentThinking {
            message_id: message.into(),
            delta: delta.to_owned(),
        }
    }

    fn tool_call(id: &str) -> ThreadEvent {
        ThreadEvent::ToolCall {
            id: id.into(),
            kind: ToolKind::Edit,
            title: "Edit main.rs".to_owned(),
            status: ToolCallStatus::Running,
            target: None,
        }
    }

    fn options() -> Vec<PermissionOption> {
        vec![
            PermissionOption {
                id: "allow".into(),
                label: "Allow".to_owned(),
                kind: PermissionOptionKind::AllowOnce,
            },
            PermissionOption {
                id: "reject".into(),
                label: "Reject".to_owned(),
                kind: PermissionOptionKind::Reject,
            },
        ]
    }

    fn call(thread: &Thread, ix: usize) -> &ToolCall {
        match &thread.entries()[ix] {
            ThreadEntry::ToolCall(call) => call,
            entry => panic!("entry {ix} is not a tool call: {entry:?}"),
        }
    }

    fn user_message() -> ThreadEvent {
        ThreadEvent::UserMessage {
            id: "u1".into(),
            content: vec![PromptSegment::Text("hi".to_owned())],
            attachments: Vec::new(),
        }
    }

    #[test]
    fn a_user_message_starts_generating() {
        let mut thread = Thread::default();
        assert_eq!(thread.apply(user_message()), EntryChange::inserted(0));
        assert_eq!(thread.status(), ThreadStatus::Generating);
    }

    #[test]
    fn deltas_of_one_kind_stream_into_one_chunk() {
        let mut thread = Thread::default();
        assert_eq!(thread.apply(text("m1", "Hel")), EntryChange::inserted(0));
        assert_eq!(
            thread.apply(text("m1", "lo")),
            EntryChange::updated(vec![0])
        );
        assert_eq!(
            thread.entries(),
            [ThreadEntry::Assistant {
                id: "m1".into(),
                chunks: vec![AssistantChunk::Text("Hello".to_owned())],
            }]
        );
    }

    #[test]
    fn switching_kind_opens_a_new_chunk_in_the_same_message() {
        let mut thread = Thread::default();
        thread.apply(thinking("m1", "hmm"));
        thread.apply(text("m1", "ok"));
        thread.apply(thinking("m1", "again"));
        assert_eq!(
            thread.entries(),
            [ThreadEntry::Assistant {
                id: "m1".into(),
                chunks: vec![
                    AssistantChunk::Thought("hmm".to_owned()),
                    AssistantChunk::Text("ok".to_owned()),
                    AssistantChunk::Thought("again".to_owned()),
                ],
            }]
        );
    }

    #[test]
    fn text_after_a_tool_call_starts_a_new_entry() {
        let mut thread = Thread::default();
        thread.apply(text("m1", "before"));
        thread.apply(tool_call("t1"));
        assert_eq!(thread.apply(text("m1", "after")), EntryChange::inserted(2));
    }

    #[test]
    fn a_different_message_id_starts_a_new_entry() {
        let mut thread = Thread::default();
        thread.apply(text("m1", "a"));
        assert_eq!(thread.apply(text("m2", "b")), EntryChange::inserted(1));
    }

    #[test]
    fn an_update_changes_only_present_fields_and_replaces_content() {
        let mut thread = Thread::default();
        thread.apply(tool_call("t1"));
        let first = vec![ToolContent::Markdown("one".to_owned())];
        thread.apply(ThreadEvent::ToolCallUpdate {
            id: "t1".into(),
            fields: ToolCallFields {
                content: Some(first),
                ..Default::default()
            },
        });
        let second = vec![ToolContent::Markdown("two".to_owned())];
        let change = thread.apply(ThreadEvent::ToolCallUpdate {
            id: "t1".into(),
            fields: ToolCallFields {
                status: Some(ToolCallStatus::Completed),
                content: Some(second.clone()),
                ..Default::default()
            },
        });
        assert_eq!(change, EntryChange::updated(vec![0]));
        let call = call(&thread, 0);
        assert_eq!(call.title, "Edit main.rs");
        assert_eq!(call.status, ToolCallStatus::Completed);
        assert_eq!(call.content, second);
    }

    #[test]
    fn an_update_keeps_the_target_unless_it_brings_one() {
        let mut thread = Thread::default();
        thread.apply(ThreadEvent::ToolCall {
            id: "t1".into(),
            kind: ToolKind::Read,
            title: "Read".to_owned(),
            status: ToolCallStatus::Running,
            target: Some(ToolTarget::File {
                path: "a.rs".to_owned(),
                lines: None,
            }),
        });
        thread.apply(ThreadEvent::ToolCallUpdate {
            id: "t1".into(),
            fields: ToolCallFields::default(),
        });
        assert!(matches!(
            &call(&thread, 0).target,
            Some(ToolTarget::File { path, .. }) if path == "a.rs"
        ));
        let lines = Some(ToolTarget::File {
            path: "a.rs".to_owned(),
            lines: Some((1, 9)),
        });
        thread.apply(ThreadEvent::ToolCallUpdate {
            id: "t1".into(),
            fields: ToolCallFields {
                target: lines.clone(),
                ..Default::default()
            },
        });
        assert_eq!(call(&thread, 0).target, lines);
    }

    #[test]
    fn an_update_for_an_unknown_tool_call_is_ignored() {
        let mut thread = Thread::default();
        let change = thread.apply(ThreadEvent::ToolCallUpdate {
            id: "missing".into(),
            fields: ToolCallFields::default(),
        });
        assert_eq!(change, EntryChange::none());
        assert!(thread.entries().is_empty());
    }

    #[test]
    fn a_permission_request_waits_and_allowing_resumes() {
        let mut thread = Thread::default();
        thread.apply(user_message());
        thread.apply(tool_call("t1"));
        thread.apply(ThreadEvent::PermissionRequest {
            tool_call_id: "t1".into(),
            options: options(),
        });
        assert_eq!(thread.status(), ThreadStatus::AwaitingPermission);
        assert_eq!(call(&thread, 1).status, ToolCallStatus::AwaitingPermission);

        thread.apply(ThreadEvent::PermissionResolved {
            tool_call_id: "t1".into(),
            option_id: "allow".into(),
        });
        assert_eq!(thread.status(), ThreadStatus::Generating);
        let call = call(&thread, 1);
        assert_eq!(call.status, ToolCallStatus::Running);
        assert_eq!(
            call.permission.as_ref().and_then(|p| p.chosen.clone()),
            Some("allow".into())
        );
    }

    #[test]
    fn rejecting_marks_the_card_rejected() {
        let mut thread = Thread::default();
        thread.apply(user_message());
        thread.apply(tool_call("t1"));
        thread.apply(ThreadEvent::PermissionRequest {
            tool_call_id: "t1".into(),
            options: options(),
        });
        thread.apply(ThreadEvent::PermissionResolved {
            tool_call_id: "t1".into(),
            option_id: "reject".into(),
        });
        assert_eq!(call(&thread, 1).status, ToolCallStatus::Rejected);
    }

    #[test]
    fn a_permission_request_for_an_unknown_call_keeps_the_status() {
        let mut thread = Thread::default();
        thread.apply(user_message());
        let change = thread.apply(ThreadEvent::PermissionRequest {
            tool_call_id: "missing".into(),
            options: options(),
        });
        assert_eq!(change, EntryChange::none());
        assert_eq!(thread.status(), ThreadStatus::Generating);
    }

    #[test]
    fn cancelling_cancels_only_unfinished_cards() {
        let mut thread = Thread::default();
        thread.apply(user_message());
        thread.apply(tool_call("done"));
        thread.apply(ThreadEvent::ToolCallUpdate {
            id: "done".into(),
            fields: ToolCallFields {
                status: Some(ToolCallStatus::Completed),
                ..Default::default()
            },
        });
        thread.apply(tool_call("running"));
        let change = thread.apply(ThreadEvent::Stop {
            reason: StopReason::Cancelled,
        });
        assert_eq!(
            change,
            EntryChange {
                removed: None,
                updated: vec![2],
                inserted: Some(3),
            }
        );
        assert_eq!(thread.status(), ThreadStatus::Idle);
        assert_eq!(call(&thread, 1).status, ToolCallStatus::Completed);
        assert_eq!(call(&thread, 2).status, ToolCallStatus::Canceled);
    }

    #[test]
    fn ending_a_turn_leaves_cards_alone() {
        let mut thread = Thread::default();
        thread.apply(user_message());
        thread.apply(tool_call("t1"));
        let change = thread.apply(ThreadEvent::Stop {
            reason: StopReason::EndTurn,
        });
        assert_eq!(change, EntryChange::inserted(2));
        assert_eq!(thread.status(), ThreadStatus::Idle);
        assert_eq!(call(&thread, 1).status, ToolCallStatus::Running);
        assert_eq!(
            thread.entries()[2],
            ThreadEntry::TurnEnd {
                reason: StopReason::EndTurn,
                tool_calls: 1,
            }
        );
    }

    fn notice(id: &str, kind: NoticeKind, message: &str) -> ThreadEvent {
        ThreadEvent::Notice {
            id: id.into(),
            kind,
            message: message.to_owned(),
            actions: Vec::new(),
        }
    }

    #[test]
    fn a_notice_updates_in_place_and_clears() {
        let mut thread = Thread::default();
        thread.apply(user_message());
        assert_eq!(
            thread.apply(notice("retry", NoticeKind::Retrying, "1 of 3")),
            EntryChange::inserted(1)
        );
        assert_eq!(
            thread.apply(notice("retry", NoticeKind::Retrying, "2 of 3")),
            EntryChange::updated(vec![1])
        );
        assert!(matches!(
            &thread.entries()[1],
            ThreadEntry::Notice(Notice { message, .. }) if message == "2 of 3"
        ));
        let change = thread.apply(ThreadEvent::ClearNotice { id: "retry".into() });
        assert_eq!(change.removed, Some(1..2));
        assert_eq!(thread.entries().len(), 1);
        assert!(
            thread
                .apply(ThreadEvent::ClearNotice { id: "retry".into() })
                .is_empty()
        );
    }

    #[test]
    fn stopping_an_idle_thread_adds_no_turn_end() {
        let mut thread = Thread::default();
        let change = thread.apply(ThreadEvent::Stop {
            reason: StopReason::EndTurn,
        });
        assert!(change.is_empty());
        assert!(thread.entries().is_empty());
    }

    #[test]
    fn a_turn_end_counts_only_its_own_tool_calls() {
        let mut thread = Thread::default();
        thread.apply(user_message());
        thread.apply(tool_call("a"));
        thread.apply(ThreadEvent::Stop {
            reason: StopReason::EndTurn,
        });
        thread.apply(user_message());
        thread.apply(tool_call("b"));
        thread.apply(tool_call("c"));
        thread.apply(ThreadEvent::Stop {
            reason: StopReason::EndTurn,
        });
        assert_eq!(
            thread.entries().last(),
            Some(&ThreadEntry::TurnEnd {
                reason: StopReason::EndTurn,
                tool_calls: 2,
            })
        );
    }

    fn edit(thread: &mut Thread, id: &str, path: &str, new: &str, status: ToolCallStatus) {
        thread.apply(tool_call(id));
        thread.apply(ThreadEvent::ToolCallUpdate {
            id: id.into(),
            fields: ToolCallFields {
                status: Some(status),
                content: Some(vec![ToolContent::Diff {
                    path: path.to_owned(),
                    old_text: Some("a\n".to_owned()),
                    new_text: new.to_owned(),
                }]),
                ..Default::default()
            },
        });
    }

    #[test]
    fn changed_files_sum_completed_edits_until_resolved() {
        let mut thread = Thread::default();
        edit(&mut thread, "e1", "a.rs", "b\n", ToolCallStatus::Completed);
        edit(
            &mut thread,
            "e2",
            "b.rs",
            "a\nb\n",
            ToolCallStatus::Completed,
        );
        edit(
            &mut thread,
            "e3",
            "a.rs",
            "b\nc\n",
            ToolCallStatus::Completed,
        );
        edit(&mut thread, "e4", "c.rs", "x\n", ToolCallStatus::Rejected);
        assert_eq!(
            thread.changed_files(),
            [
                ChangedFile {
                    path: "a.rs".to_owned(),
                    added: 3,
                    removed: 2,
                },
                ChangedFile {
                    path: "b.rs".to_owned(),
                    added: 1,
                    removed: 0,
                },
            ]
        );
        thread.resolve_edits(&["a.rs".to_owned()]);
        assert_eq!(thread.changed_files().len(), 1);
        edit(&mut thread, "e5", "a.rs", "z\n", ToolCallStatus::Completed);
        assert_eq!(thread.changed_files().len(), 2);
    }

    #[test]
    fn the_latest_plan_and_title_come_from_the_thread() {
        let mut thread = Thread::default();
        thread.apply(ThreadEvent::UserMessage {
            id: "u".into(),
            content: vec![
                PromptSegment::Text("\n  Fix ".to_owned()),
                PromptSegment::Mention {
                    id: "file:///a.rs".to_owned(),
                    label: "a.rs".to_owned(),
                },
                PromptSegment::Text(" please\nmore".to_owned()),
            ],
            attachments: Vec::new(),
        });
        assert_eq!(thread.title().as_deref(), Some("Fix @a.rs please"));
        assert!(thread.latest_plan().is_none());
        for (id, text) in [("p1", "old"), ("p2", "new")] {
            thread.apply(tool_call(id));
            thread.apply(ThreadEvent::ToolCallUpdate {
                id: id.into(),
                fields: ToolCallFields {
                    content: Some(vec![ToolContent::Todo(vec![TodoItem {
                        text: text.to_owned(),
                        status: crate::event::TodoStatus::Pending,
                    }])]),
                    ..Default::default()
                },
            });
        }
        assert_eq!(
            thread.latest_plan().map(|plan| plan[0].text.as_str()),
            Some("new")
        );
    }

    #[test]
    fn a_long_title_is_cut_with_an_ellipsis() {
        let mut thread = Thread::default();
        thread.apply(ThreadEvent::UserMessage {
            id: "u".into(),
            content: vec![PromptSegment::Text("x".repeat(80))],
            attachments: Vec::new(),
        });
        assert_eq!(thread.title(), Some(format!("{}…", "x".repeat(60))));
    }

    #[test]
    fn truncating_drops_a_prompt_and_what_followed() {
        let mut thread = Thread::default();
        thread.apply(user_message());
        thread.apply(tool_call("a"));
        let change = thread.apply(ThreadEvent::Truncate { from: "u1".into() });
        assert_eq!(change.removed, Some(0..2));
        assert!(thread.entries().is_empty());
        assert_eq!(thread.status(), ThreadStatus::Idle);
        assert!(
            thread
                .apply(ThreadEvent::Truncate { from: "u1".into() })
                .is_empty()
        );
    }

    #[test]
    fn a_turn_knows_the_prompt_that_started_it() {
        let mut thread = Thread::default();
        thread.apply(user_message());
        thread.apply(tool_call("a"));
        assert_eq!(thread.turn_message(1), Some(&"u1".into()));
        assert_eq!(thread.turn_message(9), None);
    }

    #[test]
    fn an_update_replaces_card_actions_only_when_it_brings_them() {
        let mut thread = Thread::default();
        thread.apply(tool_call("t1"));
        let actions = vec![Action::new("rerun", "Run again")];
        thread.apply(ThreadEvent::ToolCallUpdate {
            id: "t1".into(),
            fields: ToolCallFields {
                actions: Some(actions.clone()),
                ..Default::default()
            },
        });
        thread.apply(ThreadEvent::ToolCallUpdate {
            id: "t1".into(),
            fields: ToolCallFields::default(),
        });
        assert_eq!(call(&thread, 0).actions, actions);
    }

    #[test]
    fn the_user_title_wins_over_the_agent_title_and_the_prompt() {
        let mut thread = Thread::default();
        thread.apply(user_message());
        assert_eq!(thread.title().as_deref(), Some("hi"));
        thread.apply(ThreadEvent::Title {
            title: Some("Greeting".to_owned()),
        });
        assert_eq!(thread.title().as_deref(), Some("Greeting"));
        thread.set_title(Some("  Mine  ".to_owned()));
        assert_eq!(thread.title().as_deref(), Some("Mine"));
        assert!(thread.has_user_title());
        thread.set_title(Some("   ".to_owned()));
        assert_eq!(thread.title().as_deref(), Some("Greeting"));
        thread.clear();
        assert_eq!(thread.title(), None);
    }
}
