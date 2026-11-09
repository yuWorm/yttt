//! Rebuilds a board from an earlier copy, so a Client that already holds an
//! Agent's board receives what changed instead of the whole board again.
//!
//! A patch addresses list entries by position, not id: each step copies a run
//! of the earlier list or puts a new entry. A run is only copied where its
//! entries equal the newer ones, so applying a patch to the board it was made
//! from always yields the newer board exactly.

use serde::{Deserialize, Serialize};

use crate::{
    AgentBoardMessage, AgentBoardSnapshot, AgentBoardTask, AgentExecutionRecord,
    AgentQuestionRecord,
};

/// One step of rebuilding a list from an earlier copy of it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ListPatchStep<T> {
    /// Entries `from..from + count` of the earlier list.
    Keep {
        from: u32,
        count: u32,
    },
    Put(T),
}

/// How a board differs from an earlier one.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentBoardPatch {
    pub tasks: Vec<ListPatchStep<AgentBoardTask>>,
    pub executions: Vec<ListPatchStep<AgentExecutionRecord>>,
    pub messages: Vec<ListPatchStep<AgentBoardMessage>>,
    pub questions: Vec<ListPatchStep<AgentQuestionRecord>>,
    pub dropped_messages: u64,
}

impl AgentBoardSnapshot {
    /// What turns `base` into this board.
    pub fn patch_from(&self, base: &Self) -> AgentBoardPatch {
        AgentBoardPatch {
            tasks: diff_list(&base.tasks, &self.tasks),
            executions: diff_list(&base.executions, &self.executions),
            messages: diff_list(&base.messages, &self.messages),
            questions: diff_list(&base.questions, &self.questions),
            dropped_messages: self.dropped_messages,
        }
    }

    /// `base` with `patch` applied; `None` when the patch was made from another board.
    pub fn patched(base: &Self, patch: &AgentBoardPatch) -> Option<Self> {
        Some(Self {
            tasks: apply_list(&base.tasks, &patch.tasks)?,
            executions: apply_list(&base.executions, &patch.executions)?,
            messages: apply_list(&base.messages, &patch.messages)?,
            questions: apply_list(&base.questions, &patch.questions)?,
            dropped_messages: patch.dropped_messages,
        })
    }
}

/// Boards append at the end, drop from the front and change entries in place,
/// so each newer entry is looked for from just past the last one found.
fn diff_list<T: Clone + PartialEq>(base: &[T], next: &[T]) -> Vec<ListPatchStep<T>> {
    let mut steps = Vec::new();
    let mut cursor = 0;
    for entry in next {
        let Some(index) = base[cursor..]
            .iter()
            .position(|earlier| earlier == entry)
            .map(|offset| cursor + offset)
        else {
            steps.push(ListPatchStep::Put(entry.clone()));
            continue;
        };
        match steps.last_mut() {
            Some(ListPatchStep::Keep { from, count }) if (*from + *count) as usize == index => {
                *count += 1;
            }
            _ => steps.push(ListPatchStep::Keep {
                from: index as u32,
                count: 1,
            }),
        }
        cursor = index + 1;
    }
    steps
}

fn apply_list<T: Clone>(base: &[T], steps: &[ListPatchStep<T>]) -> Option<Vec<T>> {
    let mut list = Vec::new();
    for step in steps {
        match step {
            ListPatchStep::Keep { from, count } => {
                let from = *from as usize;
                list.extend_from_slice(base.get(from..from.checked_add(*count as usize)?)?);
            }
            ListPatchStep::Put(entry) => list.push(entry.clone()),
        }
    }
    Some(list)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AgentMessageRole;

    fn message(id: usize, text: &str) -> AgentBoardMessage {
        AgentBoardMessage {
            id: format!("message-{id}"),
            execution_id: "run".into(),
            role: AgentMessageRole::Assistant,
            text: text.into(),
            timestamp: id as u64,
            truncated: false,
        }
    }

    fn board(messages: impl IntoIterator<Item = AgentBoardMessage>) -> AgentBoardSnapshot {
        AgentBoardSnapshot {
            messages: messages.into_iter().collect(),
            ..AgentBoardSnapshot::default()
        }
    }

    fn round_trip(base: &AgentBoardSnapshot, next: &AgentBoardSnapshot) -> AgentBoardPatch {
        let patch = next.patch_from(base);
        assert_eq!(
            AgentBoardSnapshot::patched(base, &patch).as_ref(),
            Some(next)
        );
        patch
    }

    fn puts(patch: &AgentBoardPatch) -> usize {
        patch
            .messages
            .iter()
            .filter(|step| matches!(step, ListPatchStep::Put(_)))
            .count()
    }

    #[test]
    fn a_new_message_on_a_full_board_carries_only_that_message() {
        let base = board((0..256).map(|id| message(id, "earlier")));
        let next = board((1..257).map(|id| message(id, "earlier")));
        let patch = round_trip(&base, &next);
        assert_eq!(puts(&patch), 1);
        assert_eq!(patch.messages.len(), 2);
    }

    #[test]
    fn edits_removals_reorders_and_duplicates_rebuild_exactly() {
        let base = board([
            message(0, "a"),
            message(1, "b"),
            message(1, "b"),
            message(2, "c"),
            message(3, "d"),
        ]);
        let cases = [
            board([
                message(0, "a"),
                message(1, "B"),
                message(1, "b"),
                message(3, "d"),
            ]),
            board([
                message(3, "d"),
                message(2, "c"),
                message(1, "b"),
                message(0, "a"),
            ]),
            board([message(1, "b"), message(1, "b"), message(1, "b")]),
            board([]),
            board([message(9, "new")]),
            base.clone(),
        ];
        for next in &cases {
            round_trip(&base, next);
        }
        assert_eq!(puts(&round_trip(&base, &base)), 0);
    }

    #[test]
    fn a_patch_made_from_another_board_is_refused() {
        let base = board((0..4).map(|id| message(id, "x")));
        let patch = board((0..4).map(|id| message(id, "x"))).patch_from(&base);
        assert_eq!(AgentBoardSnapshot::patched(&board([]), &patch), None);
    }
}
