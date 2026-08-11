use serde::{Deserialize, Serialize};

use crate::{
    AgentAction, AgentEventKind, AgentExitReason, AgentInstanceId, AgentProcessExit,
    AgentProcessState, AgentSessionMetadata, AgentTask, AgentTurnState, AgentViewState,
    ChildAgentDescriptor, ChildAgentSnapshot, ChildAgentUpdate, ProviderId, TurnOutcome,
    WaitingReason, model::bounded_text,
};

const MAX_CHILD_AGENTS: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentSnapshot {
    pub instance_id: AgentInstanceId,
    pub provider_id: ProviderId,
    pub generation: u64,
    pub process_state: AgentProcessState,
    pub turn_state: AgentTurnState,
    pub waiting_reason: Option<WaitingReason>,
    pub waiting_message: Option<String>,
    pub task: Option<AgentTask>,
    pub current_action: Option<AgentAction>,
    pub last_action_failed: bool,
    #[serde(default)]
    pub children: Vec<ChildAgentSnapshot>,
    pub session: Option<AgentSessionMetadata>,
    pub process_exit: Option<AgentProcessExit>,
    pub state_started_at: u64,
    pub updated_at: u64,
}

impl AgentSnapshot {
    pub fn view_state(&self) -> AgentViewState {
        match self.process_state {
            AgentProcessState::Registered | AgentProcessState::Starting => AgentViewState::Starting,
            AgentProcessState::Running => match self.turn_state {
                AgentTurnState::Idle => AgentViewState::Idle,
                AgentTurnState::Working => AgentViewState::Working,
                AgentTurnState::Waiting => AgentViewState::Waiting,
                AgentTurnState::Completed => AgentViewState::Completed,
                AgentTurnState::Failed => AgentViewState::Failed,
                AgentTurnState::Interrupted => AgentViewState::Interrupted,
                AgentTurnState::Unknown => AgentViewState::Stale,
            },
            AgentProcessState::Exited => match self.turn_state {
                AgentTurnState::Failed => AgentViewState::Failed,
                AgentTurnState::Interrupted => AgentViewState::Interrupted,
                AgentTurnState::Completed => AgentViewState::Completed,
                AgentTurnState::Idle
                | AgentTurnState::Working
                | AgentTurnState::Waiting
                | AgentTurnState::Unknown => AgentViewState::Stale,
            },
        }
    }

    pub fn primary_text(&self) -> String {
        self.session
            .as_ref()
            .and_then(|session| session.title.clone())
            .or_else(|| {
                self.task
                    .as_ref()
                    .map(AgentTask::single_line_title)
                    .filter(|title| !title.is_empty())
            })
            .unwrap_or_else(|| self.provider_id.to_string())
    }

    pub fn secondary_text(&self) -> Option<String> {
        if self.turn_state == AgentTurnState::Waiting {
            return self
                .waiting_message
                .clone()
                .or_else(|| Some("Waiting for input".to_string()));
        }
        self.current_action.as_ref().map(AgentAction::label)
    }
}

#[derive(Clone, Debug)]
pub struct AgentReducer {
    snapshot: AgentSnapshot,
}

impl AgentReducer {
    pub fn new(instance_id: AgentInstanceId, provider_id: ProviderId, now: u64) -> Self {
        Self {
            snapshot: AgentSnapshot {
                instance_id,
                provider_id,
                generation: 0,
                process_state: AgentProcessState::Registered,
                turn_state: AgentTurnState::Idle,
                waiting_reason: None,
                waiting_message: None,
                task: None,
                current_action: None,
                last_action_failed: false,
                children: Vec::new(),
                session: None,
                process_exit: None,
                state_started_at: now,
                updated_at: now,
            },
        }
    }
    pub fn from_restored(
        instance_id: AgentInstanceId,
        provider_id: ProviderId,
        restored: &AgentSnapshot,
        now: u64,
    ) -> Self {
        let mut reducer = Self::new(instance_id, provider_id, now);
        reducer.snapshot.task = restored.task.clone();
        reducer.snapshot.session = restored.session.clone().map(sanitize_session_metadata);
        reducer
    }

    pub fn snapshot(&self) -> &AgentSnapshot {
        &self.snapshot
    }

    pub fn process_starting(&mut self, generation: u64, now: u64) {
        self.snapshot.generation = generation;
        self.snapshot.process_exit = None;
        self.set_process_state(AgentProcessState::Starting, now);
        self.set_turn_state(AgentTurnState::Idle, now);
        self.snapshot.current_action = None;
        self.snapshot.children.clear();
        self.snapshot.waiting_reason = None;
        self.snapshot.waiting_message = None;
        self.snapshot.updated_at = now;
    }

    pub fn process_started(&mut self, generation: u64, now: u64) -> bool {
        if generation != self.snapshot.generation {
            return false;
        }
        self.set_process_state(AgentProcessState::Running, now);
        self.snapshot.updated_at = now;
        true
    }

    pub fn process_exited(&mut self, generation: u64, exit: AgentProcessExit, now: u64) -> bool {
        if generation != self.snapshot.generation {
            return false;
        }
        self.snapshot.process_exit = Some(exit);
        self.set_process_state(AgentProcessState::Exited, now);
        let outcome = match exit.reason {
            AgentExitReason::KilledByUser => AgentTurnState::Interrupted,
            AgentExitReason::Failed | AgentExitReason::Disconnected => AgentTurnState::Failed,
            AgentExitReason::Completed if exit.code.unwrap_or_default() == 0 => {
                AgentTurnState::Completed
            }
            AgentExitReason::Completed => AgentTurnState::Failed,
        };
        self.set_turn_state(outcome, now);
        self.snapshot.current_action = None;
        self.snapshot.waiting_reason = None;
        self.snapshot.waiting_message = None;
        self.snapshot.children.clear();
        self.snapshot.updated_at = now;
        true
    }

    pub fn apply(&mut self, generation: u64, event: AgentEventKind, now: u64) -> bool {
        if generation != self.snapshot.generation
            || self.snapshot.process_state == AgentProcessState::Exited
        {
            return false;
        }
        match event {
            AgentEventKind::SessionStarted { metadata }
            | AgentEventKind::SessionUpdated { metadata } => {
                self.update_session(metadata, now);
            }
            AgentEventKind::TurnStarted { task } => {
                if let Some(task) = task {
                    let title = task.single_line_title();
                    let session = self
                        .snapshot
                        .session
                        .get_or_insert_with(AgentSessionMetadata::default);
                    if session.title.is_none() && !title.is_empty() {
                        session.title = Some(title);
                    }
                    self.snapshot.task = Some(task);
                }
                self.snapshot.children.clear();
                self.snapshot.current_action = None;
                self.snapshot.last_action_failed = false;
                self.clear_waiting();
                self.set_turn_state(AgentTurnState::Working, now);
            }
            AgentEventKind::Working => {
                self.clear_waiting();
                self.set_turn_state(AgentTurnState::Working, now);
            }
            AgentEventKind::ActionStarted { action } => {
                self.snapshot.current_action = Some(action);
                self.snapshot.last_action_failed = false;
                self.clear_waiting();
                self.set_turn_state(AgentTurnState::Working, now);
            }
            AgentEventKind::ActionFinished { action_id, failed } => {
                if action_id.is_none()
                    || self
                        .snapshot
                        .current_action
                        .as_ref()
                        .and_then(|action| action.id.as_ref())
                        == action_id.as_ref()
                {
                    self.snapshot.current_action = None;
                }
                self.snapshot.last_action_failed = failed;
                self.clear_waiting();
                self.set_turn_state(AgentTurnState::Working, now);
            }
            AgentEventKind::Waiting { reason, message } => {
                self.snapshot.waiting_reason = Some(reason);
                self.snapshot.waiting_message = message;
                self.set_turn_state(AgentTurnState::Waiting, now);
            }
            AgentEventKind::TurnFinished { outcome } => {
                self.snapshot.current_action = None;
                self.clear_waiting();
                self.snapshot.children.clear();
                let state = match outcome {
                    TurnOutcome::Completed => AgentTurnState::Completed,
                    TurnOutcome::Failed => AgentTurnState::Failed,
                    TurnOutcome::Interrupted => AgentTurnState::Interrupted,
                };
                self.set_turn_state(state, now);
            }
            AgentEventKind::ChildStarted { child } => {
                self.child_started(child, now);
            }
            AgentEventKind::ChildUpdated { child_id, update } => {
                self.child_updated(&child_id, update, now);
            }
            AgentEventKind::ChildFinished { child_id, .. } => {
                self.child_finished(&child_id);
            }
        }
        self.snapshot.updated_at = now;
        true
    }

    fn update_session(&mut self, metadata: AgentSessionMetadata, now: u64) {
        let metadata = sanitize_session_metadata(metadata);
        let current_session_id = self
            .snapshot
            .session
            .as_ref()
            .and_then(|session| session.session_id.as_ref());
        let identity_changed = current_session_id.is_some()
            && metadata.session_id.is_some()
            && current_session_id != metadata.session_id.as_ref();
        if identity_changed {
            self.snapshot.task = None;
            self.snapshot.current_action = None;
            self.snapshot.children.clear();
            self.snapshot.last_action_failed = false;
            self.clear_waiting();
            self.set_turn_state(AgentTurnState::Idle, now);
            self.snapshot.session = Some(metadata);
            return;
        }

        let session = self
            .snapshot
            .session
            .get_or_insert_with(AgentSessionMetadata::default);
        if metadata.session_id.is_some() {
            session.session_id = metadata.session_id;
        }
        if metadata.model.is_some() {
            session.model = metadata.model;
        }
        if metadata.title.is_some() {
            session.title = metadata.title;
        }
        if metadata.transcript_path.is_some() {
            session.transcript_path = metadata.transcript_path;
        }
    }

    fn child_started(&mut self, child: ChildAgentDescriptor, now: u64) {
        let id = bounded_text(child.id, 256);
        if id.is_empty() {
            return;
        }
        let name = child
            .name
            .map(|name| bounded_text(name, 256))
            .filter(|name| !name.is_empty());
        if let Some(existing) = self
            .snapshot
            .children
            .iter_mut()
            .find(|candidate| candidate.id == id)
        {
            existing.name = name.or_else(|| existing.name.clone());
            existing.task = child.task.or_else(|| existing.task.clone());
            existing.current_action = None;
            existing.turn_state = AgentTurnState::Working;
            existing.updated_at = now;
            return;
        }
        if self.snapshot.children.len() >= MAX_CHILD_AGENTS {
            return;
        }
        self.snapshot.children.push(ChildAgentSnapshot {
            id,
            name,
            task: child.task,
            current_action: None,
            turn_state: AgentTurnState::Working,
            started_at: now,
            updated_at: now,
        });
    }

    fn child_updated(&mut self, child_id: &str, update: ChildAgentUpdate, now: u64) {
        let child_id = bounded_text(child_id.to_string(), 256);
        let Some(child) = self
            .snapshot
            .children
            .iter_mut()
            .find(|candidate| candidate.id == child_id)
        else {
            return;
        };
        if let Some(task) = update.task {
            child.task = Some(task);
        }
        if let Some(action) = update.current_action {
            child.current_action = Some(action);
        }
        if let Some(turn_state) = update.turn_state {
            child.turn_state = turn_state;
        }
        child.updated_at = now;
    }

    fn child_finished(&mut self, child_id: &str) {
        self.snapshot
            .children
            .retain(|candidate| candidate.id != child_id);
    }

    fn clear_waiting(&mut self) {
        self.snapshot.waiting_reason = None;
        self.snapshot.waiting_message = None;
    }

    fn set_process_state(&mut self, state: AgentProcessState, now: u64) {
        if self.snapshot.process_state != state {
            self.snapshot.process_state = state;
            self.snapshot.state_started_at = now;
        }
    }

    fn set_turn_state(&mut self, state: AgentTurnState, now: u64) {
        if self.snapshot.turn_state != state {
            self.snapshot.turn_state = state;
            self.snapshot.state_started_at = now;
        }
    }
}

fn sanitize_session_metadata(metadata: AgentSessionMetadata) -> AgentSessionMetadata {
    AgentSessionMetadata {
        session_id: metadata
            .session_id
            .map(|value| bounded_text(value, 512))
            .filter(|value| !value.is_empty()),
        model: metadata
            .model
            .map(|value| bounded_text(value, 256))
            .filter(|value| !value.is_empty()),
        title: metadata
            .title
            .map(|value| bounded_text(value, 160))
            .filter(|value| !value.is_empty()),
        transcript_path: metadata
            .transcript_path
            .map(|value| bounded_text(value, 8 * 1024))
            .filter(|value| !value.is_empty()),
    }
}
