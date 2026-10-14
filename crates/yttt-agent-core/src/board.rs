use serde::{Deserialize, Serialize};

use crate::{AgentTurnState, model::bounded_text};

const MAX_TASKS: usize = 512;
const MAX_EXECUTIONS: usize = 256;
const MAX_MESSAGES: usize = 256;
const MAX_QUESTIONS: usize = 128;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentPlanStatus {
    Pending,
    InProgress,
    Completed,
    Blocked,
    Abandoned,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentPlanItem {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub phase: Option<String>,
    pub status: AgentPlanStatus,
    #[serde(default)]
    pub blocker: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentBoardTask {
    #[serde(flatten)]
    pub item: AgentPlanItem,
    pub execution_id: String,
    pub created_at: u64,
    pub updated_at: u64,
    pub accepted_at: Option<u64>,
    /// A later plan omitted this row. Keep its last observed status, not a guessed completion.
    #[serde(default)]
    pub retired: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentExecutionRecord {
    pub id: String,
    #[serde(default)]
    pub parent_id: Option<String>,
    pub name: String,
    #[serde(default)]
    pub task: Option<String>,
    pub state: AgentTurnState,
    pub started_at: u64,
    pub updated_at: u64,
    pub ended_at: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentMessageRole {
    User,
    Assistant,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentBoardMessage {
    pub id: String,
    pub execution_id: String,
    pub role: AgentMessageRole,
    pub text: String,
    pub timestamp: u64,
    #[serde(default)]
    pub truncated: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentQuestionOption {
    pub label: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub preview: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentQuestion {
    pub id: String,
    pub question: String,
    #[serde(default)]
    pub header: Option<String>,
    pub options: Vec<AgentQuestionOption>,
    #[serde(default)]
    pub multi: bool,
    #[serde(default)]
    pub recommended: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentQuestionRequest {
    pub id: String,
    pub execution_id: String,
    pub tool_call_id: String,
    pub questions: Vec<AgentQuestion>,
    /// True only while a provider adapter owns an actual answer channel.
    pub answerable: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentQuestionAnswer {
    pub question_id: String,
    #[serde(default)]
    pub selected_options: Vec<String>,
    #[serde(default)]
    pub custom_input: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentQuestionState {
    Pending,
    Submitted,
    Answered,
    Cancelled,
    Unavailable,
}

impl AgentQuestionState {
    pub fn is_open(self) -> bool {
        matches!(self, Self::Pending | Self::Submitted | Self::Unavailable)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentAnswerSource {
    Native,
    Board,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentQuestionRecord {
    pub request: AgentQuestionRequest,
    pub state: AgentQuestionState,
    pub answers: Vec<AgentQuestionAnswer>,
    pub answer_source: Option<AgentAnswerSource>,
    pub created_at: u64,
    pub updated_at: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AgentBoardEvent {
    Plan {
        execution_id: String,
        items: Vec<AgentPlanItem>,
    },
    Execution {
        execution: AgentExecutionRecord,
    },
    Message {
        message: AgentBoardMessage,
    },
    QuestionOpened {
        request: AgentQuestionRequest,
    },
    QuestionResolved {
        request_id: String,
        state: AgentQuestionState,
        answers: Vec<AgentQuestionAnswer>,
        source: AgentAnswerSource,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AgentBoardCommand {
    AnswerQuestion {
        request_id: String,
        answers: Vec<AgentQuestionAnswer>,
    },
    AcceptTask {
        execution_id: String,
        task_id: String,
    },
}

/// Bounded, persisted observations. Provider completion and human acceptance are independent.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentBoardSnapshot {
    pub tasks: Vec<AgentBoardTask>,
    pub executions: Vec<AgentExecutionRecord>,
    pub messages: Vec<AgentBoardMessage>,
    pub questions: Vec<AgentQuestionRecord>,
    pub dropped_messages: u64,
}

fn valid_id(id: &str) -> bool {
    !id.trim().is_empty() && id.len() <= 512
}

impl AgentBoardSnapshot {
    pub fn apply(&mut self, event: AgentBoardEvent, now: u64) -> bool {
        match event {
            AgentBoardEvent::Plan {
                execution_id,
                items,
            } => {
                if !valid_id(&execution_id)
                    || items.len() > 128
                    || items
                        .iter()
                        .any(|item| !valid_id(&item.id) || item.title.trim().is_empty())
                {
                    return false;
                }
                for task in self
                    .tasks
                    .iter_mut()
                    .filter(|task| task.execution_id == execution_id)
                {
                    task.retired = !items.iter().any(|item| item.id == task.item.id);
                }
                for mut item in items {
                    item.title = bounded_text(item.title, 2048);
                    item.phase = item.phase.map(|text| bounded_text(text, 256));
                    item.blocker = item.blocker.map(|text| bounded_text(text, 2048));
                    if let Some(task) = self
                        .tasks
                        .iter_mut()
                        .find(|task| task.execution_id == execution_id && task.item.id == item.id)
                    {
                        if task.item != item {
                            task.accepted_at = None;
                            task.item = item;
                            task.updated_at = now;
                        }
                        task.retired = false;
                    } else {
                        if self.tasks.len() >= MAX_TASKS {
                            if let Some(index) = self.tasks.iter().position(|task| {
                                task.retired
                                    || matches!(
                                        task.item.status,
                                        AgentPlanStatus::Completed | AgentPlanStatus::Abandoned
                                    )
                            }) {
                                self.tasks.remove(index);
                            } else {
                                continue;
                            }
                        }
                        self.tasks.push(AgentBoardTask {
                            item,
                            execution_id: execution_id.clone(),
                            created_at: now,
                            updated_at: now,
                            accepted_at: None,
                            retired: false,
                        });
                    }
                }
                true
            }
            AgentBoardEvent::Execution { mut execution } => {
                if !valid_id(&execution.id)
                    || execution
                        .parent_id
                        .as_deref()
                        .is_some_and(|id| !valid_id(id))
                {
                    return false;
                }
                execution.name = bounded_text(execution.name, 256);
                execution.task = execution.task.map(|text| bounded_text(text, 4096));
                if execution.started_at == 0 {
                    execution.started_at = now;
                }
                execution.updated_at = now;
                if let Some(current) = self
                    .executions
                    .iter_mut()
                    .find(|item| item.id == execution.id)
                {
                    // Finished executions never restart under the same identity.
                    if current.ended_at.is_some() && execution.ended_at.is_none() {
                        return false;
                    }
                    execution.started_at = current.started_at;
                    if execution.task.is_none() {
                        execution.task.clone_from(&current.task);
                    }
                    *current = execution;
                } else {
                    if self.executions.len() >= MAX_EXECUTIONS {
                        if let Some(index) = self
                            .executions
                            .iter()
                            .position(|item| item.ended_at.is_some())
                        {
                            self.executions.remove(index);
                        } else {
                            return false;
                        }
                    }
                    self.executions.push(execution);
                }
                true
            }
            AgentBoardEvent::Message { mut message } => {
                if !valid_id(&message.id)
                    || !valid_id(&message.execution_id)
                    || message.text.trim().is_empty()
                {
                    return false;
                }
                message.truncated |= message.text.len() > 16 * 1024;
                message.text = bounded_text(message.text, 16 * 1024);
                if message.timestamp == 0 {
                    message.timestamp = now;
                }
                if let Some(current) = self
                    .messages
                    .iter_mut()
                    .find(|item| item.id == message.id && item.execution_id == message.execution_id)
                {
                    if current == &message {
                        return false;
                    }
                    *current = message;
                } else {
                    if self.messages.len() >= MAX_MESSAGES {
                        self.messages.remove(0);
                        self.dropped_messages = self.dropped_messages.saturating_add(1);
                    }
                    self.messages.push(message);
                }
                true
            }
            AgentBoardEvent::QuestionOpened { request } => {
                if !valid_question_request(&request) {
                    return false;
                }
                if let Some(current) = self
                    .questions
                    .iter_mut()
                    .find(|item| item.request.id == request.id)
                {
                    if current.state == AgentQuestionState::Unavailable
                        && current.request == request
                    {
                        current.state = AgentQuestionState::Pending;
                        current.answers.clear();
                        current.answer_source = None;
                        current.updated_at = now;
                        return true;
                    }
                    return false;
                }
                if self.questions.len() >= MAX_QUESTIONS {
                    if let Some(index) =
                        self.questions.iter().position(|item| !item.state.is_open())
                    {
                        self.questions.remove(index);
                    } else {
                        return false;
                    }
                }
                self.questions.push(AgentQuestionRecord {
                    request,
                    state: AgentQuestionState::Pending,
                    answers: Vec::new(),
                    answer_source: None,
                    created_at: now,
                    updated_at: now,
                });
                true
            }
            AgentBoardEvent::QuestionResolved {
                request_id,
                state,
                answers,
                source,
            } => {
                if !matches!(
                    state,
                    AgentQuestionState::Answered | AgentQuestionState::Cancelled
                ) {
                    return false;
                }
                let Some(question) = self
                    .questions
                    .iter_mut()
                    .find(|item| item.request.id == request_id)
                else {
                    return false;
                };
                if !question.state.is_open() {
                    return false;
                }
                if state == AgentQuestionState::Answered
                    && validate_answers(&question.request, &answers).is_err()
                {
                    return false;
                }
                question.state = state;
                question.answers = answers;
                question.answer_source = Some(source);
                question.updated_at = now;
                true
            }
        }
    }

    pub fn command(&mut self, command: AgentBoardCommand, now: u64) -> Result<(), &'static str> {
        match command {
            AgentBoardCommand::AnswerQuestion {
                request_id,
                answers,
            } => {
                let question = self
                    .questions
                    .iter_mut()
                    .find(|item| item.request.id == request_id)
                    .ok_or("question no longer exists")?;
                if !question.request.answerable {
                    return Err("this provider question is read-only");
                }
                validate_answers(&question.request, &answers)?;
                if question.state == AgentQuestionState::Submitted && question.answers == answers {
                    return Ok(());
                }
                if question.state != AgentQuestionState::Pending {
                    return Err("question is no longer awaiting an answer");
                }
                question.state = AgentQuestionState::Submitted;
                question.answers = answers;
                question.answer_source = Some(AgentAnswerSource::Board);
                question.updated_at = now;
            }
            AgentBoardCommand::AcceptTask {
                execution_id,
                task_id,
            } => {
                let task = self
                    .tasks
                    .iter_mut()
                    .find(|item| item.execution_id == execution_id && item.item.id == task_id)
                    .ok_or("task no longer exists")?;
                if task.item.status != AgentPlanStatus::Completed {
                    return Err("only a provider-completed task can be accepted");
                }
                task.accepted_at = Some(now);
            }
        }
        Ok(())
    }

    /// Loss of observation does not erase pending questions or invent task outcomes.
    pub fn mark_unavailable(&mut self, now: u64) {
        for question in &mut self.questions {
            if matches!(
                question.state,
                AgentQuestionState::Pending | AgentQuestionState::Submitted
            ) {
                question.state = AgentQuestionState::Unavailable;
                question.updated_at = now;
            }
        }
        for execution in &mut self.executions {
            if execution.ended_at.is_none() {
                execution.state = AgentTurnState::Unknown;
                execution.updated_at = now;
            }
        }
    }

    pub fn finish_executions(&mut self, state: AgentTurnState, now: u64) {
        self.mark_unavailable(now);
        for execution in &mut self.executions {
            if execution.ended_at.is_none() {
                execution.state = state;
                if state != AgentTurnState::Unknown {
                    execution.ended_at = Some(now);
                }
                execution.updated_at = now;
            }
        }
    }

    /// Carry observations across Host/agent replacement without reviving an old answer channel.
    /// Current provider observations win; missing retained records are read-only history.
    pub fn merge_retained(&mut self, previous: &Self) {
        for task in &previous.tasks {
            if let Some(current) = self.tasks.iter_mut().find(|current| {
                current.execution_id == task.execution_id && current.item.id == task.item.id
            }) {
                if current.item == task.item && current.accepted_at.is_none() {
                    current.accepted_at = task.accepted_at;
                }
            } else if self.tasks.len() < MAX_TASKS {
                let mut retained = task.clone();
                retained.retired = true;
                self.tasks.push(retained);
            }
        }
        for execution in &previous.executions {
            if self.executions.len() < MAX_EXECUTIONS
                && !self
                    .executions
                    .iter()
                    .any(|current| current.id == execution.id)
            {
                let mut retained = execution.clone();
                if retained.ended_at.is_none() {
                    retained.state = AgentTurnState::Unknown;
                }
                self.executions.push(retained);
            }
        }
        for message in &previous.messages {
            if self.messages.len() < MAX_MESSAGES
                && !self.messages.iter().any(|current| {
                    current.id == message.id && current.execution_id == message.execution_id
                })
            {
                self.messages.push(message.clone());
            }
        }
        for question in &previous.questions {
            if self.questions.len() < MAX_QUESTIONS
                && !self
                    .questions
                    .iter()
                    .any(|current| current.request.id == question.request.id)
            {
                let mut retained = question.clone();
                if retained.state.is_open() {
                    retained.state = AgentQuestionState::Unavailable;
                }
                self.questions.push(retained);
            }
        }
        self.dropped_messages = self.dropped_messages.max(previous.dropped_messages);
    }
}

fn valid_question_request(request: &AgentQuestionRequest) -> bool {
    valid_id(&request.id)
        && valid_id(&request.execution_id)
        && valid_id(&request.tool_call_id)
        && request
            .questions
            .iter()
            .map(|question| {
                question.id.len()
                    + question.question.len()
                    + question.header.as_ref().map_or(0, String::len)
                    + question
                        .options
                        .iter()
                        .map(|option| {
                            option.label.len()
                                + option.description.as_ref().map_or(0, String::len)
                                + option.preview.as_ref().map_or(0, String::len)
                        })
                        .sum::<usize>()
            })
            .sum::<usize>()
            <= 32 * 1024
        && !request.questions.is_empty()
        && request.questions.len() <= 16
        && request
            .questions
            .iter()
            .enumerate()
            .all(|(index, question)| {
                valid_id(&question.id)
                    && !question.question.trim().is_empty()
                    && question.question.len() <= 8192
                    && !request.questions[..index]
                        .iter()
                        .any(|item| item.id == question.id)
                    && question
                        .header
                        .as_ref()
                        .is_none_or(|text| text.len() <= 256)
                    && question.options.len() <= 32
                    && question
                        .recommended
                        .is_none_or(|index| index < question.options.len())
                    && question.options.iter().all(|option| {
                        !option.label.is_empty()
                            && option.label.len() <= 1024
                            && option
                                .description
                                .as_ref()
                                .is_none_or(|text| text.len() <= 4096)
                            && option
                                .preview
                                .as_ref()
                                .is_none_or(|text| text.len() <= 8192)
                    })
            })
}

fn validate_answers(
    request: &AgentQuestionRequest,
    answers: &[AgentQuestionAnswer],
) -> Result<(), &'static str> {
    if answers.len() != request.questions.len() {
        return Err("answer every question before submitting");
    }
    for question in &request.questions {
        let mut matches = answers
            .iter()
            .filter(|answer| answer.question_id == question.id);
        let answer = matches
            .next()
            .ok_or("answer references an unknown question")?;
        if matches.next().is_some() {
            return Err("duplicate question answer");
        }
        if answer.selected_options.len() > question.options.len()
            || (!question.multi && answer.selected_options.len() > 1)
            || answer
                .selected_options
                .iter()
                .enumerate()
                .any(|(index, selected)| {
                    !question
                        .options
                        .iter()
                        .any(|option| &option.label == selected)
                        || answer.selected_options[..index].contains(selected)
                })
        {
            return Err("answer contains an invalid option");
        }
        let custom = answer.custom_input.as_deref().unwrap_or("").trim();
        if custom.len() > 8192 {
            return Err("custom answer is too long");
        }
        if answer.selected_options.is_empty() && custom.is_empty() {
            return Err("answer is empty");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> AgentQuestionRequest {
        AgentQuestionRequest {
            id: "ask-1".into(),
            execution_id: "run-1".into(),
            tool_call_id: "tool-1".into(),
            answerable: true,
            questions: vec![AgentQuestion {
                id: "choice".into(),
                question: "Proceed?".into(),
                header: None,
                options: ["Yes", "No"]
                    .into_iter()
                    .map(|label| AgentQuestionOption {
                        label: label.into(),
                        description: None,
                        preview: None,
                    })
                    .collect(),
                multi: false,
                recommended: None,
            }],
        }
    }

    fn answers(option: &str) -> Vec<AgentQuestionAnswer> {
        vec![AgentQuestionAnswer {
            question_id: "choice".into(),
            selected_options: vec![option.into()],
            custom_input: None,
        }]
    }

    #[test]
    fn submitted_answer_waits_for_provider_and_native_winner_cannot_be_overwritten() {
        let mut board = AgentBoardSnapshot::default();
        board.apply(AgentBoardEvent::QuestionOpened { request: request() }, 1);
        board
            .command(
                AgentBoardCommand::AnswerQuestion {
                    request_id: "ask-1".into(),
                    answers: answers("Yes"),
                },
                2,
            )
            .unwrap();
        assert_eq!(board.questions[0].state, AgentQuestionState::Submitted);
        board.apply(
            AgentBoardEvent::QuestionResolved {
                request_id: "ask-1".into(),
                state: AgentQuestionState::Answered,
                answers: answers("No"),
                source: AgentAnswerSource::Native,
            },
            3,
        );
        board.apply(
            AgentBoardEvent::QuestionResolved {
                request_id: "ask-1".into(),
                state: AgentQuestionState::Answered,
                answers: answers("Yes"),
                source: AgentAnswerSource::Board,
            },
            4,
        );
        assert_eq!(board.questions[0].state, AgentQuestionState::Answered);
        assert_eq!(board.questions[0].answers, answers("No"));
        assert_eq!(
            board.questions[0].answer_source,
            Some(AgentAnswerSource::Native)
        );
        assert!(
            board
                .command(
                    AgentBoardCommand::AnswerQuestion {
                        request_id: "ask-1".into(),
                        answers: answers("Yes")
                    },
                    5
                )
                .is_err()
        );
    }

    #[test]
    fn invalid_and_incomplete_answers_leave_question_pending() {
        let mut board = AgentBoardSnapshot::default();
        board.apply(AgentBoardEvent::QuestionOpened { request: request() }, 1);
        for invalid in [
            Vec::new(),
            answers("Unknown"),
            vec![AgentQuestionAnswer {
                question_id: "choice".into(),
                selected_options: vec!["Yes".into(), "No".into()],
                custom_input: None,
            }],
        ] {
            assert!(
                board
                    .command(
                        AgentBoardCommand::AnswerQuestion {
                            request_id: "ask-1".into(),
                            answers: invalid
                        },
                        2
                    )
                    .is_err()
            );
            assert_eq!(board.questions[0].state, AgentQuestionState::Pending);
            assert!(board.questions[0].answers.is_empty());
        }
    }

    #[test]
    fn host_replacement_retains_question_without_reviving_answer_channel() {
        let mut previous = AgentBoardSnapshot::default();
        previous.apply(AgentBoardEvent::QuestionOpened { request: request() }, 1);
        previous.apply(
            AgentBoardEvent::Execution {
                execution: AgentExecutionRecord {
                    id: "run-1".into(),
                    parent_id: None,
                    name: "OMP".into(),
                    task: None,
                    state: AgentTurnState::Working,
                    started_at: 1,
                    updated_at: 1,
                    ended_at: None,
                },
            },
            1,
        );
        let mut replacement = AgentBoardSnapshot::default();
        replacement.merge_retained(&previous);
        assert_eq!(replacement.questions[0].request, request());
        assert_eq!(
            replacement.questions[0].state,
            AgentQuestionState::Unavailable
        );
        assert_eq!(replacement.executions[0].state, AgentTurnState::Unknown);
        assert_eq!(replacement.executions[0].ended_at, None);
        assert!(
            replacement
                .command(
                    AgentBoardCommand::AnswerQuestion {
                        request_id: "ask-1".into(),
                        answers: answers("Yes")
                    },
                    2
                )
                .is_err()
        );
    }

    #[test]
    fn reopening_completed_task_requires_new_human_acceptance() {
        let mut board = AgentBoardSnapshot::default();
        let plan = |status| AgentBoardEvent::Plan {
            execution_id: "run-1".into(),
            items: vec![AgentPlanItem {
                id: "task-1".into(),
                title: "Ship".into(),
                phase: None,
                status,
                blocker: None,
            }],
        };
        board.apply(plan(AgentPlanStatus::Completed), 1);
        assert_eq!(board.tasks[0].accepted_at, None);
        board
            .command(
                AgentBoardCommand::AcceptTask {
                    execution_id: "run-1".into(),
                    task_id: "task-1".into(),
                },
                2,
            )
            .unwrap();
        assert_eq!(board.tasks[0].accepted_at, Some(2));
        board.apply(plan(AgentPlanStatus::InProgress), 3);
        assert_eq!(board.tasks[0].accepted_at, None);
        assert!(
            board
                .command(
                    AgentBoardCommand::AcceptTask {
                        execution_id: "run-1".into(),
                        task_id: "task-1".into()
                    },
                    4
                )
                .is_err()
        );
        board.apply(
            AgentBoardEvent::Plan {
                execution_id: "run-1".into(),
                items: Vec::new(),
            },
            5,
        );
        assert!(board.tasks[0].retired);
        assert_eq!(board.tasks[0].item.status, AgentPlanStatus::InProgress);
    }

    #[test]
    fn pending_native_ask_takes_precedence_over_late_working_events() {
        use crate::{AgentEventKind, AgentInstanceId, AgentReducer, ProviderId, WaitingReason};
        let mut reducer = AgentReducer::new(
            AgentInstanceId::random(),
            ProviderId::new("omp").unwrap(),
            0,
        );
        reducer.process_starting(1, 0);
        reducer.process_started(1, 0);
        reducer.apply(
            1,
            AgentEventKind::Board {
                event: AgentBoardEvent::Execution {
                    execution: AgentExecutionRecord {
                        id: "run-1".into(),
                        parent_id: None,
                        name: "OMP".into(),
                        task: None,
                        state: AgentTurnState::Working,
                        started_at: 1,
                        updated_at: 1,
                        ended_at: None,
                    },
                },
            },
            1,
        );
        reducer.apply(
            1,
            AgentEventKind::Board {
                event: AgentBoardEvent::QuestionOpened { request: request() },
            },
            2,
        );
        reducer.apply(1, AgentEventKind::Working, 3);
        assert_eq!(reducer.snapshot().turn_state, AgentTurnState::Waiting);
        assert_eq!(
            reducer.snapshot().waiting_reason,
            Some(WaitingReason::UserInput)
        );
        reducer.apply(
            1,
            AgentEventKind::Board {
                event: AgentBoardEvent::QuestionResolved {
                    request_id: "ask-1".into(),
                    state: AgentQuestionState::Answered,
                    answers: answers("Yes"),
                    source: AgentAnswerSource::Board,
                },
            },
            4,
        );
        assert_eq!(reducer.snapshot().turn_state, AgentTurnState::Working);
        assert_eq!(reducer.snapshot().waiting_reason, None);
    }
}
