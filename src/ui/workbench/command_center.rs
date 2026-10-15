use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    fmt::Write as _,
    time::{SystemTime, UNIX_EPOCH},
};

use gpui::{
    App, Context, Entity, Focusable as _, FontWeight, IntoElement as _, Subscription, Task, Window,
    div, prelude::*, px, relative,
};
use gpui_component::{
    Disableable as _,
    input::{InputEvent, InputState},
};
use yttt_agent_core::{
    AgentBoardCommand, AgentBoardTask, AgentExecutionRecord, AgentExecutionTelemetry,
    AgentExitReason, AgentMessageRole, AgentPlanStatus, AgentProcessState, AgentQuestionAnswer,
    AgentQuestionRecord, AgentQuestionState, AgentSnapshot, AgentTurnState,
};
use yttt_client_core::{ClientEvent, ConnectionState};
use yttt_protocol::{
    Request, Response,
    agent::{AgentCommandRequest, AgentHookScope, AgentSnapshotUpdate},
};

use crate::ui::i18n::{Locale, UiText};

use super::*;

/// UI-local drafts are intentionally never represented as provider answers. They survive selection
/// changes and a temporarily unavailable Host, but only the provider board is authoritative.
#[derive(Default)]
pub(super) struct CommandCenterState {
    pub(super) previous_project: Option<ProjectId>,
    selection: Option<CommandCenterSelection>,
    question_drafts: HashMap<String, CommandCenterQuestionDraft>,
    composer: Option<Entity<InputState>>,
    composer_subscription: Option<Subscription>,
    composer_target: Option<String>,
    composer_locale: Option<Locale>,
    prompt_drafts: HashMap<String, String>,
    feedback: HashMap<String, CommandCenterFeedback>,
    live_updates_runtime: Option<std::sync::Arc<crate::host_runtime::DesktopHostRuntime>>,
    live_updates_task: Option<Task<()>>,
    live_updates_closed: bool,
    live_updates_generation: u64,
    execution_history_expanded: bool,
    project_history_expanded: BTreeSet<String>,
    project_viewport_height: f32,
    question_history_expanded: bool,
    question_scroll: gpui::ScrollHandle,
    management_session: Option<String>,
    pub(super) pending_terminal_termination: Option<String>,
}

#[derive(Clone)]
enum CommandCenterFeedback {
    Text(UiTextKey),
    Detail { key: UiTextKey, detail: String },
    QuestionNotAwaiting(AgentQuestionState),
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct CommandCenterSelection {
    terminal_session_id: String,
    execution_id: String,
    task_id: Option<String>,
    question_request_id: Option<String>,
    question_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct CommandCenterQuestionTarget {
    terminal_session_id: String,
    execution_id: String,
    request_id: String,
    question_id: String,
}

#[derive(Default)]
struct CommandCenterQuestionDraft {
    selected_options: BTreeMap<String, BTreeSet<String>>,
    custom_inputs: HashMap<String, Entity<InputState>>,
    input_subscriptions: HashMap<String, Subscription>,
    question_focuses: HashMap<String, gpui::FocusHandle>,
    input_locale: Option<Locale>,
}

#[derive(Clone)]
struct CommandCenterOperationTarget {
    host_epoch: u64,
    instance_id: yttt_agent_core::AgentInstanceId,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct CommandCenterSessionTarget {
    project_id: ProjectId,
    tab_id: String,
    pane_id: String,
}

#[derive(Clone)]
pub(super) struct CommandCenterSource {
    scope: AgentHookScope,
    session_target: Option<CommandCenterSessionTarget>,
    project_name: String,
    pub(super) terminal_session_id: String,
    snapshot: AgentSnapshot,
    operation_target: Option<CommandCenterOperationTarget>,
}

impl WorkbenchView {
    pub(super) fn command_center_view(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        self.command_center_ensure_live_updates(window, cx);
        let sources = self.command_center_sources();
        self.command_center_ensure_selection(&sources);
        let theme = self.theme_runtime().ui;
        let style = self.theme_runtime().style;
        let host_status = match self.terminal.host_runtime.as_ref() {
            Some(runtime) if matches!(runtime.state(), ConnectionState::Ready { .. }) => {
                if self.shared_mutation_allowed() {
                    CommandCenterHostStatus::Live
                } else {
                    CommandCenterHostStatus::LiveReadOnly
                }
            }
            Some(_) => CommandCenterHostStatus::DisconnectedRetained,
            None => CommandCenterHostStatus::UnavailableRetained,
        };
        let pending_questions = sources
            .iter()
            .flat_map(|source| source.snapshot.board.questions.iter())
            .filter(|record| matches!(record.state, AgentQuestionState::Pending))
            .count();
        let execution_count = sources
            .iter()
            .map(|source| source.snapshot.board.executions.len())
            .sum::<usize>();
        let plan_task_count = sources
            .iter()
            .map(|source| source.snapshot.board.tasks.len())
            .sum::<usize>();

        let mut project_sources = BTreeMap::<(String, String), Vec<&CommandCenterSource>>::new();
        for source in &sources {
            project_sources
                .entry((source.scope.project_id.clone(), source.project_name.clone()))
                .or_default()
                .push(source);
        }
        let project_rows = project_sources.len().div_ceil(3).max(2);
        // Keep six fixed slots regardless of project count. Additional rows extend the
        // scrollable content instead of shrinking the cards already visible.
        let project_row_height =
            ((self.command_center.project_viewport_height - 32.0) / 2.0).max(0.0);
        let project_matrix_height =
            project_row_height * project_rows as f32 + (project_rows - 1) as f32 * 10.0 + 22.0;
        let measure_view = cx.entity().downgrade();
        let mut board = div()
            .id("command-center-projects")
            .debug_selector(|| "command-center-project-matrix".to_string())
            .grid()
            .grid_cols(3)
            .grid_rows(project_rows as u16)
            .gap(px(10.0))
            .w_full()
            .h(px(project_matrix_height))
            .min_w_0()
            .flex_none()
            .p(px(11.0));
        if project_sources.is_empty() {
            board = board.child(
                div()
                    .w_full()
                    .h_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_sm()
                    .text_color(theme.text_muted)
                    .child(
                        div()
                            .w_full()
                            .whitespace_normal()
                            .text_center()
                            .child(self.ui_text.get(UiTextKey::CommandCenterNoRecords)),
                    ),
            );
        } else {
            for ((project_id, project_name), sources) in project_sources {
                board = board.child(self.command_center_project_card(
                    &project_id,
                    &project_name,
                    &sources,
                    theme,
                    window,
                    cx,
                ));
            }
        }

        let lower = div()
            .flex()
            .min_w_0()
            .min_h_0()
            .flex_basis(relative(0.37))
            .flex_shrink(1.0)
            .gap(px(11.0))
            .px(px(11.0))
            .pb(px(8.0))
            .child(self.command_center_questions_panel(&sources, theme, style, window, cx))
            .child(self.command_center_conversation_panel(&sources, theme, style, window, cx))
            .child(self.command_center_executions_panel(&sources, theme, window, cx));

        div()
            .debug_selector(|| "command-center-surface".to_string())
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .overflow_hidden()
            .bg(theme.app_background)
            .child(
                div()
                    .flex_none()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .min_h(px(28.0))
                    .py(px(3.0))
                    .px(px(11.0))
                    .gap_x(px(8.0))
                    .gap_y(px(1.0))
                    .border_b(px(1.0))
                    .border_color(theme.border)
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(self.ui_text.get(UiTextKey::CommandCenterTitle)),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(host_status.color(theme))
                            .child(self.ui_text.get(host_status.key())),
                    )
                    .child(div().flex_1())
                    .child(div().text_xs().text_color(theme.text_muted).child(
                        command_center_count(
                            self.ui_text,
                            UiTextKey::CommandCenterProjectsCount,
                            sources_project_count(&sources),
                        ),
                    ))
                    .child(div().text_xs().text_color(theme.text_muted).child(
                        command_center_count(
                            self.ui_text,
                            UiTextKey::CommandCenterTasksCount,
                            plan_task_count,
                        ),
                    ))
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.warning)
                            .child(command_center_count(
                                self.ui_text,
                                UiTextKey::CommandCenterQuestionsCount,
                                pending_questions,
                            )),
                    )
                    .child(div().text_xs().text_color(theme.text_muted).child(
                        command_center_count(
                            self.ui_text,
                            UiTextKey::CommandCenterExecutionsCount,
                            execution_count,
                        ),
                    )),
            )
            .child(
                div()
                    .id("command-center-project-viewport")
                    .debug_selector(|| "command-center-project-viewport".to_string())
                    .relative()
                    .flex()
                    .min_w_0()
                    .min_h_0()
                    .overflow_y_scroll()
                    .flex_basis(relative(0.63))
                    .flex_shrink(1.0)
                    .child(board)
                    .child(
                        gpui::canvas(
                            move |bounds, _, cx| {
                                let height = f32::from(bounds.size.height);
                                if measure_view
                                    .read_with(cx, |root, _| {
                                        (root.command_center.project_viewport_height - height).abs()
                                            > 1.0
                                    })
                                    .unwrap_or(false)
                                {
                                    let view = measure_view.clone();
                                    cx.defer(move |cx| {
                                        let _ = view.update(cx, |root, cx| {
                                            root.command_center.project_viewport_height = height;
                                            cx.notify();
                                        });
                                    });
                                }
                            },
                            |_, _, _, _| {},
                        )
                        .absolute()
                        .size_full(),
                    ),
            )
            .child(lower)
    }

    pub(super) fn command_center_stop_live_updates(&mut self) {
        self.command_center.live_updates_generation =
            self.command_center.live_updates_generation.wrapping_add(1);
        self.command_center.live_updates_task = None;
        self.command_center.live_updates_runtime = None;
        self.command_center.live_updates_closed = false;
    }

    fn command_center_ensure_live_updates(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(runtime) = self.terminal.host_runtime.clone() else {
            self.command_center_stop_live_updates();
            return;
        };
        let has_current_subscription = self
            .command_center
            .live_updates_runtime
            .as_ref()
            .is_some_and(|current| std::sync::Arc::ptr_eq(current, &runtime))
            && (self.command_center.live_updates_task.is_some()
                || self.command_center.live_updates_closed);
        if has_current_subscription {
            return;
        }
        self.command_center_stop_live_updates();
        let generation = self.command_center.live_updates_generation;
        let events = runtime.events();
        self.command_center.live_updates_runtime = Some(runtime);
        self.command_center.live_updates_closed = false;
        self.command_center.live_updates_task = Some(cx.spawn_in(window, async move |this, cx| {
            while let Ok(event) = events.recv_async().await {
                let mut refresh = command_center_event_requires_refresh(&event);
                for queued in events.try_iter().take(128) {
                    refresh |= command_center_event_requires_refresh(&queued);
                }
                if this
                    .update_in(cx, |root, _window, cx| {
                        if refresh
                            && root.command_center_is_active()
                            && root.command_center.live_updates_generation == generation
                        {
                            cx.notify();
                        }
                    })
                    .is_err()
                {
                    return;
                }
                // Always yield: a busy Host queue must not starve native input or desktop control.
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(100))
                    .await;
            }
            let _ = this.update_in(cx, |root, _window, cx| {
                if root.command_center.live_updates_generation == generation {
                    root.command_center.live_updates_task = None;
                    root.command_center.live_updates_closed = true;
                    if root.command_center_is_active() {
                        cx.notify();
                    }
                }
            });
        }));
    }

    pub(super) fn command_center_sources(&self) -> Vec<CommandCenterSource> {
        let updates = self
            .terminal
            .host_runtime
            .as_ref()
            .map(|runtime| runtime.agent_snapshots())
            .unwrap_or_default();
        self.command_center_sources_with_updates(updates)
    }

    fn command_center_sources_with_updates(
        &self,
        updates: Vec<AgentSnapshotUpdate>,
    ) -> Vec<CommandCenterSource> {
        // Host hook scope IDs are opaque terminal IDs, not desktop tab/pane IDs.
        // Join by terminal session ID; never split IDs to guess layout coordinates.
        let mut placements = BTreeMap::new();
        let mut project_names = HashMap::new();
        for project in self.workspace.opened_projects() {
            project_names.insert(project.id.as_str(), project.layout.project.name.as_str());
            for tab in &project.layout.tabs {
                let Some(tab_state) = project.tab_state(&tab.id) else {
                    continue;
                };
                for pane in &tab_state.pane_states {
                    let session_id = terminal_pane_key(project.id.as_str(), &tab.id, &pane.pane_id);
                    placements.insert(session_id, (project, &tab.id, pane));
                }
            }
        }

        let mut sources = Vec::new();
        for update in updates {
            let session_id = update.terminal_session_id.to_string();
            let (session_target, project_name, previous) = match placements.remove(&session_id) {
                Some((project, tab_id, pane)) => (
                    Some(CommandCenterSessionTarget {
                        project_id: project.id.clone(),
                        tab_id: tab_id.clone(),
                        pane_id: pane.pane_id.clone(),
                    }),
                    project.layout.project.name.clone(),
                    pane.agent_snapshot.as_ref(),
                ),
                None => (
                    None,
                    project_names
                        .get(update.scope.project_id.as_str())
                        .map(|name| (*name).to_string())
                        .unwrap_or_else(|| update.scope.project_id.clone()),
                    None,
                ),
            };
            let mut snapshot = update.snapshot;
            if let Some(previous) = previous {
                snapshot.board.merge_retained(&previous.board);
            }
            let instance_id = snapshot.instance_id.clone();
            sources.push(CommandCenterSource {
                scope: update.scope,
                session_target,
                project_name,
                terminal_session_id: session_id,
                snapshot,
                operation_target: Some(CommandCenterOperationTarget {
                    host_epoch: update.host_epoch,
                    instance_id,
                }),
            });
        }
        for (terminal_session_id, (project, tab_id, pane)) in placements {
            let Some(snapshot) = &pane.agent_snapshot else {
                continue;
            };
            let target = CommandCenterSessionTarget {
                project_id: project.id.clone(),
                tab_id: tab_id.clone(),
                pane_id: pane.pane_id.clone(),
            };
            sources.push(CommandCenterSource {
                scope: AgentHookScope {
                    project_id: target.project_id.to_string(),
                    tab_id: target.tab_id.clone(),
                    pane_id: target.pane_id.clone(),
                    generation: snapshot.generation,
                },
                session_target: Some(target),
                project_name: project.layout.project.name.clone(),
                terminal_session_id,
                snapshot: snapshot.clone(),
                operation_target: None,
            });
        }
        sources.sort_by(|left, right| {
            (
                left.project_name.as_str(),
                left.scope.project_id.as_str(),
                left.terminal_session_id.as_str(),
            )
                .cmp(&(
                    right.project_name.as_str(),
                    right.scope.project_id.as_str(),
                    right.terminal_session_id.as_str(),
                ))
        });
        sources
    }

    fn command_center_ensure_selection(&mut self, sources: &[CommandCenterSource]) {
        let selection_is_current =
            self.command_center
                .selection
                .as_ref()
                .is_some_and(|selection| {
                    sources.iter().any(|source| {
                        if source.terminal_session_id != selection.terminal_session_id {
                            return false;
                        }
                        if let Some(request_id) = selection.question_request_id.as_deref() {
                            return source.snapshot.board.questions.iter().any(|record| {
                                record.request.id == request_id
                                    && record.request.execution_id == selection.execution_id
                                    && selection.question_id.as_deref().is_none_or(|question_id| {
                                        record
                                            .request
                                            .questions
                                            .iter()
                                            .any(|question| question.id == question_id)
                                    })
                            });
                        }
                        source
                            .snapshot
                            .board
                            .executions
                            .iter()
                            .any(|execution| execution.id == selection.execution_id)
                            || source.snapshot.board.tasks.iter().any(|task| {
                                task.execution_id == selection.execution_id
                                    && selection.task_id.as_deref() == Some(task.item.id.as_str())
                            })
                            || source.snapshot.board.questions.iter().any(|question| {
                                question.request.execution_id == selection.execution_id
                            })
                            || (selection.execution_id == source.terminal_session_id
                                && source.snapshot.board.executions.is_empty())
                    })
                });
        if selection_is_current {
            return;
        }
        for source in sources {
            if let Some((record, question)) = source
                .snapshot
                .board
                .questions
                .iter()
                .filter(|record| record.state == AgentQuestionState::Pending)
                .find_map(|record| {
                    record
                        .request
                        .questions
                        .first()
                        .map(|question| (record, question))
                })
            {
                self.command_center.selection = Some(CommandCenterSelection {
                    terminal_session_id: source.terminal_session_id.clone(),
                    execution_id: record.request.execution_id.clone(),
                    task_id: None,
                    question_request_id: Some(record.request.id.clone()),
                    question_id: Some(question.id.clone()),
                });
                return;
            }
        }
        self.command_center.selection = sources.first().map(|source| {
            let task = source
                .snapshot
                .board
                .tasks
                .iter()
                .find(|task| !command_center_task_is_history(task))
                .or_else(|| source.snapshot.board.tasks.first());
            CommandCenterSelection {
                terminal_session_id: source.terminal_session_id.clone(),
                execution_id: task
                    .map(|task| task.execution_id.clone())
                    .or_else(|| {
                        source
                            .snapshot
                            .board
                            .executions
                            .first()
                            .map(|execution| execution.id.clone())
                    })
                    .unwrap_or_else(|| source.terminal_session_id.clone()),
                task_id: task.map(|task| task.item.id.clone()),
                question_request_id: None,
                question_id: None,
            }
        });
    }

    fn command_center_session_label(&self, source: &CommandCenterSource) -> String {
        format!(
            "{} · {}",
            command_center_agent_label(&source.snapshot),
            self.command_center_session_location(source),
        )
    }

    fn command_center_session_location(&self, source: &CommandCenterSource) -> String {
        source
            .session_target
            .as_ref()
            .and_then(|target| {
                let project = self.workspace.project(&target.project_id)?;
                let tab = project.layout.tab(&target.tab_id)?;
                let pane = tab.layout.find_pane(&target.pane_id)?;
                Some(if tab.title == pane.title {
                    tab.title.clone()
                } else {
                    format!("{} · {}", tab.title, pane.title)
                })
            })
            .unwrap_or_else(|| {
                self.ui_text
                    .get(UiTextKey::CommandCenterSessionNotOpen)
                    .to_string()
            })
    }

    fn command_center_project_card(
        &mut self,
        project_id: &str,
        project_name: &str,
        sources: &[&CommandCenterSource],
        theme: WorkbenchTheme,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let host_live = self
            .terminal
            .host_runtime
            .as_ref()
            .is_some_and(|runtime| matches!(runtime.state(), ConnectionState::Ready { .. }));
        let (online_agents, active_children) = sources
            .iter()
            .filter(|source| {
                host_live
                    && source.operation_target.is_some()
                    && source.snapshot.process_state == AgentProcessState::Running
            })
            .fold((0, 0), |(roots, children), source| {
                let active = source
                    .snapshot
                    .children
                    .iter()
                    .filter(|child| {
                        matches!(
                            child.turn_state,
                            AgentTurnState::Working | AgentTurnState::Waiting
                        )
                    })
                    .count();
                (roots + 1, children + active)
            });
        let pending_questions = sources
            .iter()
            .flat_map(|source| source.snapshot.board.questions.iter())
            .filter(|record| matches!(record.state, AgentQuestionState::Pending))
            .count();
        let mut task_status_counts = [
            (AgentPlanStatus::Pending, 0usize),
            (AgentPlanStatus::InProgress, 0usize),
            (AgentPlanStatus::Completed, 0usize),
            (AgentPlanStatus::Blocked, 0usize),
            (AgentPlanStatus::Abandoned, 0usize),
            (AgentPlanStatus::Unknown, 0usize),
        ];
        let mut task_count = 0;
        let mut history_count = 0;
        for task in sources
            .iter()
            .flat_map(|source| source.snapshot.board.tasks.iter())
        {
            task_count += 1;
            if command_center_task_is_history(task) {
                history_count += 1;
            }
            let status_index = match task.item.status {
                AgentPlanStatus::Pending => 0,
                AgentPlanStatus::InProgress => 1,
                AgentPlanStatus::Completed => 2,
                AgentPlanStatus::Blocked => 3,
                AgentPlanStatus::Abandoned => 4,
                AgentPlanStatus::Unknown => 5,
            };
            task_status_counts[status_index].1 += 1;
        }
        let source_usage: Vec<_> = sources
            .iter()
            .map(|source| CommandCenterProjectUsage::new(self.ui_text, source))
            .collect();
        history_count += source_usage
            .iter()
            .flat_map(|usage| &usage.independent)
            .filter(|execution| command_center_execution_priority(execution) == 3)
            .count();
        history_count += sources
            .iter()
            .zip(&source_usage)
            .filter(|(source, usage)| {
                source.snapshot.process_state == AgentProcessState::Exited
                    && source.snapshot.board.tasks.is_empty()
                    && usage.independent.is_empty()
            })
            .count();
        let selected_project = self
            .command_center
            .selection
            .as_ref()
            .is_some_and(|selection| {
                sources
                    .iter()
                    .any(|source| source.terminal_session_id == selection.terminal_session_id)
            });
        let history_expanded = self
            .command_center
            .project_history_expanded
            .contains(project_id);
        let history_project_id = project_id.to_string();
        let mut summary = div()
            .flex_none()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_x(px(6.0))
            .gap_y(px(2.0))
            .mb(px(5.0))
            .text_xs()
            .text_color(theme.text_muted)
            .child(div().child(command_center_count(
                self.ui_text,
                UiTextKey::CommandCenterOnlineAgentsCount,
                online_agents,
            )))
            .child(
                div()
                    .text_color(if pending_questions > 0 {
                        theme.warning
                    } else {
                        theme.text_muted
                    })
                    .child(command_center_count(
                        self.ui_text,
                        UiTextKey::CommandCenterPendingQuestionsCount,
                        pending_questions,
                    )),
            );
        if active_children > 0 {
            summary = summary.child(div().child(command_center_count(
                self.ui_text,
                UiTextKey::CommandCenterActiveChildrenCount,
                active_children,
            )));
        }
        if task_count == 0 {
            summary = summary
                .child(div().child(self.ui_text.get(UiTextKey::CommandCenterNoReportedTasks)));
        } else {
            for (status, count) in task_status_counts {
                if count == 0 {
                    continue;
                }
                summary = summary.child(div().child(format!(
                    "{count} {}",
                    self.ui_text.get(command_center_plan_label(status))
                )));
            }
        }

        let mut task_list = div()
            .id(format!("command-center-tasks:{project_id}"))
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .overflow_y_scroll();
        for priority in 0..4 {
            let history = priority == 3;
            if history && history_count > 0 {
                let history_project_id = history_project_id.clone();
                let history_debug_project_id = project_id.to_string();
                task_list = task_list.child(
                    div()
                        .id(format!("command-center-task-history:{project_id}"))
                        .debug_selector(move || {
                            format!("command-center-project-history:{history_debug_project_id}")
                        })
                        .cursor_pointer()
                        .flex_none()
                        .py(px(5.0))
                        .px(px(5.0))
                        .text_xs()
                        .text_color(theme.text_muted)
                        .child(command_center_count(
                            self.ui_text,
                            if history_expanded {
                                UiTextKey::CommandCenterTaskHistoryHide
                            } else {
                                UiTextKey::CommandCenterTaskHistoryShow
                            },
                            history_count,
                        ))
                        .on_click(cx.listener(move |root, _, _, cx| {
                            if !root
                                .command_center
                                .project_history_expanded
                                .insert(history_project_id.clone())
                            {
                                root.command_center
                                    .project_history_expanded
                                    .remove(&history_project_id);
                            }
                            cx.notify();
                        })),
                );
                if !history_expanded {
                    continue;
                }
            }
            for (source, usage) in sources.iter().zip(&source_usage) {
                let mut visible_tasks = source
                    .snapshot
                    .board
                    .tasks
                    .iter()
                    .filter(|task| command_center_task_priority(task) == priority)
                    .peekable();
                let has_visible_tasks = visible_tasks.peek().is_some();
                let mut visible_executions = usage
                    .independent
                    .iter()
                    .copied()
                    .filter(|execution| command_center_execution_priority(execution) == priority)
                    .peekable();
                let has_visible_executions = visible_executions.peek().is_some();
                if !has_visible_tasks
                    && !has_visible_executions
                    && command_center_session_summary_priority(source, usage) != Some(priority)
                {
                    continue;
                }
                let selection_execution_id = visible_tasks
                    .peek()
                    .map(|task| task.execution_id.clone())
                    .or_else(|| {
                        visible_executions
                            .peek()
                            .map(|execution| execution.id.clone())
                    })
                    .or_else(|| {
                        source
                            .snapshot
                            .board
                            .executions
                            .iter()
                            .filter(|execution| execution.parent_id.is_none())
                            .max_by_key(|execution| {
                                (execution.ended_at.is_none(), execution.updated_at)
                            })
                            .or_else(|| source.snapshot.board.executions.last())
                            .map(|execution| execution.id.clone())
                    })
                    .unwrap_or_else(|| source.terminal_session_id.clone());
                let selection_task_id = visible_tasks.peek().map(|task| task.item.id.clone());
                let session_id = source.terminal_session_id.clone();
                let agent_label = command_center_agent_label(&source.snapshot);
                let session_location = self.command_center_session_location(source);
                let (status_label, status_state) = command_center_agent_status(
                    &source.snapshot,
                    host_live && source.operation_target.is_some(),
                );
                let show_status = !history || (!has_visible_tasks && !has_visible_executions);
                let session_selected =
                    self.command_center
                        .selection
                        .as_ref()
                        .is_some_and(|selection| {
                            selection.terminal_session_id == session_id
                                && selection.task_id.is_none()
                        });
                let session_row = div()
                    .id(format!(
                        "command-center-session:{}:{priority}",
                        source.terminal_session_id
                    ))
                    .debug_selector(|| {
                        format!(
                            "command-center-session:{}:{priority}",
                            source.terminal_session_id
                        )
                    })
                    .cursor_pointer()
                    .flex_none()
                    .flex()
                    .items_center()
                    .min_h(px(24.0))
                    .px(px(5.0))
                    .gap(px(6.0))
                    .border_l(px(2.0))
                    .border_color(if session_selected {
                        theme.accent
                    } else {
                        theme.app_background
                    })
                    .bg(if session_selected {
                        theme.selection
                    } else {
                        theme.ghost_element_background
                    })
                    .on_click(cx.listener(move |root, _, _window, cx| {
                        root.command_center_select(
                            session_id.clone(),
                            selection_execution_id.clone(),
                            selection_task_id.clone(),
                            cx,
                        );
                    }))
                    .child(
                        div()
                            .w(px(12.0))
                            .text_xs()
                            .text_color(if show_status {
                                command_center_turn_color(status_state, theme)
                            } else {
                                theme.text_subtle
                            })
                            .child(if show_status {
                                command_center_turn_symbol(status_state)
                            } else {
                                ""
                            }),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .truncate()
                                    .text_xs()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(agent_label),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .text_xs()
                                    .text_color(theme.text_subtle)
                                    .child(session_location),
                            ),
                    )
                    .when(show_status, |row| {
                        row.child(
                            div()
                                .flex_none()
                                .text_xs()
                                .text_color(command_center_turn_color(status_state, theme))
                                .child(self.ui_text.get(status_label)),
                        )
                    });
                let mut group = div()
                    .flex()
                    .flex_col()
                    .flex_none()
                    .min_w_0()
                    .child(session_row);
                for task in visible_tasks {
                    group = group.child(self.command_center_task_row(
                        source,
                        task,
                        &usage.task_labels[task.execution_id.as_str()],
                        theme,
                        cx,
                    ));
                }
                if has_visible_executions {
                    group = group.child(
                        div()
                            .px(px(5.0))
                            .text_xs()
                            .text_color(theme.text_subtle)
                            .child(self.ui_text.get(UiTextKey::CommandCenterExecutionUsage)),
                    );
                }
                for execution in visible_executions {
                    group = group.child(
                        div()
                            .debug_selector(|| {
                                format!(
                                    "command-center-project-usage:{}:{}",
                                    source.terminal_session_id, execution.id
                                )
                            })
                            .child(self.command_center_execution_row(
                                source,
                                Some(execution),
                                execution.id.clone(),
                                &execution.name,
                                execution.state,
                                execution.started_at,
                                execution.ended_at,
                                theme,
                                cx,
                            )),
                    );
                }
                task_list = task_list.child(group);
            }
        }

        let debug_project_id = project_id.to_string();
        let card = div()
            .debug_selector(move || format!("command-center-project-card:{debug_project_id}"))
            .flex()
            .flex_col()
            .min_w_0()
            .min_h_0()
            .h_full()
            .p(px(7.0))
            .border(px(1.0))
            .border_color(if selected_project {
                theme.accent
            } else {
                theme.border
            })
            .bg(theme.surface);
        card.child(
            div()
                .flex_none()
                .flex()
                .items_center()
                .gap(px(6.0))
                .mb(px(4.0))
                .child(
                    div()
                        .min_w_0()
                        .flex_1()
                        .truncate()
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(project_name.to_string()),
                ),
        )
        .child(summary)
        .child(task_list)
    }

    fn command_center_task_row(
        &self,
        source: &CommandCenterSource,
        task: &AgentBoardTask,
        usage_label: &SharedString,
        theme: WorkbenchTheme,
        cx: &Context<Self>,
    ) -> gpui::Stateful<Div> {
        let session_id = source.terminal_session_id.clone();
        let execution_id = task.execution_id.clone();
        let task_id = task.item.id.clone();
        let selected = self
            .command_center
            .selection
            .as_ref()
            .is_some_and(|selection| {
                selection.terminal_session_id == session_id
                    && selection.execution_id == execution_id
                    && selection.task_id.as_deref() == Some(task_id.as_str())
            });
        let history = command_center_task_is_history(task);
        div()
            .id(format!(
                "command-center-task:{session_id}:{execution_id}:{task_id}"
            ))
            .debug_selector(|| format!("command-center-task:{session_id}:{execution_id}:{task_id}"))
            .cursor_pointer()
            .flex()
            .items_center()
            .min_h(px(24.0))
            .px(px(5.0))
            .gap(px(5.0))
            .border_l(px(2.0))
            .border_color(if selected {
                theme.accent
            } else {
                theme.app_background
            })
            .bg(if selected {
                theme.selection
            } else {
                theme.ghost_element_background
            })
            .opacity(if history { 0.58 } else { 1.0 })
            .on_click(cx.listener(move |root, _, _window, cx| {
                root.command_center_select(
                    session_id.clone(),
                    execution_id.clone(),
                    Some(task_id.clone()),
                    cx,
                );
            }))
            .child(
                div()
                    .w(px(12.0))
                    .text_xs()
                    .text_color(command_center_plan_color(task.item.status, theme))
                    .child(command_center_plan_symbol(task.item.status)),
            )
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .truncate()
                    .text_xs()
                    .child(task.item.title.clone()),
            )
            .child(
                div()
                    .id("task-usage")
                    .flex_none()
                    .text_xs()
                    .text_color(theme.text_subtle)
                    .child(usage_label.clone())
                    .tooltip(self.command_center_usage_tooltip(
                        source,
                        &task.execution_id,
                        true,
                        cx,
                    )),
            )
            .child(
                div().text_xs().text_color(theme.text_subtle).child(
                    self.ui_text
                        .get(command_center_plan_label(task.item.status)),
                ),
            )
            .when(task.accepted_at.is_some(), |row| {
                row.child(
                    div()
                        .text_xs()
                        .text_color(theme.text_subtle)
                        .child(self.ui_text.get(UiTextKey::CommandCenterTaskAccepted)),
                )
            })
            .when(task.retired, |row| {
                row.child(
                    div()
                        .text_xs()
                        .text_color(theme.text_subtle)
                        .child(self.ui_text.get(UiTextKey::CommandCenterTaskRetired)),
                )
            })
    }

    fn command_center_usage_tooltip(
        &self,
        source: &CommandCenterSource,
        execution_id: &str,
        task_hint: bool,
        cx: &Context<Self>,
    ) -> impl Fn(&mut Window, &mut App) -> gpui::AnyView + 'static {
        let weak = cx.entity().downgrade();
        let session_id = source.terminal_session_id.clone();
        let execution_id = execution_id.to_string();
        let ui_text = self.ui_text;
        move |window, cx| {
            // Resolve only on hover, using current observations rather than formatting and
            // duplicating every execution's details for every task on each live repaint.
            let details = weak
                .upgrade()
                .and_then(|root| {
                    let root = root.read(cx);
                    let sources = root.command_center_sources();
                    let source = sources
                        .iter()
                        .find(|source| source.terminal_session_id == session_id)?;
                    let execution = source
                        .snapshot
                        .board
                        .executions
                        .iter()
                        .find(|execution| execution.id == execution_id)?;
                    let details = command_center_execution_details(root.ui_text, source, execution);
                    Some(if task_hint {
                        format!(
                            "{}\n{details}",
                            root.ui_text.get(UiTextKey::CommandCenterTaskUsageHint)
                        )
                    } else {
                        details
                    })
                })
                .unwrap_or_else(|| {
                    ui_text
                        .get(UiTextKey::CommandCenterTaskUsageUnavailable)
                        .to_string()
                });
            yttt_ui::primitives::tooltip::yttt_tooltip(
                details,
                crate::ui::theme::current_workbench_theme(cx),
                crate::ui::theme::current_ui_style(cx),
            )
            .build(window, cx)
        }
    }

    fn command_center_questions_panel(
        &mut self,
        sources: &[CommandCenterSource],
        theme: WorkbenchTheme,
        style: UiStyle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let all_records = || {
            sources.iter().flat_map(|source| {
                source
                    .snapshot
                    .board
                    .questions
                    .iter()
                    .map(move |record| (source, record))
            })
        };
        let question_count = all_records()
            .filter(|(_, record)| !command_center_question_is_resolved(record.state))
            .count();
        let history_count = all_records()
            .filter(|(_, record)| command_center_question_is_resolved(record.state))
            .count();
        let has_pending =
            all_records().any(|(_, record)| record.state == AgentQuestionState::Pending);
        let question_scroll = self.command_center.question_scroll.clone();
        let mut records = div()
            .id("command-center-questions")
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&question_scroll);
        if question_count == 0 {
            records = records.child(
                div()
                    .flex_none()
                    .p(px(7.0))
                    .text_xs()
                    .text_color(theme.text_muted)
                    .child(self.ui_text.get(UiTextKey::CommandCenterNoQuestions)),
            );
        }
        // Keep the selected result in its queue position after settlement. Only an explicit
        // selection change lets it join collapsed history; incoming updates never select a row.
        for unavailable in [false, true] {
            for (source, record) in all_records() {
                if unavailable != (record.state == AgentQuestionState::Unavailable) {
                    continue;
                }
                let selected = self
                    .command_center
                    .selection
                    .as_ref()
                    .is_some_and(|selection| {
                        selection.terminal_session_id == source.terminal_session_id
                            && selection.question_request_id.as_deref()
                                == Some(record.request.id.as_str())
                    });
                if command_center_question_is_resolved(record.state) && !selected {
                    continue;
                }
                records = records.child(
                    self.command_center_question_card(source, record, theme, style, window, cx),
                );
            }
        }
        if history_count > 0 {
            records = records.child(
                div().flex_none().p(px(5.0)).child(
                    yttt_button(
                        "command-center-question-history",
                        self.ui_text
                            .get(if self.command_center.question_history_expanded {
                                UiTextKey::CommandCenterHideResolvedQuestions
                            } else {
                                UiTextKey::CommandCenterShowResolvedQuestions
                            }),
                        YtttButtonVariant::Secondary,
                        theme,
                        style,
                        cx,
                    )
                    .on_click(cx.listener(|root, _, _, cx| {
                        root.command_center.question_history_expanded =
                            !root.command_center.question_history_expanded;
                        cx.notify();
                    })),
                ),
            );
            if self.command_center.question_history_expanded {
                for (source, record) in all_records() {
                    if !command_center_question_is_resolved(record.state) {
                        continue;
                    }
                    let selected =
                        self.command_center
                            .selection
                            .as_ref()
                            .is_some_and(|selection| {
                                selection.terminal_session_id == source.terminal_session_id
                                    && selection.question_request_id.as_deref()
                                        == Some(record.request.id.as_str())
                            });
                    if !selected {
                        records = records.child(self.command_center_question_card(
                            source, record, theme, style, window, cx,
                        ));
                    }
                }
            }
        }
        command_center_panel(
            self.ui_text.get(UiTextKey::CommandCenterQuestions),
            command_center_count(
                self.ui_text,
                UiTextKey::CommandCenterRecordsCount,
                question_count,
            ),
            theme,
        )
        .flex_basis(relative(1.38 / 3.28))
        .child(
            div()
                .flex_none()
                .flex()
                .items_center()
                .gap(px(4.0))
                .px(px(5.0))
                .py(px(3.0))
                .border_b(px(1.0))
                .border_color(theme.border_variant)
                .child(
                    yttt_button(
                        "command-center-question-previous",
                        self.ui_text.get(UiTextKey::CommandCenterPreviousQuestion),
                        YtttButtonVariant::Secondary,
                        theme,
                        style,
                        cx,
                    )
                    .disabled(!has_pending)
                    .tooltip(
                        self.ui_text
                            .get(UiTextKey::CommandCenterQuestionShortcutsHint),
                    )
                    .on_click(cx.listener(|root, _, window, cx| {
                        root.command_center_move_selected_question(true, window, cx);
                    })),
                )
                .child(
                    yttt_button(
                        "command-center-question-next",
                        self.ui_text.get(UiTextKey::CommandCenterNextQuestion),
                        YtttButtonVariant::Secondary,
                        theme,
                        style,
                        cx,
                    )
                    .disabled(!has_pending)
                    .tooltip(
                        self.ui_text
                            .get(UiTextKey::CommandCenterQuestionShortcutsHint),
                    )
                    .on_click(cx.listener(|root, _, window, cx| {
                        root.command_center_move_selected_question(false, window, cx);
                    })),
                ),
        )
        .child(records)
    }

    fn command_center_question_card(
        &mut self,
        source: &CommandCenterSource,
        record: &AgentQuestionRecord,
        theme: WorkbenchTheme,
        style: UiStyle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let request_id = record.request.id.clone();
        let session_id = source.terminal_session_id.clone();
        let execution_id = record.request.execution_id.clone();
        let draft_key = command_center_question_key(&session_id, &request_id);
        let record_question_id = record
            .request
            .questions
            .first()
            .map(|question| question.id.clone());
        let record_target = CommandCenterQuestionTarget {
            terminal_session_id: session_id.clone(),
            execution_id: execution_id.clone(),
            request_id: request_id.clone(),
            question_id: record_question_id.clone().unwrap_or_default(),
        };
        let selected = self
            .command_center
            .selection
            .as_ref()
            .is_some_and(|selection| {
                selection.terminal_session_id == session_id
                    && selection.question_request_id.as_deref() == Some(request_id.as_str())
            });
        let capability = self.command_center_question_capability(source, record);
        let form_feedback = self.command_center.feedback.get(&draft_key).cloned();
        let mut card = div()
            .flex_none()
            .flex()
            .flex_col()
            .gap(px(4.0))
            .p(px(7.0))
            .border_b(px(1.0))
            .border_color(theme.border_variant)
            .bg(if selected {
                theme.selection
            } else {
                theme.ghost_element_background
            })
            .opacity(if command_center_question_is_resolved(record.state) {
                0.62
            } else {
                1.0
            })
            .child(
                div()
                    .id(format!("command-center-question:{draft_key}"))
                    .cursor_pointer()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .on_click(cx.listener(move |root, _, _window, cx| {
                        root.command_center_select_question(
                            record_target.clone(),
                            record_question_id.as_deref(),
                            cx,
                        );
                    }))
                    .child(
                        div()
                            .text_xs()
                            .text_color(command_center_question_color(record.state, theme))
                            .child(
                                self.ui_text
                                    .get(command_center_question_label(record.state)),
                            ),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .flex_1()
                            .truncate()
                            .text_xs()
                            .text_color(theme.text_muted)
                            .child(format!(
                                "{} · {}",
                                source.project_name, source.snapshot.provider_id
                            )),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.text_subtle)
                            .child(command_center_age(self.ui_text, record.updated_at)),
                    ),
            );
        for question in &record.request.questions {
            let question_id = question.id.clone();
            let question_target = CommandCenterQuestionTarget {
                terminal_session_id: session_id.clone(),
                execution_id: execution_id.clone(),
                request_id: request_id.clone(),
                question_id: question_id.clone(),
            };
            let question_selected =
                self.command_center
                    .selection
                    .as_ref()
                    .is_some_and(|selection| {
                        selection.terminal_session_id == session_id
                            && selection.question_request_id.as_deref() == Some(request_id.as_str())
                            && selection.question_id.as_deref() == Some(question_id.as_str())
                    });
            let selected_options = self
                .command_center
                .question_drafts
                .get(&draft_key)
                .and_then(|draft| draft.selected_options.get(&question_id))
                .cloned()
                .unwrap_or_default();
            let question_focus = self.command_center_question_focus(&draft_key, &question_id, cx);
            let custom_input = self.command_center_question_input(
                &draft_key,
                &question_id,
                question_target.clone(),
                window,
                cx,
            );
            let title_target = question_target.clone();
            let title_focus = question_focus.clone();
            let keyboard_target = question_target.clone();
            let mut question_body = div()
                .flex()
                .flex_col()
                .gap(px(3.0))
                .p(px(4.0))
                .border_l(px(2.0))
                .border_color(if question_selected {
                    theme.accent
                } else {
                    theme.ghost_element_background
                })
                .bg(if question_selected {
                    theme.selection
                } else {
                    theme.ghost_element_background
                })
                .child(
                    div()
                        .id(format!(
                            "command-center-question-title:{draft_key}:{question_id}"
                        ))
                        .track_focus(&question_focus)
                        .cursor_pointer()
                        .flex()
                        .flex_col()
                        .gap(px(2.0))
                        .on_click(cx.listener(move |root, _, window, cx| {
                            root.command_center_select_question(
                                title_target.clone(),
                                Some(&title_target.question_id),
                                cx,
                            );
                            title_focus.focus(window, cx);
                        }))
                        .on_key_down(cx.listener(
                            move |root, event: &gpui::KeyDownEvent, _window, cx| {
                                let modifiers = event.keystroke.modifiers;
                                if modifiers.control
                                    || modifiers.platform
                                    || modifiers.alt
                                    || modifiers.function
                                    || modifiers.shift
                                {
                                    return;
                                }
                                let Some(index) = event
                                    .keystroke
                                    .key
                                    .parse::<usize>()
                                    .ok()
                                    .and_then(|number| number.checked_sub(1))
                                else {
                                    return;
                                };
                                if root.command_center_select_option_by_index(
                                    &keyboard_target,
                                    index,
                                    cx,
                                ) {
                                    cx.stop_propagation();
                                }
                            },
                        ))
                        .child(
                            div()
                                .text_xs()
                                .text_color(theme.text_muted)
                                .children(question.header.iter().cloned()),
                        )
                        .child(div().text_sm().child(question.question.clone())),
                );
            let multi = question.multi;
            let preview_label = self.ui_text.get(UiTextKey::CommandCenterOptionPreview);
            let mut options = div().flex().flex_wrap().gap(px(4.0));
            for (option_index, option) in question.options.iter().enumerate() {
                let option_label = option.label.clone();
                let option_question_id = question.id.clone();
                let option_draft_key = draft_key.clone();
                let option_target = question_target.clone();
                let option_focus = question_focus.clone();
                let selected_option = selected_options.contains(&option_label);
                options = options.child(
                    yttt_button(
                        format!(
                            "command-center-option:{draft_key}:{}:{option_index}",
                            question.id
                        ),
                        option.label.clone(),
                        if selected_option {
                            YtttButtonVariant::Primary
                        } else {
                            YtttButtonVariant::Secondary
                        },
                        theme,
                        style,
                        cx,
                    )
                    .disabled(!capability.can_answer)
                    .on_click(cx.listener(move |root, _, window, cx| {
                        root.command_center_select_option(
                            option_target.clone(),
                            &option_draft_key,
                            &option_question_id,
                            option_label.clone(),
                            multi,
                            cx,
                        );
                        option_focus.focus(window, cx);
                    })),
                );
            }
            question_body = question_body
                .child(options)
                .child(div().text_xs().text_color(theme.text_subtle).children(
                    question.options.iter().filter_map(|option| {
                        let detail =
                            match (option.description.as_deref(), option.preview.as_deref()) {
                                (Some(description), Some(preview)) => {
                                    format!("{description} · {preview_label}: {preview}")
                                }
                                (Some(description), None) => description.to_string(),
                                (None, Some(preview)) => format!("{preview_label}: {preview}"),
                                (None, None) => return None,
                            };
                        Some(
                            div()
                                .truncate()
                                .child(format!("{} — {detail}", option.label))
                                .into_any_element(),
                        )
                    }),
                ))
                .child(
                    yttt_input(&custom_input, YtttInputKind::Settings, theme, style)
                        .disabled(!capability.can_answer),
                );
            card = card.child(question_body);
        }
        if capability.can_answer {
            let submit_session_id = session_id;
            let submit_request_id = record.request.id.clone();
            card = card.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .child(
                        yttt_button(
                            format!("command-center-answer:{draft_key}"),
                            self.ui_text.get(UiTextKey::CommandCenterQueueAnswer),
                            YtttButtonVariant::Primary,
                            theme,
                            style,
                            cx,
                        )
                        .on_click(cx.listener(
                            move |root, _, window, cx| {
                                root.command_center_submit_answers(
                                    &submit_session_id,
                                    &submit_request_id,
                                    window,
                                    cx,
                                );
                            },
                        )),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.text_subtle)
                            .child(self.ui_text.get(UiTextKey::CommandCenterQueuedHint)),
                    ),
            );
        } else {
            card = card.child(
                div().text_xs().text_color(capability.color(theme)).child(
                    self.ui_text
                        .get(capability.reason.expect("disabled capability")),
                ),
            );
        }
        if let Some(feedback) = form_feedback {
            card = card.child(
                div()
                    .text_xs()
                    .text_color(theme.warning)
                    .child(self.command_center_feedback_text(&feedback)),
            );
        }
        if command_center_question_is_resolved(record.state) {
            let answers = command_center_answers_text(record);
            card = card.child(div().text_xs().text_color(theme.success).child(format!(
                "{}{}",
                self.ui_text.get(command_center_question_label(record.state)),
                answers
            )));
        }
        card
    }

    fn command_center_conversation_panel(
        &mut self,
        sources: &[CommandCenterSource],
        theme: WorkbenchTheme,
        style: UiStyle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let Some((source, selection)) = self.command_center_selected(sources) else {
            return command_center_panel(
                self.ui_text.get(UiTextKey::CommandCenterLinkedConversation),
                self.ui_text
                    .get(UiTextKey::CommandCenterNoSelection)
                    .to_string(),
                theme,
            )
            .flex_basis(relative(1.12 / 3.28))
            .child(command_center_empty_panel(
                self.ui_text.get(UiTextKey::CommandCenterSelectRecord),
                theme,
            ));
        };
        let selection = selection.clone();
        let execution_id = selection.execution_id.clone();
        let task = selection.task_id.as_deref().and_then(|task_id| {
            source
                .snapshot
                .board
                .tasks
                .iter()
                .find(|task| task.execution_id == execution_id && task.item.id == task_id)
        });
        let execution = source
            .snapshot
            .board
            .executions
            .iter()
            .find(|record| record.id == execution_id);
        let title = task
            .map(|task| task.item.title.clone())
            .or_else(|| execution.map(|record| record.name.clone()))
            .unwrap_or_else(|| source.snapshot.primary_text());
        let selected_question = selection
            .question_request_id
            .as_deref()
            .zip(selection.question_id.as_deref())
            .and_then(|(request_id, question_id)| {
                source
                    .snapshot
                    .board
                    .questions
                    .iter()
                    .find(|record| record.request.id == request_id)
                    .and_then(|record| {
                        record
                            .request
                            .questions
                            .iter()
                            .find(|question| question.id == question_id)
                    })
                    .map(|question| question.question.clone())
            });
        let mut messages = source
            .snapshot
            .board
            .messages
            .iter()
            .filter(|message| message.execution_id == execution_id)
            .collect::<Vec<_>>();
        messages.sort_by_key(|message| message.timestamp);
        let mut transcript = div()
            .id("command-center-transcript")
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .p(px(8.0));
        if messages.is_empty() {
            transcript = transcript.child(command_center_empty_panel(
                self.ui_text.get(UiTextKey::CommandCenterNoConversation),
                theme,
            ));
        } else {
            for message in messages {
                transcript = transcript.child(
                    div()
                        .flex_none()
                        .flex()
                        .items_start()
                        .gap(px(6.0))
                        .mb(px(5.0))
                        .child(
                            div()
                                .w(px(38.0))
                                .text_xs()
                                .text_color(theme.text_subtle)
                                .child(command_center_time(self.ui_text, message.timestamp)),
                        )
                        .child(
                            div()
                                .min_w_0()
                                .flex_1()
                                .text_xs()
                                .child(
                                    div()
                                        .text_color(if message.role == AgentMessageRole::User {
                                            theme.accent
                                        } else {
                                            theme.icon_accent
                                        })
                                        .child(self.ui_text.get(
                                            if message.role == AgentMessageRole::User {
                                                UiTextKey::CommandCenterYou
                                            } else {
                                                UiTextKey::CommandCenterAgent
                                            },
                                        )),
                                )
                                .child(div().whitespace_normal().child(message.text.clone()))
                                .when(message.truncated, |row| {
                                    row.child(div().text_xs().text_color(theme.text_subtle).child(
                                        self.ui_text.get(UiTextKey::CommandCenterMessageTruncated),
                                    ))
                                }),
                        ),
                );
            }
        }
        let mut header = div()
            .flex_none()
            .px(px(8.0))
            .py(px(5.0))
            .border_b(px(1.0))
            .border_color(theme.border_variant)
            .text_xs()
            .text_color(theme.text_muted)
            .child(title);
        if let Some(execution) = execution {
            if let Some(action) = execution.current_action.as_deref() {
                header = header.child(div().truncate().child(format!(
                    "{} · {action}",
                    self.ui_text.get(UiTextKey::CommandCenterCurrentAction),
                )));
            }
            if let Some(metrics) = command_center_telemetry_line(
                self.ui_text,
                &execution.telemetry,
                source.snapshot.provider_id.as_str() == "claude",
                theme,
            ) {
                let details = command_center_execution_details(self.ui_text, source, execution);
                header = header.child(metrics.id("command-center-selected-telemetry").tooltip(
                    move |window, cx| {
                        yttt_ui::primitives::tooltip::yttt_tooltip(
                            details.clone(),
                            crate::ui::theme::current_workbench_theme(cx),
                            crate::ui::theme::current_ui_style(cx),
                        )
                        .build(window, cx)
                    },
                ));
            }
        }
        if let Some(question) = selected_question {
            header = header.child(div().truncate().child(format!(
                "{} · {question}",
                self.ui_text.get(UiTextKey::CommandCenterSelectedQuestion),
            )));
        }
        header = header.child(self.command_center_session_link(source, theme, style, cx));
        let mut panel = command_center_panel(
            self.ui_text.get(UiTextKey::CommandCenterLinkedConversation),
            format!(
                "{} · {}",
                source.project_name,
                self.command_center_session_label(source)
            ),
            theme,
        )
        .flex_basis(relative(1.12 / 3.28))
        .child(header)
        .child(transcript);
        panel =
            panel.child(self.command_center_selected_actions(
                source, &selection, task, theme, style, window, cx,
            ));
        panel
    }

    fn command_center_session_link(
        &self,
        source: &CommandCenterSource,
        theme: WorkbenchTheme,
        style: UiStyle,
        cx: &mut Context<Self>,
    ) -> Div {
        let can_open = source
            .session_target
            .as_ref()
            .is_some_and(|target| self.command_center_session_is_open(target));
        let target = source.session_target.clone();
        let session_id = source.terminal_session_id.clone();
        let hint = self.ui_text.get(if can_open {
            UiTextKey::CommandCenterOpenSessionHint
        } else {
            UiTextKey::CommandCenterSessionNotOpen
        });
        let feedback_key = format!("open-session:{}", source.terminal_session_id);
        div()
            .flex()
            .flex_col()
            .gap(px(4.0))
            .mt(px(4.0))
            .child(
                yttt_button(
                    "command-center-open-session",
                    self.ui_text.get(UiTextKey::CommandCenterOpenSession),
                    YtttButtonVariant::Secondary,
                    theme,
                    style,
                    cx,
                )
                .debug_selector(|| "command-center-open-session".to_string())
                .disabled(!can_open)
                .tooltip(hint)
                .on_click(cx.listener(move |root, _, window, cx| {
                    if let Some(target) = &target {
                        root.command_center_open_session(target, &session_id, window, cx);
                    }
                })),
            )
            .when(!can_open, |row| {
                row.child(
                    div()
                        .whitespace_normal()
                        .text_color(theme.text_subtle)
                        .child(hint),
                )
            })
            .children(
                self.command_center
                    .feedback
                    .get(&feedback_key)
                    .map(|feedback| {
                        div()
                            .whitespace_normal()
                            .text_color(theme.warning)
                            .child(self.command_center_feedback_text(feedback))
                    }),
            )
    }

    fn command_center_session_is_open(&self, target: &CommandCenterSessionTarget) -> bool {
        self.workspace
            .project(&target.project_id)
            .and_then(|project| project.layout.tab(&target.tab_id))
            .is_some_and(|tab| tab.layout.find_pane(&target.pane_id).is_some())
    }

    fn command_center_open_session(
        &mut self,
        target: &CommandCenterSessionTarget,
        session_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let feedback_key = format!("open-session:{session_id}");
        // Revalidate on click: the project, tab, or pane can close after rendering.
        if !self.command_center_session_is_open(target) {
            self.command_center.feedback.insert(
                feedback_key,
                CommandCenterFeedback::Text(UiTextKey::CommandCenterSessionNotOpen),
            );
            cx.notify();
            return;
        }
        match self.activate_agent_pane(&target.project_id, &target.tab_id, &target.pane_id) {
            Ok(()) => {
                self.command_center.feedback.remove(&feedback_key);
                window.blur();
            }
            Err(error) => {
                self.command_center.feedback.insert(
                    feedback_key,
                    CommandCenterFeedback::Detail {
                        key: UiTextKey::CommandCenterOpenSessionFailed,
                        detail: error.to_string(),
                    },
                );
            }
        }
        cx.notify();
    }

    fn command_center_selected_actions(
        &mut self,
        source: &CommandCenterSource,
        selection: &CommandCenterSelection,
        task: Option<&AgentBoardTask>,
        theme: WorkbenchTheme,
        style: UiStyle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let mut actions = div()
            .flex_none()
            .flex()
            .flex_col()
            .gap(px(5.0))
            .p(px(7.0))
            .border_t(px(1.0))
            .border_color(theme.border_variant);
        let prompt_target = format!(
            "task:{}:{}",
            source.terminal_session_id, selection.execution_id
        );
        let prompt_capability = self.command_center_prompt_capability(source);
        if prompt_capability.can_answer {
            let input = self.command_center_composer_input(&prompt_target, window, cx);
            let session_id = source.terminal_session_id.clone();
            let execution_id = selection.execution_id.clone();
            actions = actions
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.text_subtle)
                        .child(self.ui_text.get(UiTextKey::CommandCenterPromptHint)),
                )
                .child(
                    div()
                        .flex()
                        .gap(px(5.0))
                        .child(div().flex_1().min_w_0().child(yttt_input(
                            &input,
                            YtttInputKind::Settings,
                            theme,
                            style,
                        )))
                        .child(
                            yttt_button(
                                "command-center-send-prompt",
                                self.ui_text.get(UiTextKey::CommandCenterSend),
                                YtttButtonVariant::Primary,
                                theme,
                                style,
                                cx,
                            )
                            .on_click(cx.listener(
                                move |root, _, window, cx| {
                                    root.command_center_send_prompt(
                                        &session_id,
                                        &execution_id,
                                        window,
                                        cx,
                                    );
                                },
                            )),
                        ),
                );
        } else {
            actions = actions.child(
                div()
                    .text_xs()
                    .text_color(prompt_capability.color(theme))
                    .child(
                        self.ui_text
                            .get(prompt_capability.reason.expect("disabled capability")),
                    ),
            );
        }
        if let Some(task) = task
            && task.item.status == AgentPlanStatus::Completed
            && task.accepted_at.is_none()
            && !task.retired
        {
            let request_id = format!(
                "accept:{}:{}:{}",
                source.terminal_session_id, task.execution_id, task.item.id
            );
            let can_accept = source.operation_target.is_some() && self.shared_mutation_allowed();
            let session_id = source.terminal_session_id.clone();
            let execution_id = task.execution_id.clone();
            let task_id = task.item.id.clone();
            actions = actions.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .child(
                        yttt_button(
                            format!("command-center-accept:{session_id}:{execution_id}:{task_id}"),
                            self.ui_text
                                .get(UiTextKey::CommandCenterAcceptCompletedTask),
                            YtttButtonVariant::Secondary,
                            theme,
                            style,
                            cx,
                        )
                        .disabled(!can_accept)
                        .on_click(cx.listener(
                            move |root, _, window, cx| {
                                root.command_center_accept_task(
                                    &session_id,
                                    &execution_id,
                                    &task_id,
                                    window,
                                    cx,
                                );
                            },
                        )),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.text_subtle)
                            .child(self.ui_text.get(if can_accept {
                                UiTextKey::CommandCenterAcceptHint
                            } else if source.operation_target.is_none() {
                                UiTextKey::CommandCenterAcceptRetainedReadOnly
                            } else {
                                UiTextKey::CommandCenterAcceptDesktopReadOnly
                            })),
                    )
                    .children(
                        self.command_center
                            .feedback
                            .get(&request_id)
                            .map(|feedback| {
                                div()
                                    .text_xs()
                                    .text_color(theme.warning)
                                    .child(self.command_center_feedback_text(feedback))
                                    .into_any_element()
                            }),
                    ),
            );
        }
        if source.operation_target.is_some()
            && self
                .terminal
                .terminal_panes
                .contains_key(&source.terminal_session_id)
        {
            let management_open = self.command_center.management_session.as_deref()
                == Some(source.terminal_session_id.as_str());
            let session_id = source.terminal_session_id.clone();
            actions = actions.child(
                div()
                    .id("command-center-session-actions")
                    .cursor_pointer()
                    .text_xs()
                    .text_color(theme.text_subtle)
                    .child(self.ui_text.get(if management_open {
                        UiTextKey::CommandCenterHideSessionActions
                    } else {
                        UiTextKey::CommandCenterSessionActions
                    }))
                    .on_click(cx.listener(move |root, _, _, cx| {
                        root.command_center.management_session =
                            (!management_open).then(|| session_id.clone());
                        root.command_center.pending_terminal_termination = None;
                        cx.notify();
                    })),
            );
            if management_open {
                let can_terminate = self.shared_mutation_allowed();
                let session_id = source.terminal_session_id.clone();
                let requires_confirmation = can_terminate
                    && self.command_center.pending_terminal_termination.as_deref()
                        == Some(session_id.as_str());
                let label = self.ui_text.get(if requires_confirmation {
                    UiTextKey::CommandCenterConfirmTerminateTerminal
                } else {
                    UiTextKey::CommandCenterTerminateTerminal
                });
                actions = actions.child(
                    div()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap(px(6.0))
                        .child(
                            yttt_button(
                                format!("command-center-terminate:{session_id}"),
                                label,
                                YtttButtonVariant::Danger,
                                theme,
                                style,
                                cx,
                            )
                            .disabled(!can_terminate)
                            .on_click(cx.listener(
                                move |root, _, _window, cx| {
                                    root.command_center_terminate_terminal(&session_id, cx);
                                },
                            )),
                        )
                        .child(
                            div()
                                .min_w_0()
                                .w_full()
                                .text_xs()
                                .text_color(theme.text_subtle)
                                .child(self.ui_text.get(if can_terminate {
                                    UiTextKey::CommandCenterTerminateHint
                                } else {
                                    UiTextKey::CommandCenterTerminateDesktopReadOnly
                                })),
                        ),
                );
            }
        }
        let feedback_key = format!(
            "prompt:{}:{}",
            source.terminal_session_id, selection.execution_id
        );
        if let Some(feedback) = self.command_center.feedback.get(&feedback_key) {
            actions = actions.child(
                div()
                    .text_xs()
                    .text_color(theme.warning)
                    .child(self.command_center_feedback_text(feedback)),
            );
        }
        let termination_feedback_key = format!("terminate:{}", source.terminal_session_id);
        if let Some(feedback) = self.command_center.feedback.get(&termination_feedback_key) {
            actions = actions.child(
                div()
                    .text_xs()
                    .text_color(theme.warning)
                    .child(self.command_center_feedback_text(feedback)),
            );
        }
        actions
    }

    fn command_center_executions_panel(
        &mut self,
        sources: &[CommandCenterSource],
        theme: WorkbenchTheme,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let mut current_count = 0;
        let mut history_count = 0;
        for source in sources {
            if source.snapshot.board.executions.is_empty() {
                if source.snapshot.process_state == AgentProcessState::Exited {
                    history_count += 1;
                } else {
                    current_count += 1;
                }
            } else {
                for execution in &source.snapshot.board.executions {
                    if execution.ended_at.is_some() {
                        history_count += 1;
                    } else {
                        current_count += 1;
                    }
                }
            }
        }
        let expanded = self.command_center.execution_history_expanded;
        let mut rows = div()
            .id("command-center-executions")
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .p(px(4.0));
        if current_count == 0 {
            rows = rows.child(
                div()
                    .p(px(6.0))
                    .text_xs()
                    .text_color(theme.text_subtle)
                    .child(
                        self.ui_text
                            .get(UiTextKey::CommandCenterNoCurrentExecutions),
                    ),
            );
        }
        for history in [false, true] {
            if history && history_count > 0 {
                rows = rows.child(
                    div()
                        .id("command-center-execution-history")
                        .debug_selector(|| "command-center-execution-history".to_string())
                        .cursor_pointer()
                        .flex_none()
                        .py(px(5.0))
                        .px(px(4.0))
                        .text_xs()
                        .text_color(theme.text_muted)
                        .child(command_center_count(
                            self.ui_text,
                            if expanded {
                                UiTextKey::CommandCenterExecutionHistoryHide
                            } else {
                                UiTextKey::CommandCenterExecutionHistoryShow
                            },
                            history_count,
                        ))
                        .on_click(cx.listener(|root, _, _, cx| {
                            root.command_center.execution_history_expanded =
                                !root.command_center.execution_history_expanded;
                            cx.notify();
                        })),
                );
            }
            for source in sources {
                let selected_id = self
                    .command_center
                    .selection
                    .as_ref()
                    .filter(|selected| selected.terminal_session_id == source.terminal_session_id)
                    .map(|selected| selected.execution_id.as_str());
                let mut group = div().flex().flex_col().flex_none();
                let mut has_rows = false;
                if source.snapshot.board.executions.is_empty() {
                    let is_history = source.snapshot.process_state == AgentProcessState::Exited;
                    if history == is_history
                        && (!history
                            || expanded
                            || selected_id == Some(source.terminal_session_id.as_str()))
                    {
                        has_rows = true;
                        group = group.child(self.command_center_execution_row(
                            source,
                            None,
                            source.terminal_session_id.clone(),
                            self.ui_text.get(UiTextKey::CommandCenterExecutionSummary),
                            source.snapshot.turn_state,
                            source.snapshot.state_started_at,
                            is_history.then_some(source.snapshot.updated_at),
                            theme,
                            cx,
                        ));
                    }
                } else {
                    for execution in &source.snapshot.board.executions {
                        if history != execution.ended_at.is_some()
                            || (history && !expanded && selected_id != Some(execution.id.as_str()))
                        {
                            continue;
                        }
                        has_rows = true;
                        group = group.child(self.command_center_execution_row(
                            source,
                            Some(execution),
                            execution.id.clone(),
                            &execution.name,
                            execution.state,
                            execution.started_at,
                            execution.ended_at,
                            theme,
                            cx,
                        ));
                    }
                }
                if has_rows {
                    rows = rows
                        .child(
                            div()
                                .flex_none()
                                .px(px(4.0))
                                .pt(px(4.0))
                                .text_xs()
                                .text_color(theme.text_subtle)
                                .truncate()
                                .child(format!(
                                    "{} · {}",
                                    source.project_name,
                                    self.command_center_session_label(source)
                                )),
                        )
                        .child(group);
                }
            }
        }
        command_center_panel(
            self.ui_text.get(UiTextKey::CommandCenterExecutionRecords),
            command_center_count(
                self.ui_text,
                UiTextKey::CommandCenterActivityCount,
                current_count,
            ),
            theme,
        )
        .flex_basis(relative(0.78 / 3.28))
        .child(rows)
    }

    #[allow(clippy::too_many_arguments)]
    fn command_center_execution_row(
        &self,
        source: &CommandCenterSource,
        execution: Option<&AgentExecutionRecord>,
        execution_id: String,
        name: &str,
        state: AgentTurnState,
        started_at: u64,
        ended_at: Option<u64>,
        theme: WorkbenchTheme,
        cx: &Context<Self>,
    ) -> gpui::Stateful<Div> {
        let state = if ended_at.is_none()
            && source.snapshot.board.questions.iter().any(|question| {
                question.request.execution_id == execution_id
                    && matches!(
                        question.state,
                        AgentQuestionState::Pending | AgentQuestionState::Submitted
                    )
            }) {
            AgentTurnState::Waiting
        } else {
            state
        };
        let session_id = source.terminal_session_id.clone();
        let selected = self
            .command_center
            .selection
            .as_ref()
            .is_some_and(|selection| {
                selection.terminal_session_id == session_id
                    && selection.execution_id == execution_id
            });
        let hierarchy = execution
            .and_then(|record| record.parent_id.as_deref())
            .map(|_| "↳")
            .unwrap_or(self.ui_text.get(UiTextKey::CommandCenterRoot));
        let task = execution
            .and_then(|record| record.task.as_deref())
            .unwrap_or(name);
        let title = if execution.is_some_and(|record| record.parent_id.is_some()) {
            name
        } else {
            task
        };
        let live =
            source.snapshot.process_state == AgentProcessState::Running
                && state != AgentTurnState::Unknown
                && self.terminal.host_runtime.as_ref().is_some_and(|runtime| {
                    matches!(runtime.state(), ConnectionState::Ready { .. })
                });
        let (status_label, status_state) = if execution.is_none() {
            command_center_agent_status(
                &source.snapshot,
                source.operation_target.is_some()
                    && self.terminal.host_runtime.as_ref().is_some_and(|runtime| {
                        matches!(runtime.state(), ConnectionState::Ready { .. })
                    }),
            )
        } else {
            (command_center_turn_label(state), state)
        };
        let elapsed = execution
            .and_then(|record| record.telemetry.duration_ms)
            .map(|millis| command_center_format_duration(self.ui_text, millis))
            .unwrap_or_else(|| {
                command_center_duration(
                    self.ui_text,
                    started_at,
                    ended_at.or_else(|| {
                        (!live).then_some(
                            execution
                                .map_or(source.snapshot.updated_at, |record| record.updated_at),
                        )
                    }),
                )
            });
        let tooltip = execution
            .map(|record| self.command_center_usage_tooltip(source, &record.id, false, cx));
        let depth = execution.map_or(0, |record| {
            command_center_execution_depth(record, &source.snapshot.board.executions)
        });
        let mut row = div()
            .id(format!(
                "command-center-execution:{session_id}:{execution_id}"
            ))
            .debug_selector({
                let id = execution_id.clone();
                move || format!("command-center-execution-row:{id}")
            })
            .cursor_pointer()
            .flex()
            .flex_col()
            .flex_none()
            .min_w_0()
            .px(px(4.0))
            .py(px(2.0))
            .border_l(px(2.0))
            .border_color(if selected {
                theme.accent
            } else {
                theme.app_background
            })
            .bg(if selected {
                theme.selection
            } else {
                theme.ghost_element_background
            })
            .when_some(tooltip, |row, tooltip| row.tooltip(tooltip))
            .on_click(cx.listener(move |root, _, _window, cx| {
                root.command_center_select(session_id.clone(), execution_id.clone(), None, cx);
            }))
            .child(
                div()
                    .flex()
                    .items_center()
                    .min_w_0()
                    .min_h(px(23.0))
                    .gap(px(5.0))
                    .pl(px(depth.saturating_sub(1).min(3) as f32 * 8.0))
                    .child(
                        div()
                            .flex_none()
                            .text_xs()
                            .text_color(theme.text_subtle)
                            .child(hierarchy),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .flex_1()
                            .truncate()
                            .text_xs()
                            .child(title.to_string()),
                    )
                    .child(
                        div()
                            .flex_none()
                            .whitespace_nowrap()
                            .text_xs()
                            .text_color(command_center_turn_color(status_state, theme))
                            .child(self.ui_text.get(status_label)),
                    )
                    .child(
                        div()
                            .flex_none()
                            .min_w(px(37.0))
                            .text_right()
                            .whitespace_nowrap()
                            .text_xs()
                            .text_color(theme.text_subtle)
                            .child(elapsed),
                    ),
            );
        if let Some(metrics) = execution.and_then(|record| {
            command_center_telemetry_line(
                self.ui_text,
                &record.telemetry,
                source.snapshot.provider_id.as_str() == "claude",
                theme,
            )
        }) {
            row = row.child(metrics);
        }
        row
    }

    fn command_center_question_focus(
        &mut self,
        draft_key: &str,
        question_id: &str,
        cx: &mut Context<Self>,
    ) -> gpui::FocusHandle {
        self.command_center
            .question_drafts
            .entry(draft_key.to_string())
            .or_default()
            .question_focuses
            .entry(question_id.to_string())
            .or_insert_with(|| cx.focus_handle())
            .clone()
    }

    fn command_center_question_input(
        &mut self,
        draft_key: &str,
        question_id: &str,
        question_target: CommandCenterQuestionTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<InputState> {
        let locale = self.ui_text.locale();
        let placeholder = self
            .ui_text
            .get(UiTextKey::CommandCenterOptionalAnswerPlaceholder);
        let (existing, inputs_to_refresh) = {
            let draft = self
                .command_center
                .question_drafts
                .entry(draft_key.to_string())
                .or_default();
            let locale_changed = draft.input_locale.replace(locale) != Some(locale);
            let existing = draft.custom_inputs.get(question_id).cloned();
            let inputs_to_refresh = if locale_changed {
                draft.custom_inputs.values().cloned().collect::<Vec<_>>()
            } else {
                Vec::new()
            };
            (existing, inputs_to_refresh)
        };
        for input in inputs_to_refresh {
            input.update(cx, |input, cx| {
                input.set_placeholder(placeholder, window, cx)
            });
        }
        if let Some(input) = existing {
            return input;
        }
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(placeholder)
                .clean_on_escape()
        });
        let subscription =
            cx.subscribe_in(&input, window, move |root, input, event, window, cx| {
                root.on_command_center_question_input_event(
                    &question_target,
                    input,
                    event,
                    window,
                    cx,
                );
            });
        let draft = self
            .command_center
            .question_drafts
            .get_mut(draft_key)
            .expect("question draft was created");
        draft
            .custom_inputs
            .insert(question_id.to_string(), input.clone());
        draft
            .input_subscriptions
            .insert(question_id.to_string(), subscription);
        input
    }

    fn on_command_center_question_input_event(
        &mut self,
        question_target: &CommandCenterQuestionTarget,
        _input: &Entity<InputState>,
        event: &InputEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            InputEvent::Focus => self.command_center_select_question(
                question_target.clone(),
                Some(&question_target.question_id),
                cx,
            ),
            InputEvent::Change | InputEvent::Blur => cx.notify(),
            InputEvent::PressEnter { .. } => {}
        }
    }

    fn command_center_composer_input(
        &mut self,
        target: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<InputState> {
        let locale = self.ui_text.locale();
        let placeholder = self.ui_text.get(UiTextKey::CommandCenterPromptPlaceholder);
        let (input, existing) = if let Some(input) = &self.command_center.composer {
            (input.clone(), true)
        } else {
            let input = cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(placeholder)
                    .clean_on_escape()
            });
            let subscription =
                cx.subscribe_in(&input, window, Self::on_command_center_composer_input_event);
            self.command_center.composer = Some(input.clone());
            self.command_center.composer_subscription = Some(subscription);
            (input, false)
        };
        if self.command_center.composer_locale.replace(locale) != Some(locale) && existing {
            input.update(cx, |input, cx| {
                input.set_placeholder(placeholder, window, cx)
            });
        }
        if self.command_center.composer_target.as_deref() != Some(target) {
            if let Some(previous_target) = self.command_center.composer_target.clone() {
                self.command_center
                    .prompt_drafts
                    .insert(previous_target, input.read(cx).value().to_string());
            }
            self.command_center.composer_target = Some(target.to_string());
            let value = self
                .command_center
                .prompt_drafts
                .get(target)
                .cloned()
                .unwrap_or_default();
            input.update(cx, |input, cx| input.set_value(value, window, cx));
        }
        input
    }

    fn on_command_center_composer_input_event(
        &mut self,
        input: &Entity<InputState>,
        event: &InputEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            InputEvent::Change => {
                if let Some(target) = self.command_center.composer_target.clone() {
                    self.command_center
                        .prompt_drafts
                        .insert(target, input.read(cx).value().to_string());
                }
                cx.notify();
            }
            InputEvent::Focus | InputEvent::Blur => cx.notify(),
            InputEvent::PressEnter { .. } => {}
        }
    }

    pub(super) fn command_center_input_is_focused(&self, window: &Window, cx: &App) -> bool {
        let input_contains_focus = |input: &Entity<InputState>| {
            input.read(cx).focus_handle(cx).contains_focused(window, cx)
        };
        self.command_center
            .composer
            .as_ref()
            .is_some_and(input_contains_focus)
            || self
                .command_center
                .question_drafts
                .values()
                .flat_map(|draft| draft.custom_inputs.values())
                .any(input_contains_focus)
    }

    fn command_center_feedback_text(&self, feedback: &CommandCenterFeedback) -> String {
        match feedback {
            CommandCenterFeedback::Text(key) => self.ui_text.get(*key).to_string(),
            CommandCenterFeedback::Detail { key, detail } => {
                format!("{}: {detail}", self.ui_text.get(*key))
            }
            CommandCenterFeedback::QuestionNotAwaiting(state) => command_center_template(
                self.ui_text
                    .get(UiTextKey::CommandCenterQuestionNotAwaiting),
                &[(
                    "state",
                    self.ui_text.get(command_center_question_label(*state)),
                )],
            ),
        }
    }

    fn command_center_select(
        &mut self,
        terminal_session_id: String,
        execution_id: String,
        task_id: Option<String>,
        cx: &mut Context<Self>,
    ) {
        self.command_center.selection = Some(CommandCenterSelection {
            terminal_session_id,
            execution_id,
            task_id,
            question_request_id: None,
            question_id: None,
        });
        self.command_center.management_session = None;
        self.command_center.pending_terminal_termination = None;
        cx.notify();
    }

    fn command_center_select_question(
        &mut self,
        target: CommandCenterQuestionTarget,
        question_id: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        self.command_center.selection = Some(CommandCenterSelection {
            terminal_session_id: target.terminal_session_id,
            execution_id: target.execution_id,
            task_id: None,
            question_request_id: Some(target.request_id),
            question_id: question_id.map(str::to_string),
        });
        self.command_center.management_session = None;
        self.command_center.pending_terminal_termination = None;
        cx.notify();
    }

    fn command_center_select_option(
        &mut self,
        target: CommandCenterQuestionTarget,
        draft_key: &str,
        question_id: &str,
        label: String,
        multi: bool,
        cx: &mut Context<Self>,
    ) {
        self.command_center_select_question(target, Some(question_id), cx);
        self.command_center_toggle_option(draft_key, question_id, label, multi, cx);
    }

    fn command_center_select_option_by_index(
        &mut self,
        target: &CommandCenterQuestionTarget,
        option_index: usize,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.command_center_is_active()
            || !self
                .command_center
                .selection
                .as_ref()
                .is_some_and(|selection| {
                    selection.terminal_session_id == target.terminal_session_id
                        && selection.execution_id == target.execution_id
                        && selection.question_request_id.as_deref()
                            == Some(target.request_id.as_str())
                        && selection.question_id.as_deref() == Some(target.question_id.as_str())
                })
        {
            return false;
        }
        let sources = self.command_center_sources();
        let Some(source) = sources
            .iter()
            .find(|source| source.terminal_session_id == target.terminal_session_id)
        else {
            return false;
        };
        let Some(record) = source.snapshot.board.questions.iter().find(|record| {
            record.request.id == target.request_id
                && record.request.execution_id == target.execution_id
        }) else {
            return false;
        };
        if !self
            .command_center_question_capability(source, record)
            .can_answer
        {
            return false;
        }
        let Some(question) = record
            .request
            .questions
            .iter()
            .find(|question| question.id == target.question_id)
        else {
            return false;
        };
        let Some(option) = question.options.get(option_index) else {
            return false;
        };
        let draft_key =
            command_center_question_key(&target.terminal_session_id, &target.request_id);
        self.command_center_select_option(
            target.clone(),
            &draft_key,
            &question.id,
            option.label.clone(),
            question.multi,
            cx,
        );
        true
    }

    fn command_center_move_selected_question(
        &mut self,
        previous: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let sources = self.command_center_sources();
        self.command_center_move_selected_question_in_sources(&sources, previous, cx);
        if let Some(selection) = &self.command_center.selection
            && let (Some(request_id), Some(question_id)) =
                (&selection.question_request_id, &selection.question_id)
        {
            let draft_key = command_center_question_key(&selection.terminal_session_id, request_id);
            let question_id = question_id.clone();
            self.command_center_question_focus(&draft_key, &question_id, cx)
                .focus(window, cx);
        }
    }

    fn command_center_move_selected_question_in_sources(
        &mut self,
        sources: &[CommandCenterSource],
        previous: bool,
        cx: &mut Context<Self>,
    ) {
        let targets = || {
            sources.iter().flat_map(|source| {
                source
                    .snapshot
                    .board
                    .questions
                    .iter()
                    .filter(|record| record.state == AgentQuestionState::Pending)
                    .flat_map(move |record| {
                        record
                            .request
                            .questions
                            .iter()
                            .map(move |question| (source, record, question))
                    })
            })
        };
        let current = self
            .command_center
            .selection
            .as_ref()
            .and_then(|selection| {
                targets().position(|(source, record, question)| {
                    selection.terminal_session_id == source.terminal_session_id
                        && selection.question_request_id.as_deref()
                            == Some(record.request.id.as_str())
                        && selection.question_id.as_deref() == Some(question.id.as_str())
                })
            });
        let next = match (current, previous) {
            (Some(current), true) => targets().nth(current.saturating_sub(1)),
            (Some(current), false) => targets().nth(current + 1).or_else(|| targets().last()),
            (None, true) => targets().last(),
            (None, false) => targets().next(),
        };
        let Some((source, record, question)) = next else {
            return;
        };
        let target = CommandCenterQuestionTarget {
            terminal_session_id: source.terminal_session_id.clone(),
            execution_id: record.request.execution_id.clone(),
            request_id: record.request.id.clone(),
            question_id: question.id.clone(),
        };
        let record_index = sources
            .iter()
            .flat_map(|source| {
                source
                    .snapshot
                    .board
                    .questions
                    .iter()
                    .filter(|record| {
                        matches!(
                            record.state,
                            AgentQuestionState::Pending | AgentQuestionState::Submitted
                        )
                    })
                    .map(move |record| (source, record))
            })
            .chain(sources.iter().flat_map(|source| {
                source
                    .snapshot
                    .board
                    .questions
                    .iter()
                    .filter(|record| matches!(record.state, AgentQuestionState::Unavailable))
                    .map(move |record| (source, record))
            }))
            .position(|(source, record)| {
                source.terminal_session_id == target.terminal_session_id
                    && record.request.id == target.request_id
            });
        self.command_center_select_question(target.clone(), Some(&target.question_id), cx);
        if let Some(record_index) = record_index {
            // Only explicit navigation scrolls. The toolbar is outside the scroll container.
            self.command_center
                .question_scroll
                .scroll_to_item(record_index);
        }
    }

    pub(super) fn on_command_center_question_previous(
        &mut self,
        _: &CommandCenterQuestionPrevious,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.command_center_is_active() || self.command_center_input_is_focused(window, cx) {
            cx.propagate();
            return;
        }
        self.command_center_move_selected_question(true, window, cx);
    }

    pub(super) fn on_command_center_question_next(
        &mut self,
        _: &CommandCenterQuestionNext,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.command_center_is_active() || self.command_center_input_is_focused(window, cx) {
            cx.propagate();
            return;
        }
        self.command_center_move_selected_question(false, window, cx);
    }

    pub(super) fn on_command_center_question_submit(
        &mut self,
        _: &CommandCenterQuestionSubmit,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.command_center_is_active() || self.command_center_input_is_focused(window, cx) {
            cx.propagate();
            return;
        }
        let Some((terminal_session_id, request_id)) =
            self.command_center
                .selection
                .as_ref()
                .and_then(|selection| {
                    selection.question_request_id.as_ref().map(|request_id| {
                        (selection.terminal_session_id.clone(), request_id.clone())
                    })
                })
        else {
            return;
        };
        self.command_center_submit_answers(&terminal_session_id, &request_id, window, cx);
    }

    fn command_center_toggle_option(
        &mut self,
        draft_key: &str,
        question_id: &str,
        label: String,
        multi: bool,
        cx: &mut Context<Self>,
    ) {
        let selected = self
            .command_center
            .question_drafts
            .entry(draft_key.to_string())
            .or_default()
            .selected_options
            .entry(question_id.to_string())
            .or_default();
        if multi {
            if !selected.insert(label.clone()) {
                selected.remove(&label);
            }
        } else if selected.contains(&label) {
            selected.clear();
        } else {
            selected.clear();
            selected.insert(label);
        }
        self.command_center.feedback.remove(draft_key);
        cx.notify();
    }

    fn command_center_submit_answers(
        &mut self,
        terminal_session_id: &str,
        request_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let draft_key = command_center_question_key(terminal_session_id, request_id);
        let sources = self.command_center_sources();
        let Some(source) = sources
            .iter()
            .find(|source| source.terminal_session_id == terminal_session_id)
        else {
            self.command_center.feedback.insert(
                draft_key,
                CommandCenterFeedback::Text(UiTextKey::CommandCenterQuestionNoLongerRetained),
            );
            cx.notify();
            return;
        };
        let Some(record) = source
            .snapshot
            .board
            .questions
            .iter()
            .find(|record| record.request.id == request_id)
        else {
            self.command_center.feedback.insert(
                draft_key,
                CommandCenterFeedback::Text(UiTextKey::CommandCenterQuestionNoLongerRetained),
            );
            cx.notify();
            return;
        };
        let capability = self.command_center_question_capability(source, record);
        if !capability.can_answer {
            self.command_center.feedback.insert(
                draft_key,
                CommandCenterFeedback::Text(capability.reason.expect("disabled capability")),
            );
            cx.notify();
            return;
        }
        let Some(draft) = self.command_center.question_drafts.get(&draft_key) else {
            self.command_center.feedback.insert(
                draft_key,
                CommandCenterFeedback::Text(UiTextKey::CommandCenterAnswerAllQuestions),
            );
            cx.notify();
            return;
        };
        let answers = record
            .request
            .questions
            .iter()
            .map(|question| AgentQuestionAnswer {
                question_id: question.id.clone(),
                selected_options: draft
                    .selected_options
                    .get(&question.id)
                    .map(|selected| selected.iter().cloned().collect())
                    .unwrap_or_default(),
                custom_input: draft
                    .custom_inputs
                    .get(&question.id)
                    .map(|input| input.read(cx).value().trim().to_string())
                    .filter(|value| !value.is_empty()),
            })
            .collect::<Vec<_>>();
        if answers.iter().any(|answer| {
            answer.selected_options.is_empty()
                && answer
                    .custom_input
                    .as_deref()
                    .unwrap_or_default()
                    .is_empty()
        }) {
            self.command_center.feedback.insert(
                draft_key,
                CommandCenterFeedback::Text(UiTextKey::CommandCenterAnswerRequired),
            );
            cx.notify();
            return;
        }
        if let Err(error) =
            self.command_center_validate_answer_feedback(source, request_id, &answers)
        {
            self.command_center.feedback.insert(draft_key, error);
            cx.notify();
            return;
        }
        self.command_center_dispatch_board_command(
            source,
            AgentBoardCommand::AnswerQuestion {
                request_id: request_id.to_string(),
                answers,
            },
            draft_key,
            window,
            cx,
        );
    }

    fn command_center_accept_task(
        &mut self,
        session_id: &str,
        execution_id: &str,
        task_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let sources = self.command_center_sources();
        let Some(source) = sources
            .iter()
            .find(|source| source.terminal_session_id == session_id)
        else {
            return;
        };
        let feedback_key = format!("accept:{session_id}:{execution_id}:{task_id}");
        self.command_center_dispatch_board_command(
            source,
            AgentBoardCommand::AcceptTask {
                execution_id: execution_id.to_string(),
                task_id: task_id.to_string(),
            },
            feedback_key,
            window,
            cx,
        );
    }

    fn command_center_dispatch_board_command(
        &mut self,
        source: &CommandCenterSource,
        command: AgentBoardCommand,
        feedback_key: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (runtime, request) = match self.command_center_host_request_feedback(source, command) {
            Ok(request) => request,
            Err(feedback) => {
                self.command_center.feedback.insert(feedback_key, feedback);
                cx.notify();
                return;
            }
        };
        self.command_center.feedback.insert(
            feedback_key.clone(),
            CommandCenterFeedback::Text(UiTextKey::CommandCenterSendingToHost),
        );
        cx.spawn_in(window, async move |this, cx| {
            let result = runtime.request(request).recv_async().await;
            let _ = this.update_in(cx, |root, _window, cx| {
                let feedback = match result {
                    Ok(Ok(Response::Applied)) => {
                        CommandCenterFeedback::Text(UiTextKey::CommandCenterQueuedWaitingProvider)
                    }
                    Ok(Ok(_)) => CommandCenterFeedback::Text(
                        UiTextKey::CommandCenterUnexpectedCommandResponse,
                    ),
                    Ok(Err(error)) => CommandCenterFeedback::Detail {
                        key: UiTextKey::CommandCenterHostRejectedCommand,
                        detail: error.to_string(),
                    },
                    Err(_) => {
                        CommandCenterFeedback::Text(UiTextKey::CommandCenterCommandChannelClosed)
                    }
                };
                root.command_center.feedback.insert(feedback_key, feedback);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn command_center_host_request_feedback(
        &self,
        source: &CommandCenterSource,
        command: AgentBoardCommand,
    ) -> Result<
        (
            std::sync::Arc<crate::host_runtime::DesktopHostRuntime>,
            Request,
        ),
        CommandCenterFeedback,
    > {
        let runtime = self
            .terminal
            .host_runtime
            .clone()
            .ok_or(CommandCenterFeedback::Text(
                UiTextKey::CommandCenterHostUnavailableReadOnly,
            ))?;
        let target = source
            .operation_target
            .clone()
            .ok_or(CommandCenterFeedback::Text(
                UiTextKey::CommandCenterLiveSnapshotRequired,
            ))?;
        if !self.shared_mutation_allowed() {
            return Err(CommandCenterFeedback::Text(
                UiTextKey::CommandCenterDesktopReadOnlyCommands,
            ));
        }
        Ok((
            runtime,
            Request::AgentCommand(AgentCommandRequest {
                scope: source.scope.clone(),
                host_epoch: target.host_epoch,
                instance_id: target.instance_id,
                command,
            }),
        ))
    }

    pub(super) fn command_center_host_request(
        &self,
        source: &CommandCenterSource,
        command: AgentBoardCommand,
    ) -> Result<
        (
            std::sync::Arc<crate::host_runtime::DesktopHostRuntime>,
            Request,
        ),
        String,
    > {
        self.command_center_host_request_feedback(source, command)
            .map_err(|feedback| self.command_center_feedback_text(&feedback))
    }

    fn command_center_validate_answer_feedback(
        &self,
        source: &CommandCenterSource,
        request_id: &str,
        answers: &[AgentQuestionAnswer],
    ) -> Result<(), CommandCenterFeedback> {
        let record = source
            .snapshot
            .board
            .questions
            .iter()
            .find(|record| record.request.id == request_id)
            .ok_or(CommandCenterFeedback::Text(
                UiTextKey::CommandCenterQuestionNotInSelectedPane,
            ))?;
        if !matches!(record.state, AgentQuestionState::Pending) {
            return Err(CommandCenterFeedback::QuestionNotAwaiting(record.state));
        }
        if !record.request.answerable {
            return Err(CommandCenterFeedback::Text(
                UiTextKey::CommandCenterProviderNoAnswerChannel,
            ));
        }
        let mut board = source.snapshot.board.clone();
        board
            .command(
                AgentBoardCommand::AnswerQuestion {
                    request_id: request_id.to_string(),
                    answers: answers.to_vec(),
                },
                command_center_now_millis(),
            )
            .map_err(|error| CommandCenterFeedback::Detail {
                key: UiTextKey::CommandCenterAnswerRejected,
                detail: error.to_string(),
            })
    }

    pub(super) fn command_center_validate_answer(
        &self,
        source: &CommandCenterSource,
        request_id: &str,
        answers: &[AgentQuestionAnswer],
    ) -> Result<(), String> {
        self.command_center_validate_answer_feedback(source, request_id, answers)
            .map_err(|feedback| self.command_center_feedback_text(&feedback))
    }

    fn command_center_send_prompt(
        &mut self,
        session_id: &str,
        execution_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let target = format!("task:{session_id}:{execution_id}");
        let text = self
            .command_center
            .prompt_drafts
            .get(&target)
            .cloned()
            .unwrap_or_default();
        let feedback_key = format!("prompt:{session_id}:{execution_id}");
        if text.trim().is_empty() {
            self.command_center.feedback.insert(
                feedback_key,
                CommandCenterFeedback::Text(UiTextKey::CommandCenterPromptRequired),
            );
            cx.notify();
            return;
        }
        let sources = self.command_center_sources();
        let Some(source) = sources
            .iter()
            .find(|source| source.terminal_session_id == session_id)
        else {
            self.command_center.feedback.insert(
                feedback_key,
                CommandCenterFeedback::Text(UiTextKey::CommandCenterSessionNoLongerRetained),
            );
            cx.notify();
            return;
        };
        let capability = self.command_center_prompt_capability(source);
        if !capability.can_answer {
            self.command_center.feedback.insert(
                feedback_key,
                CommandCenterFeedback::Text(capability.reason.expect("disabled capability")),
            );
            cx.notify();
            return;
        }
        let Some(pane) = self.terminal.terminal_panes.get(session_id).cloned() else {
            self.command_center.feedback.insert(
                feedback_key,
                CommandCenterFeedback::Text(UiTextKey::CommandCenterPromptOtherWindow),
            );
            cx.notify();
            return;
        };
        let (response, _) = match pane.read(cx).control_input(&text, true, false, cx) {
            Ok(response) => response,
            Err(error) => {
                self.command_center.feedback.insert(
                    feedback_key,
                    CommandCenterFeedback::Detail {
                        key: UiTextKey::CommandCenterInputFailed,
                        detail: error.to_string(),
                    },
                );
                cx.notify();
                return;
            }
        };
        self.command_center.feedback.insert(
            feedback_key.clone(),
            CommandCenterFeedback::Text(UiTextKey::CommandCenterAwaitingHostInput),
        );
        let target_for_reply = target.clone();
        cx.spawn_in(window, async move |this, cx| {
            let result = response.recv_async().await;
            let _ = this.update_in(cx, |root, window, cx| {
                let feedback = match result {
                    Ok(Ok(Response::TerminalInputAccepted { .. })) => {
                        root.command_center.prompt_drafts.remove(&target_for_reply);
                        if root.command_center.composer_target.as_deref()
                            == Some(target_for_reply.as_str())
                            && let Some(input) = &root.command_center.composer
                        {
                            input.update(cx, |input, cx| input.set_value("", window, cx));
                        }
                        CommandCenterFeedback::Text(UiTextKey::CommandCenterPromptAccepted)
                    }
                    Ok(Ok(_)) => {
                        CommandCenterFeedback::Text(UiTextKey::CommandCenterUnexpectedInputResponse)
                    }
                    Ok(Err(error)) => CommandCenterFeedback::Detail {
                        key: UiTextKey::CommandCenterHostRejectedInput,
                        detail: error.to_string(),
                    },
                    Err(_) => CommandCenterFeedback::Text(
                        UiTextKey::CommandCenterInputAcknowledgementLost,
                    ),
                };
                root.command_center.feedback.insert(feedback_key, feedback);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn command_center_terminate_terminal(&mut self, session_id: &str, cx: &mut Context<Self>) {
        if self.command_center.pending_terminal_termination.as_deref() != Some(session_id) {
            self.command_center.pending_terminal_termination = Some(session_id.to_string());
            cx.notify();
            return;
        }
        let sources = self.command_center_sources();
        let Some(source) = sources
            .iter()
            .find(|source| source.terminal_session_id == session_id)
        else {
            self.command_center.pending_terminal_termination = None;
            cx.notify();
            return;
        };
        let Some(_) = &source.operation_target else {
            self.command_center.pending_terminal_termination = None;
            cx.notify();
            return;
        };
        let feedback_key = format!("terminate:{session_id}");
        match self.terminate_host_pane(
            &source.scope.project_id,
            &source.scope.tab_id,
            &source.scope.pane_id,
        ) {
            Ok(()) => {
                self.command_center.feedback.insert(
                    feedback_key,
                    CommandCenterFeedback::Text(UiTextKey::CommandCenterTerminationConfirmed),
                );
            }
            Err(error) => {
                self.command_center.feedback.insert(
                    feedback_key,
                    CommandCenterFeedback::Detail {
                        key: UiTextKey::CommandCenterHostRejectedTermination,
                        detail: error.to_string(),
                    },
                );
            }
        }
        self.command_center.pending_terminal_termination = None;
        cx.notify();
    }

    fn command_center_question_capability(
        &self,
        source: &CommandCenterSource,
        record: &AgentQuestionRecord,
    ) -> CommandCenterCapability {
        if !matches!(record.state, AgentQuestionState::Pending) {
            let (reason, severity) = match record.state {
                AgentQuestionState::Submitted => (
                    UiTextKey::CommandCenterQuestionSubmitted,
                    CommandCenterSeverity::Muted,
                ),
                AgentQuestionState::Answered => (
                    UiTextKey::CommandCenterQuestionAnswered,
                    CommandCenterSeverity::Warning,
                ),
                AgentQuestionState::Cancelled => (
                    UiTextKey::CommandCenterQuestionCancelled,
                    CommandCenterSeverity::Warning,
                ),
                AgentQuestionState::Unavailable => (
                    UiTextKey::CommandCenterQuestionUnavailable,
                    CommandCenterSeverity::Warning,
                ),
                AgentQuestionState::Pending => unreachable!(),
            };
            return CommandCenterCapability::disabled(reason, severity);
        }
        if !record.request.answerable {
            return CommandCenterCapability::disabled(
                UiTextKey::CommandCenterProviderNoAnswerChannel,
                CommandCenterSeverity::Muted,
            );
        }
        if source.operation_target.is_none() {
            return CommandCenterCapability::disabled(
                UiTextKey::CommandCenterQuestionLiveSnapshotRequired,
                CommandCenterSeverity::Muted,
            );
        }
        if !self.shared_mutation_allowed() {
            return CommandCenterCapability::disabled(
                UiTextKey::CommandCenterDesktopReadOnlyAnswer,
                CommandCenterSeverity::Warning,
            );
        }
        CommandCenterCapability::enabled()
    }

    fn command_center_prompt_capability(
        &self,
        source: &CommandCenterSource,
    ) -> CommandCenterCapability {
        if source.operation_target.is_none() {
            return CommandCenterCapability::disabled(
                UiTextKey::CommandCenterPromptRetainedReadOnly,
                CommandCenterSeverity::Warning,
            );
        }
        if !self
            .terminal
            .terminal_panes
            .contains_key(&source.terminal_session_id)
        {
            return CommandCenterCapability::disabled(
                UiTextKey::CommandCenterPromptOtherWindow,
                CommandCenterSeverity::Muted,
            );
        }
        if !self.shared_mutation_allowed() {
            return CommandCenterCapability::disabled(
                UiTextKey::CommandCenterDesktopReadOnlyPrompt,
                CommandCenterSeverity::Warning,
            );
        }
        if source.snapshot.process_state != AgentProcessState::Running {
            return CommandCenterCapability::disabled(
                UiTextKey::CommandCenterAgentNotRunning,
                CommandCenterSeverity::Muted,
            );
        }
        if !matches!(
            source.snapshot.turn_state,
            AgentTurnState::Idle
                | AgentTurnState::Completed
                | AgentTurnState::Failed
                | AgentTurnState::Interrupted
        ) {
            return CommandCenterCapability::disabled(
                UiTextKey::CommandCenterAgentBusy,
                CommandCenterSeverity::Muted,
            );
        }
        CommandCenterCapability::enabled()
    }

    fn command_center_selected<'a>(
        &self,
        sources: &'a [CommandCenterSource],
    ) -> Option<(&'a CommandCenterSource, &CommandCenterSelection)> {
        let selection = self.command_center.selection.as_ref()?;
        let source = sources
            .iter()
            .find(|source| source.terminal_session_id == selection.terminal_session_id)?;
        Some((source, selection))
    }
}

#[derive(Clone, Copy)]
enum CommandCenterSeverity {
    Muted,
    Warning,
}

struct CommandCenterCapability {
    can_answer: bool,
    reason: Option<UiTextKey>,
    severity: CommandCenterSeverity,
}

impl CommandCenterCapability {
    fn enabled() -> Self {
        Self {
            can_answer: true,
            reason: None,
            severity: CommandCenterSeverity::Muted,
        }
    }

    fn disabled(reason: UiTextKey, severity: CommandCenterSeverity) -> Self {
        Self {
            can_answer: false,
            reason: Some(reason),
            severity,
        }
    }

    fn color(&self, theme: WorkbenchTheme) -> gpui::Rgba {
        match self.severity {
            CommandCenterSeverity::Muted => theme.text_subtle,
            CommandCenterSeverity::Warning => theme.warning,
        }
    }
}

#[derive(Clone, Copy)]
enum CommandCenterHostStatus {
    Live,
    LiveReadOnly,
    DisconnectedRetained,
    UnavailableRetained,
}

impl CommandCenterHostStatus {
    fn key(self) -> UiTextKey {
        match self {
            Self::Live => UiTextKey::CommandCenterHostLive,
            Self::LiveReadOnly => UiTextKey::CommandCenterHostLiveReadOnly,
            Self::DisconnectedRetained => UiTextKey::CommandCenterHostDisconnectedRetained,
            Self::UnavailableRetained => UiTextKey::CommandCenterHostUnavailableRetained,
        }
    }

    fn color(self, theme: WorkbenchTheme) -> gpui::Rgba {
        match self {
            Self::Live | Self::LiveReadOnly => theme.text_muted,
            Self::DisconnectedRetained | Self::UnavailableRetained => theme.warning,
        }
    }
}

fn command_center_panel(title: &str, note: String, theme: WorkbenchTheme) -> Div {
    div()
        .flex()
        .flex_col()
        .min_w_0()
        .min_h_0()
        .border(px(1.0))
        .border_color(theme.border)
        .bg(theme.surface)
        .child(
            div()
                .flex_none()
                .flex()
                .flex_wrap()
                .gap_x(px(6.0))
                .gap_y(px(2.0))
                .items_center()
                .justify_between()
                .min_h(px(23.0))
                .py(px(3.0))
                .px(px(8.0))
                .border_b(px(1.0))
                .border_color(theme.border_variant)
                .child(
                    div()
                        .flex_none()
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(title.to_string()),
                )
                .child(
                    div()
                        .min_w_0()
                        .flex_basis(px(80.0))
                        .flex_grow(1.0)
                        .truncate()
                        .text_xs()
                        .text_right()
                        .text_color(theme.text_subtle)
                        .child(note),
                ),
        )
}

fn command_center_empty_panel(message: &str, theme: WorkbenchTheme) -> Div {
    div()
        .flex()
        .flex_1()
        .min_w_0()
        .min_h_0()
        .items_center()
        .justify_center()
        .p(px(10.0))
        .text_xs()
        .text_color(theme.text_subtle)
        .child(
            div()
                .w_full()
                .whitespace_normal()
                .text_center()
                .child(message.to_string()),
        )
}

fn sources_project_count(sources: &[CommandCenterSource]) -> usize {
    sources
        .iter()
        .map(|source| source.scope.project_id.as_str())
        .collect::<BTreeSet<_>>()
        .len()
}

fn command_center_template(template: &str, replacements: &[(&str, &str)]) -> String {
    let mut rendered = template.to_string();
    for &(name, value) in replacements {
        rendered = rendered.replace(&format!("{{{name}}}"), value);
    }
    rendered
}

fn command_center_count(ui_text: UiText, key: UiTextKey, count: usize) -> String {
    let count = count.to_string();
    command_center_template(ui_text.get(key), &[("count", &count)])
}

fn command_center_task_is_history(task: &AgentBoardTask) -> bool {
    task.retired
        || matches!(
            task.item.status,
            AgentPlanStatus::Completed | AgentPlanStatus::Abandoned
        )
}

fn command_center_task_priority(task: &AgentBoardTask) -> u8 {
    if command_center_task_is_history(task) {
        return 3;
    }
    match task.item.status {
        AgentPlanStatus::InProgress | AgentPlanStatus::Blocked => 0,
        AgentPlanStatus::Pending => 1,
        _ => 2,
    }
}

fn command_center_agent_label(snapshot: &AgentSnapshot) -> String {
    let provider = BuiltinAgent::ALL
        .into_iter()
        .find(|agent| agent.id() == snapshot.provider_id.as_str())
        .map(BuiltinAgent::display_name)
        .unwrap_or(snapshot.provider_id.as_str());
    let title = snapshot
        .session
        .as_ref()
        .and_then(|session| session.title.as_deref())
        .filter(|title| !title.trim().is_empty())
        .or_else(|| snapshot.task.as_ref().map(|task| task.title.as_str()));
    match title
        .map(str::trim)
        .filter(|title| !title.is_empty() && *title != provider)
    {
        Some(title) => format!("{provider} · {title}"),
        None => provider.to_string(),
    }
}

fn command_center_agent_status(
    snapshot: &AgentSnapshot,
    live: bool,
) -> (UiTextKey, AgentTurnState) {
    if snapshot.process_state == AgentProcessState::Exited {
        return if snapshot
            .process_exit
            .is_none_or(|exit| exit.reason == AgentExitReason::Disconnected)
        {
            (
                UiTextKey::CommandCenterAgentDisconnected,
                AgentTurnState::Unknown,
            )
        } else {
            (
                UiTextKey::CommandCenterAgentExited,
                AgentTurnState::Interrupted,
            )
        };
    }
    if !live {
        return (
            UiTextKey::CommandCenterAgentRetained,
            AgentTurnState::Unknown,
        );
    }
    if snapshot.process_state != AgentProcessState::Running {
        return (
            UiTextKey::CommandCenterAgentStarting,
            AgentTurnState::Unknown,
        );
    }
    let label = match snapshot.turn_state {
        AgentTurnState::Idle => UiTextKey::CommandCenterAgentOnlineIdle,
        AgentTurnState::Working => UiTextKey::CommandCenterAgentOnlineWorking,
        AgentTurnState::Waiting => UiTextKey::CommandCenterAgentOnlineWaiting,
        AgentTurnState::Completed => UiTextKey::CommandCenterAgentOnlineCompleted,
        AgentTurnState::Failed => UiTextKey::CommandCenterAgentOnlineFailed,
        AgentTurnState::Interrupted => UiTextKey::CommandCenterAgentOnlineInterrupted,
        AgentTurnState::Unknown => UiTextKey::CommandCenterAgentOnlineUnknown,
    };
    (label, snapshot.turn_state)
}

// A live session remains visible after all its work becomes history. An empty, ended
// observation belongs to history instead of masquerading as current unknown work.
fn command_center_session_summary_priority(
    source: &CommandCenterSource,
    usage: &CommandCenterProjectUsage<'_>,
) -> Option<u8> {
    if source.snapshot.process_state == AgentProcessState::Exited {
        return (source.snapshot.board.tasks.is_empty() && usage.independent.is_empty())
            .then_some(3);
    }
    let has_current_work = source
        .snapshot
        .board
        .tasks
        .iter()
        .any(|task| !command_center_task_is_history(task))
        || usage
            .independent
            .iter()
            .any(|execution| command_center_execution_priority(execution) != 3);
    (!has_current_work).then_some(0)
}

struct CommandCenterProjectUsage<'a> {
    task_labels: HashMap<&'a str, SharedString>,
    independent: Vec<&'a AgentExecutionRecord>,
}

impl<'a> CommandCenterProjectUsage<'a> {
    fn new(ui_text: UiText, source: &'a CommandCenterSource) -> Self {
        let unknown = SharedString::from(ui_text.get(UiTextKey::CommandCenterTaskUsageUnknown));
        let mut task_labels: HashMap<_, _> = source
            .snapshot
            .board
            .tasks
            .iter()
            .map(|task| (task.execution_id.as_str(), unknown.clone()))
            .collect();
        let mut independent = Vec::new();
        for execution in &source.snapshot.board.executions {
            if let Some(label) = task_labels.get_mut(execution.id.as_str()) {
                *label = command_center_task_usage(ui_text, source, Some(execution)).into();
            } else if execution.telemetry.tokens.is_some()
                || execution.telemetry.input_tokens.is_some()
                || execution.telemetry.output_tokens.is_some()
                || execution.telemetry.cache_read_tokens.is_some()
                || execution.telemetry.cache_write_tokens.is_some()
                || execution.telemetry.cost_microusd.is_some()
            {
                independent.push(execution);
            }
        }
        Self {
            task_labels,
            independent,
        }
    }
}

fn command_center_execution_priority(execution: &AgentExecutionRecord) -> u8 {
    if execution.ended_at.is_some() {
        return 3;
    }
    match execution.state {
        AgentTurnState::Working | AgentTurnState::Waiting => 0,
        AgentTurnState::Idle => 1,
        AgentTurnState::Unknown => 2,
        AgentTurnState::Completed | AgentTurnState::Failed | AgentTurnState::Interrupted => 3,
    }
}

fn command_center_task_usage(
    ui_text: UiText,
    source: &CommandCenterSource,
    execution: Option<&AgentExecutionRecord>,
) -> String {
    let Some(tokens) = execution.and_then(|record| record.telemetry.tokens) else {
        return ui_text
            .get(UiTextKey::CommandCenterTaskUsageUnknown)
            .to_string();
    };
    command_center_template(
        ui_text.get(if source.snapshot.provider_id.as_str() == "claude" {
            UiTextKey::CommandCenterTaskLastRequestUsage
        } else {
            UiTextKey::CommandCenterTaskExecutionUsage
        }),
        &[("value", &command_center_compact_count(tokens))],
    )
}

fn command_center_question_key(terminal_session_id: &str, request_id: &str) -> String {
    format!(
        "question:{}:{terminal_session_id}:{request_id}",
        terminal_session_id.len()
    )
}

fn command_center_now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default()
}

fn command_center_execution_depth(
    execution: &AgentExecutionRecord,
    records: &[AgentExecutionRecord],
) -> usize {
    let mut parent = execution.parent_id.as_deref();
    let mut depth = 0;
    // Indentation is capped, including malformed or partially retained parent chains.
    while let Some(id) = parent {
        depth += 1;
        if depth == 4 {
            break;
        }
        parent = records
            .iter()
            .find(|record| record.id == id)
            .and_then(|record| record.parent_id.as_deref());
    }
    depth
}

fn command_center_compact_count(value: u64) -> String {
    if value >= 1_000_000 {
        format!("{:.1}M", value as f64 / 1_000_000.0)
    } else if value >= 1_000 {
        format!("{:.1}k", value as f64 / 1_000.0)
    } else {
        value.to_string()
    }
}

fn command_center_cost(microusd: u64) -> String {
    if microusd.is_multiple_of(1_000_000) {
        return format!("${}", microusd / 1_000_000);
    }
    format!("${}.{:06}", microusd / 1_000_000, microusd % 1_000_000)
        .trim_end_matches('0')
        .to_string()
}

fn command_center_telemetry_line(
    ui_text: UiText,
    telemetry: &AgentExecutionTelemetry,
    last_request: bool,
    theme: WorkbenchTheme,
) -> Option<Div> {
    if telemetry.model.is_none()
        && telemetry.tokens.is_none()
        && telemetry.self_estimated_completion.is_none()
        && telemetry.cost_microusd.is_none()
    {
        return None;
    }
    let mut row = div()
        .flex()
        .flex_wrap()
        .items_center()
        .min_w_0()
        .gap(px(5.0))
        .text_xs()
        .text_color(theme.text_subtle);
    if let Some(model) = &telemetry.model {
        row = row.child(div().min_w_0().flex_1().truncate().child(model.clone()));
    }
    if let Some(tokens) = telemetry.tokens {
        row = row.child(
            div()
                .flex_none()
                .whitespace_nowrap()
                .child(command_center_template(
                    ui_text.get(if last_request {
                        UiTextKey::CommandCenterLastRequestTokens
                    } else {
                        UiTextKey::CommandCenterTokenCount
                    }),
                    &[("value", &command_center_compact_count(tokens))],
                )),
        );
    }
    if let Some(estimate) = telemetry.self_estimated_completion {
        row = row.child(
            div()
                .flex_none()
                .whitespace_nowrap()
                .text_color(theme.accent)
                .child(command_center_template(
                    ui_text.get(UiTextKey::CommandCenterSelfEstimate),
                    &[("value", &estimate.to_string())],
                )),
        );
    } else if let Some(cost) = telemetry.cost_microusd {
        row = row.child(
            div()
                .flex_none()
                .whitespace_nowrap()
                .child(command_center_cost(cost)),
        );
    }
    Some(row)
}

fn command_center_execution_details(
    ui_text: UiText,
    source: &CommandCenterSource,
    execution: &AgentExecutionRecord,
) -> String {
    let mut details = execution.name.clone();
    if let Some(task) = &execution.task
        && task != &execution.name
    {
        let _ = write!(details, "\n{task}");
    }
    if let Some(parent) = execution.parent_id.as_deref().and_then(|id| {
        source
            .snapshot
            .board
            .executions
            .iter()
            .find(|record| record.id == id)
    }) {
        let _ = write!(
            details,
            "\n{}: {}",
            ui_text.get(UiTextKey::CommandCenterParentExecution),
            parent.name
        );
    }
    if let Some(action) = &execution.current_action {
        let _ = write!(
            details,
            "\n{}: {action}",
            ui_text.get(UiTextKey::CommandCenterCurrentAction)
        );
    }
    let telemetry = &execution.telemetry;
    if let Some(model) = &telemetry.model {
        let _ = write!(
            details,
            "\n{}: {model}",
            ui_text.get(UiTextKey::CommandCenterModel)
        );
    }
    if source.snapshot.provider_id.as_str() == "claude"
        && telemetry
            .tokens
            .or(telemetry.input_tokens)
            .or(telemetry.output_tokens)
            .or(telemetry.cache_read_tokens)
            .or(telemetry.cache_write_tokens)
            .is_some()
    {
        let _ = write!(
            details,
            "\n{}",
            ui_text.get(UiTextKey::CommandCenterLastRequestTokensHint)
        );
    }
    if let Some(tokens) = telemetry.tokens {
        let _ = write!(
            details,
            "\n{}",
            command_center_template(
                ui_text.get(UiTextKey::CommandCenterTokenCount),
                &[("value", &tokens.to_string())],
            )
        );
    }
    for (key, value) in [
        (UiTextKey::CommandCenterInputTokens, telemetry.input_tokens),
        (
            UiTextKey::CommandCenterOutputTokens,
            telemetry.output_tokens,
        ),
        (
            UiTextKey::CommandCenterCacheReadTokens,
            telemetry.cache_read_tokens,
        ),
        (
            UiTextKey::CommandCenterCacheWriteTokens,
            telemetry.cache_write_tokens,
        ),
        (UiTextKey::CommandCenterRequests, telemetry.requests),
        (UiTextKey::CommandCenterToolCalls, telemetry.tool_calls),
    ] {
        if let Some(value) = value {
            let _ = write!(details, "\n{}: {value}", ui_text.get(key));
        }
    }
    if let Some(tokens) = telemetry.context_tokens {
        let _ = write!(
            details,
            "\n{}: {tokens}",
            ui_text.get(UiTextKey::CommandCenterContextTokens)
        );
        if let Some(window) = telemetry.context_window {
            let _ = write!(details, " / {window}");
        }
    }
    if let Some(duration) = telemetry.duration_ms {
        let _ = write!(
            details,
            "\n{}: {}",
            ui_text.get(UiTextKey::CommandCenterReportedDuration),
            command_center_format_duration(ui_text, duration)
        );
    }
    if let Some(cost) = telemetry.cost_microusd {
        let _ = write!(
            details,
            "\n{}: {}",
            ui_text.get(UiTextKey::CommandCenterCost),
            command_center_cost(cost)
        );
    }
    if let Some(estimate) = telemetry.self_estimated_completion {
        let _ = write!(
            details,
            "\n{}\n{}",
            command_center_template(
                ui_text.get(UiTextKey::CommandCenterSelfEstimate),
                &[("value", &estimate.to_string())],
            ),
            ui_text.get(UiTextKey::CommandCenterSelfEstimateHint)
        );
    }
    if let Some(observed_at) = telemetry.observed_at.filter(|timestamp| *timestamp > 0) {
        let _ = write!(
            details,
            "\n{}",
            command_center_template(
                ui_text.get(UiTextKey::CommandCenterTelemetryObserved),
                &[("value", &command_center_age(ui_text, observed_at))],
            )
        );
    }
    details
}

fn command_center_duration(ui_text: UiText, started_at: u64, ended_at: Option<u64>) -> String {
    if started_at == 0 {
        return "—".to_string();
    }
    let millis = ended_at
        .unwrap_or_else(command_center_now_millis)
        .saturating_sub(started_at);
    command_center_format_duration(ui_text, millis)
}

fn command_center_format_duration(ui_text: UiText, millis: u64) -> String {
    let seconds = millis / 1_000;
    let (key, value) = if seconds >= 3_600 {
        (
            UiTextKey::CommandCenterDurationHours,
            (seconds / 3_600).to_string(),
        )
    } else if seconds >= 60 {
        (
            UiTextKey::CommandCenterDurationMinutes,
            (seconds / 60).to_string(),
        )
    } else {
        (UiTextKey::CommandCenterDurationSeconds, seconds.to_string())
    };
    command_center_template(ui_text.get(key), &[("value", &value)])
}

fn command_center_age(ui_text: UiText, timestamp: u64) -> String {
    if timestamp == 0 {
        return "—".to_string();
    }
    command_center_duration(ui_text, timestamp, Some(command_center_now_millis()))
}

fn command_center_time(ui_text: UiText, timestamp: u64) -> String {
    if timestamp == 0 {
        return "—".to_string();
    }
    command_center_age(ui_text, timestamp)
}

fn command_center_plan_label(status: AgentPlanStatus) -> UiTextKey {
    match status {
        AgentPlanStatus::Pending => UiTextKey::CommandCenterPlanPending,
        AgentPlanStatus::InProgress => UiTextKey::CommandCenterPlanWorking,
        AgentPlanStatus::Completed => UiTextKey::CommandCenterPlanCompleted,
        AgentPlanStatus::Blocked => UiTextKey::CommandCenterPlanBlocked,
        AgentPlanStatus::Abandoned => UiTextKey::CommandCenterPlanAbandoned,
        AgentPlanStatus::Unknown => UiTextKey::CommandCenterPlanUnknown,
    }
}

fn command_center_plan_symbol(status: AgentPlanStatus) -> &'static str {
    match status {
        AgentPlanStatus::Pending => "○",
        AgentPlanStatus::InProgress => "●",
        AgentPlanStatus::Completed => "✓",
        AgentPlanStatus::Blocked => "?",
        AgentPlanStatus::Abandoned => "—",
        AgentPlanStatus::Unknown => "◇",
    }
}

fn command_center_plan_color(status: AgentPlanStatus, theme: WorkbenchTheme) -> gpui::Rgba {
    match status {
        AgentPlanStatus::InProgress => theme.accent,
        AgentPlanStatus::Completed => theme.success,
        AgentPlanStatus::Blocked => theme.warning,
        AgentPlanStatus::Abandoned => theme.text_disabled,
        AgentPlanStatus::Unknown => theme.text_muted,
        AgentPlanStatus::Pending => theme.text_subtle,
    }
}

fn command_center_turn_label(state: AgentTurnState) -> UiTextKey {
    match state {
        AgentTurnState::Idle => UiTextKey::CommandCenterTurnIdle,
        AgentTurnState::Working => UiTextKey::CommandCenterTurnWorking,
        AgentTurnState::Waiting => UiTextKey::CommandCenterTurnWaiting,
        AgentTurnState::Completed => UiTextKey::CommandCenterTurnCompleted,
        AgentTurnState::Failed => UiTextKey::CommandCenterTurnFailed,
        AgentTurnState::Interrupted => UiTextKey::CommandCenterTurnInterrupted,
        AgentTurnState::Unknown => UiTextKey::CommandCenterTurnUnknown,
    }
}

fn command_center_turn_symbol(state: AgentTurnState) -> &'static str {
    match state {
        AgentTurnState::Idle => "○",
        AgentTurnState::Working => "●",
        AgentTurnState::Waiting => "?",
        AgentTurnState::Completed => "✓",
        AgentTurnState::Failed => "!",
        AgentTurnState::Interrupted => "—",
        AgentTurnState::Unknown => "◇",
    }
}

fn command_center_turn_color(state: AgentTurnState, theme: WorkbenchTheme) -> gpui::Rgba {
    match state {
        AgentTurnState::Working => theme.accent,
        AgentTurnState::Waiting => theme.warning,
        AgentTurnState::Completed => theme.success,
        AgentTurnState::Failed => theme.danger,
        AgentTurnState::Interrupted | AgentTurnState::Unknown => theme.text_muted,
        AgentTurnState::Idle => theme.text_subtle,
    }
}

fn command_center_question_label(state: AgentQuestionState) -> UiTextKey {
    match state {
        AgentQuestionState::Pending => UiTextKey::CommandCenterQuestionAwaitingAnswer,
        AgentQuestionState::Submitted => UiTextKey::CommandCenterQuestionQueued,
        AgentQuestionState::Answered => UiTextKey::CommandCenterQuestionAnsweredStatus,
        AgentQuestionState::Cancelled => UiTextKey::CommandCenterQuestionCancelledStatus,
        AgentQuestionState::Unavailable => UiTextKey::CommandCenterQuestionUnavailableStatus,
    }
}

fn command_center_question_color(state: AgentQuestionState, theme: WorkbenchTheme) -> gpui::Rgba {
    match state {
        AgentQuestionState::Pending | AgentQuestionState::Submitted => theme.warning,
        AgentQuestionState::Answered => theme.success,
        AgentQuestionState::Cancelled => theme.text_muted,
        AgentQuestionState::Unavailable => theme.danger,
    }
}

fn command_center_question_is_resolved(state: AgentQuestionState) -> bool {
    matches!(
        state,
        AgentQuestionState::Answered | AgentQuestionState::Cancelled
    )
}

fn command_center_answers_text(record: &AgentQuestionRecord) -> String {
    if record.answers.is_empty() {
        return String::new();
    }
    let values = record
        .answers
        .iter()
        .map(|answer| {
            let selected = answer.selected_options.join(" / ");
            match (&selected[..], answer.custom_input.as_deref()) {
                ("", Some(custom)) => custom.to_string(),
                (_, Some(custom)) => format!("{selected}; {custom}"),
                _ => selected,
            }
        })
        .collect::<Vec<_>>();
    format!(": {}", values.join(" · "))
}

fn command_center_event_requires_refresh(event: &ClientEvent) -> bool {
    match event {
        ClientEvent::Connection(_)
        | ClientEvent::AgentSnapshotUpdated(_)
        | ClientEvent::ResourceCatalogUpdated(_)
        | ClientEvent::TerminalUnavailable(_) => true,
        ClientEvent::Server(event) => matches!(
            event.body,
            yttt_protocol::ServerEvent::ProfileControl(_)
                | yttt_protocol::ServerEvent::TerminalExit { .. }
                | yttt_protocol::ServerEvent::TerminalLeaseRevoked { .. }
                | yttt_protocol::ServerEvent::HostDraining
                | yttt_protocol::ServerEvent::HostStopping
        ),
        ClientEvent::TerminalUpdated(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_presence_distinguishes_finished_turns_exit_and_lost_observation() {
        let (_, view, _) = session_jump_fixture();
        let mut snapshot = view.command_center_sources().pop().unwrap().snapshot;
        snapshot.process_state = AgentProcessState::Running;
        snapshot.turn_state = AgentTurnState::Completed;
        assert_eq!(
            command_center_agent_status(&snapshot, true).0,
            UiTextKey::CommandCenterAgentOnlineCompleted,
        );
        assert_eq!(
            command_center_agent_status(&snapshot, false).0,
            UiTextKey::CommandCenterAgentRetained,
        );
        snapshot.process_state = AgentProcessState::Exited;
        snapshot.process_exit = Some(yttt_agent_core::AgentProcessExit {
            code: None,
            reason: AgentExitReason::Disconnected,
        });
        assert_eq!(
            command_center_agent_status(&snapshot, true).0,
            UiTextKey::CommandCenterAgentDisconnected,
        );
        snapshot.process_exit = Some(yttt_agent_core::AgentProcessExit {
            code: Some(0),
            reason: AgentExitReason::Completed,
        });
        assert_eq!(
            command_center_agent_status(&snapshot, true).0,
            UiTextKey::CommandCenterAgentExited,
        );
    }

    #[gpui::test]
    fn completed_shell_agents_remain_selectable_outside_collapsed_history(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(gpui_component::init);
        let (_temp, mut view, target) = session_jump_fixture();
        let mut snapshot = view.command_center_sources().pop().unwrap().snapshot;
        snapshot.process_state = AgentProcessState::Running;
        snapshot.turn_state = AgentTurnState::Completed;
        snapshot.board.executions[0].parent_id = None;
        snapshot.board.tasks = vec![usage_task("done", AgentPlanStatus::Completed)];
        for pane in [&target.pane_id, "shell"] {
            view.workspace
                .record_agent_snapshot(&target.project_id, &target.tab_id, pane, snapshot.clone())
                .unwrap();
        }
        snapshot.mark_disconnected();
        snapshot.board = Default::default();
        view.workspace
            .record_agent_snapshot(&target.project_id, "other", "agent:pane", snapshot)
            .unwrap();
        let slot = Rc::new(std::cell::RefCell::new(None));
        let output = slot.clone();
        let (_, cx) = cx.add_window_view(|window, cx| {
            let root = cx.new(|_| view);
            *output.borrow_mut() = Some(root.clone());
            ComponentRoot::new(root, window, cx)
        });
        let root = slot.borrow_mut().take().unwrap();
        cx.run_until_parked();
        cx.refresh().unwrap();
        let history = cx
            .debug_bounds("command-center-project-history:target:project")
            .unwrap();
        let sessions = [
            (
                "target:project:conversation:one:agent:pane",
                "command-center-session:target:project:conversation:one:agent:pane:0",
                "command-center-task:target:project:conversation:one:agent:pane:child-execution:done",
            ),
            (
                "target:project:conversation:one:shell",
                "command-center-session:target:project:conversation:one:shell:0",
                "command-center-task:target:project:conversation:one:shell:child-execution:done",
            ),
        ];
        for (session_id, row_selector, task_selector) in sessions {
            let row = cx
                .debug_bounds(row_selector)
                .expect("a running agent must remain visible after its last task finishes");
            assert!(row.bottom() <= history.top());
            assert!(cx.debug_bounds(task_selector).is_none());
            cx.simulate_click(row.center(), gpui::Modifiers::none());
            cx.run_until_parked();
            root.read_with(cx, |view, _| {
                let selection = view.command_center.selection.as_ref().unwrap();
                assert_eq!(selection.terminal_session_id, session_id);
                assert_eq!(selection.execution_id, "child-execution");
                assert!(selection.task_id.is_none());
            });
        }
        assert!(
            cx.debug_bounds("command-center-session:target:project:other:agent:pane:0")
                .is_none()
        );
        cx.simulate_click(history.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        cx.refresh().unwrap();
        assert!(
            cx.debug_bounds("command-center-session:target:project:other:agent:pane:3")
                .is_some()
        );
        for (_, row_selector, task_selector) in sessions {
            assert!(cx.debug_bounds(row_selector).is_some());
            assert!(cx.debug_bounds(task_selector).is_some());
        }
    }

    fn usage_task(id: &str, status: AgentPlanStatus) -> AgentBoardTask {
        AgentBoardTask {
            item: yttt_agent_core::AgentPlanItem {
                id: id.into(),
                title: id.into(),
                phase: None,
                status,
                blocker: None,
            },
            execution_id: "child-execution".into(),
            created_at: 1,
            updated_at: 2,
            accepted_at: None,
            retired: false,
        }
    }
    #[gpui::test]
    fn root_plan_keeps_child_usage_visible_in_project_card(cx: &mut gpui::TestAppContext) {
        cx.update(gpui_component::init);
        let (_temp, mut view, target) = session_jump_fixture();
        let source = view.command_center_sources().pop().unwrap();
        let mut snapshot = source.snapshot;
        let child = &mut snapshot.board.executions[0];
        child.state = AgentTurnState::Working;
        child.ended_at = None;
        child.telemetry.tokens = Some(48_325);
        let mut root_execution = child.clone();
        root_execution.id = "root-execution".into();
        root_execution.parent_id = None;
        root_execution.telemetry = Default::default();
        snapshot.board.apply(
            yttt_agent_core::AgentBoardEvent::Execution {
                execution: root_execution,
            },
            3,
        );
        snapshot.board.apply(
            yttt_agent_core::AgentBoardEvent::Plan {
                execution_id: "root-execution".into(),
                items: vec![usage_task("root-todo", AgentPlanStatus::InProgress).item],
            },
            4,
        );
        view.workspace
            .record_agent_snapshot(
                &target.project_id,
                &target.tab_id,
                &target.pane_id,
                snapshot,
            )
            .unwrap();
        let slot = Rc::new(std::cell::RefCell::new(None));
        let output = slot.clone();
        let (_, cx) = cx.add_window_view(|window, cx| {
            let root = cx.new(|_| view);
            *output.borrow_mut() = Some(root.clone());
            ComponentRoot::new(root, window, cx)
        });
        let root = slot.borrow_mut().take().unwrap();
        cx.run_until_parked();
        cx.refresh().unwrap();
        let usage = cx.debug_bounds(
            "command-center-project-usage:target:project:conversation:one:agent:pane:child-execution",
        ).expect("OMP child usage must be shown independently of root Todo attribution");
        let task = cx.debug_bounds(
            "command-center-task:target:project:conversation:one:agent:pane:root-execution:root-todo",
        ).unwrap();
        assert!(usage.origin.y >= task.bottom());
        cx.simulate_click(usage.center(), gpui::Modifiers::default());
        root.read_with(cx, |view, _| {
            let selected = view.command_center.selection.as_ref().unwrap();
            assert_eq!(selected.execution_id, "child-execution");
            assert!(selected.task_id.is_none());
        });
        root.update(cx, |view, cx| {
            let mut snapshot = view.command_center_sources().pop().unwrap().snapshot;
            snapshot.board.tasks[0].item.status = AgentPlanStatus::Pending;
            let child = snapshot
                .board
                .executions
                .iter_mut()
                .find(|execution| execution.id == "child-execution")
                .unwrap();
            child.state = AgentTurnState::Completed;
            child.ended_at = Some(5);
            child.updated_at = 5;
            view.workspace
                .record_agent_snapshot(
                    &target.project_id,
                    &target.tab_id,
                    &target.pane_id,
                    snapshot,
                )
                .unwrap();
            cx.notify();
        });
        cx.run_until_parked();
        cx.refresh().unwrap();
        assert!(cx.debug_bounds(
            "command-center-project-usage:target:project:conversation:one:agent:pane:child-execution",
        ).is_none());
        let history = cx
            .debug_bounds("command-center-project-history:target:project")
            .unwrap();
        cx.simulate_click(history.center(), gpui::Modifiers::default());
        cx.run_until_parked();
        cx.refresh().unwrap();
        let completed = cx.debug_bounds(
            "command-center-project-usage:target:project:conversation:one:agent:pane:child-execution",
        ).unwrap();
        let pending = cx.debug_bounds(
            "command-center-task:target:project:conversation:one:agent:pane:root-execution:root-todo",
        ).unwrap();
        assert!(completed.origin.y >= pending.bottom());
    }

    #[test]
    fn execution_usage_without_plans_preserves_zero_and_missing_measurements() {
        let (_temp, view, _) = session_jump_fixture();
        let mut source = view.command_center_sources().pop().unwrap();
        assert!(source.snapshot.board.tasks.is_empty());
        assert!(
            CommandCenterProjectUsage::new(view.ui_text, &source)
                .independent
                .is_empty()
        );
        source.snapshot.board.executions[0].telemetry.tokens = Some(0);
        let usage = CommandCenterProjectUsage::new(view.ui_text, &source);
        assert_eq!(
            usage
                .independent
                .iter()
                .map(|record| record.id.as_str())
                .collect::<Vec<_>>(),
            ["child-execution"]
        );
        assert!(usage.task_labels.is_empty());
    }

    #[test]
    fn task_usage_never_falls_back_to_an_unrelated_execution() {
        let (_temp, view, _) = session_jump_fixture();
        let mut source = view.command_center_sources().pop().unwrap();
        let task = usage_task("task", AgentPlanStatus::InProgress);
        source.snapshot.board.executions[0].id = "unrelated-run".into();
        source.snapshot.board.executions[0].telemetry.tokens = Some(100_000);
        source.snapshot.board.tasks.push(task);
        let usage = CommandCenterProjectUsage::new(view.ui_text, &source);
        assert_eq!(
            usage.task_labels["child-execution"].as_ref(),
            view.ui_text.get(UiTextKey::CommandCenterTaskUsageUnknown)
        );
        assert_eq!(usage.independent[0].id, "unrelated-run");
    }

    #[gpui::test]
    fn task_rows_keep_active_and_pending_before_completed(cx: &mut gpui::TestAppContext) {
        cx.update(gpui_component::init);
        let (_temp, mut view, target) = session_jump_fixture();
        let source = view.command_center_sources().pop().unwrap();
        let mut snapshot = source.snapshot;
        snapshot.board.tasks = vec![
            usage_task("completed", AgentPlanStatus::Completed),
            usage_task("pending", AgentPlanStatus::Pending),
            usage_task("working", AgentPlanStatus::InProgress),
        ];
        view.workspace
            .record_agent_snapshot(
                &target.project_id,
                &target.tab_id,
                &target.pane_id,
                snapshot,
            )
            .unwrap();
        view.command_center
            .project_history_expanded
            .insert(target.project_id.to_string());
        let (_, cx) = cx.add_window_view(|window, cx| {
            let root = cx.new(|_| view);
            ComponentRoot::new(root, window, cx)
        });
        cx.run_until_parked();
        cx.refresh().unwrap();
        let working = cx.debug_bounds("command-center-task:target:project:conversation:one:agent:pane:child-execution:working").unwrap();
        let pending = cx.debug_bounds("command-center-task:target:project:conversation:one:agent:pane:child-execution:pending").unwrap();
        let completed = cx.debug_bounds("command-center-task:target:project:conversation:one:agent:pane:child-execution:completed").unwrap();
        assert!(working.origin.y < pending.origin.y);
        assert!(pending.origin.y < completed.origin.y);
    }

    fn session_jump_fixture() -> (tempfile::TempDir, WorkbenchView, CommandCenterSessionTarget) {
        use crate::model::project::{ProjectDescriptor, ProjectLocation};
        use yttt_agent_core::{AgentInstanceId, AgentReducer, ProviderId};

        let temp = tempfile::tempdir().unwrap();
        let layout: ProjectLayout = toml::from_str(
            r#"
            [project]
            name = "Session jump"
            default_tab = "other"
            [[tabs]]
            id = "other"
            title = "Other"
            layout = { type = "pane", id = "agent:pane", title = "Other", command = "" }
            [[tabs]]
            id = "conversation:one"
            title = "Conversation"
            [tabs.layout]
            type = "split"
            direction = "horizontal"
            ratio = 0.5
            left = { type = "pane", id = "shell", title = "Shell", command = "" }
            right = { type = "pane", id = "agent:pane", title = "Agent", command = "" }
        "#,
        )
        .unwrap();
        let mut workspace = Workspace::new();
        for id in ["target:project", "other:project"] {
            let path = temp.path().join(id.replace(':', "-"));
            std::fs::create_dir(&path).unwrap();
            workspace
                .open_project(
                    ProjectDescriptor::new(ProjectId::new(id), ProjectLocation::local(path)),
                    layout.clone(),
                )
                .unwrap();
        }
        let mut snapshot = AgentReducer::new(
            AgentInstanceId::new("session-agent").unwrap(),
            ProviderId::from_static("omp"),
            1,
        )
        .snapshot()
        .clone();
        snapshot.board.executions.push(AgentExecutionRecord {
            id: "child-execution".into(),
            parent_id: Some("root-execution".into()),
            name: "Child agent".into(),
            task: None,
            state: AgentTurnState::Completed,
            started_at: 1,
            updated_at: 2,
            ended_at: Some(2),
            current_action: None,
            telemetry: Default::default(),
        });
        workspace
            .record_agent_snapshot(
                &ProjectId::new("target:project"),
                "conversation:one",
                "agent:pane",
                snapshot,
            )
            .unwrap();
        let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
        let mut root = WorkbenchView::with_workspace_for_test_and_config_paths(workspace, paths);
        let scope = root
            .command_center_sources()
            .pop()
            .unwrap()
            .session_target
            .unwrap();
        root.select_command_center();
        (temp, root, scope)
    }

    fn question_target(request_id: &str, question_id: &str) -> CommandCenterQuestionTarget {
        CommandCenterQuestionTarget {
            terminal_session_id: "same-agent".into(),
            execution_id: "run-1".into(),
            request_id: request_id.into(),
            question_id: question_id.into(),
        }
    }

    fn question_record(
        request_id: &str,
        question_id: &str,
        state: AgentQuestionState,
        answerable: bool,
    ) -> AgentQuestionRecord {
        use yttt_agent_core::{AgentQuestion, AgentQuestionOption, AgentQuestionRequest};

        AgentQuestionRecord {
            request: AgentQuestionRequest {
                id: request_id.into(),
                execution_id: "run-1".into(),
                tool_call_id: format!("tool-{request_id}"),
                questions: vec![AgentQuestion {
                    id: question_id.into(),
                    question: format!("Question {question_id}"),
                    header: None,
                    options: vec![AgentQuestionOption {
                        label: "Yes".into(),
                        description: None,
                        preview: None,
                    }],
                    multi: false,
                    recommended: None,
                }],
                answerable,
            },
            state,
            answers: Vec::new(),
            answer_source: None,
            created_at: 1,
            updated_at: 1,
        }
    }

    fn question_source(records: Vec<AgentQuestionRecord>) -> CommandCenterSource {
        use yttt_agent_core::{AgentInstanceId, AgentReducer, ProviderId};

        let mut snapshot = AgentReducer::new(
            AgentInstanceId::new("same-agent").unwrap(),
            ProviderId::from_static("omp"),
            1,
        )
        .snapshot()
        .clone();
        snapshot.board.questions = records;
        CommandCenterSource {
            scope: AgentHookScope {
                project_id: "project".into(),
                tab_id: "tab".into(),
                pane_id: "pane".into(),
                generation: 1,
            },
            session_target: None,
            project_name: "Project".into(),
            terminal_session_id: "same-agent".into(),
            snapshot,
            operation_target: None,
        }
    }

    #[test]
    fn closed_or_unanswerable_questions_cannot_be_submitted() {
        let temp = tempfile::tempdir().unwrap();
        let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
        let root = WorkbenchView::with_config_paths_for_test(paths);
        let answers = vec![AgentQuestionAnswer {
            question_id: "choice".into(),
            selected_options: vec!["Yes".into()],
            custom_input: None,
        }];
        let closed = question_source(vec![question_record(
            "request-closed",
            "choice",
            AgentQuestionState::Answered,
            true,
        )]);
        assert!(
            root.command_center_validate_answer(&closed, "request-closed", &answers)
                .is_err()
        );
        let unanswerable = question_source(vec![question_record(
            "request-unanswerable",
            "choice",
            AgentQuestionState::Pending,
            false,
        )]);
        assert!(
            root.command_center_validate_answer(&unanswerable, "request-unanswerable", &answers)
                .is_err()
        );
    }

    #[test]
    fn host_terminal_scopes_resolve_only_through_workspace_session_placements() {
        let (_temp, root, target) = session_jump_fixture();
        let source = root.command_center_sources().pop().unwrap();
        // Both retained and newly reported panes must resolve; closed panes must not.
        for pane_id in ["agent:pane", "shell", "closed"] {
            let session_id = terminal_pane_key(target.project_id.as_str(), &target.tab_id, pane_id);
            let host_scope = AgentHookScope {
                project_id: target.project_id.to_string(),
                tab_id: session_id.clone(),
                pane_id: session_id.clone(),
                generation: 19,
            };
            let mut snapshot = source.snapshot.clone();
            snapshot.generation = 19;
            let sources = root.command_center_sources_with_updates(vec![AgentSnapshotUpdate {
                scope: host_scope.clone(),
                terminal_session_id: TerminalSessionId::new(&session_id),
                host_epoch: 7,
                sequence: 1,
                snapshot,
            }]);
            let live = sources
                .iter()
                .find(|source| source.terminal_session_id == session_id)
                .unwrap();
            assert_eq!(
                live.scope, host_scope,
                "Host command fencing must retain its opaque IDs"
            );
            if pane_id == "closed" {
                assert!(live.session_target.is_none());
            } else {
                assert_eq!(
                    live.session_target,
                    Some(CommandCenterSessionTarget {
                        project_id: target.project_id.clone(),
                        tab_id: target.tab_id.clone(),
                        pane_id: pane_id.to_string(),
                    })
                );
            }
        }
    }

    #[gpui::test]
    fn option_selection_keeps_same_agent_questions_disambiguated(cx: &mut gpui::TestAppContext) {
        let temp = tempfile::tempdir().unwrap();
        let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
        let root = cx.new(|_| WorkbenchView::with_config_paths_for_test(paths));
        root.update(cx, |root, cx| {
            let first = question_target("request-a", "choice-a");
            let second = question_target("request-b", "choice-b");
            root.command_center_select_option(
                first,
                "draft-a",
                "choice-a",
                "Yes".into(),
                false,
                cx,
            );
            root.command_center_select_option(
                second,
                "draft-b",
                "choice-b",
                "Yes".into(),
                false,
                cx,
            );
            let selection = root.command_center.selection.as_ref().unwrap();
            assert_eq!(selection.terminal_session_id, "same-agent");
            assert_eq!(selection.question_request_id.as_deref(), Some("request-b"));
            assert_eq!(selection.question_id.as_deref(), Some("choice-b"));
            assert!(
                root.command_center.question_drafts["draft-a"].selected_options["choice-a"]
                    .contains("Yes")
            );
            assert!(
                root.command_center.question_drafts["draft-b"].selected_options["choice-b"]
                    .contains("Yes")
            );
        });
    }

    #[gpui::test]
    fn input_focus_selects_its_question_without_disabling_typing_or_keyboard_text_safety(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(gpui_component::init);
        let (_temp, mut view, placement) = session_jump_fixture();
        let source = view.command_center_sources().pop().unwrap();
        let session_id = source.terminal_session_id;
        let mut snapshot = source.snapshot;
        snapshot.board.questions = vec![
            question_record("request-a", "choice-a", AgentQuestionState::Pending, true),
            question_record("request-b", "choice-b", AgentQuestionState::Pending, true),
        ];
        view.workspace
            .record_agent_snapshot(
                &placement.project_id,
                &placement.tab_id,
                &placement.pane_id,
                snapshot,
            )
            .unwrap();
        let slot = Rc::new(std::cell::RefCell::new(None));
        let output = slot.clone();
        let (_, cx) = cx.add_window_view(|window, cx| {
            let root = cx.new(|_| view);
            *output.borrow_mut() = Some(root.clone());
            ComponentRoot::new(root, window, cx)
        });
        let root = slot.borrow_mut().take().unwrap();
        cx.run_until_parked();
        cx.refresh().unwrap();
        root.update_in(cx, |root, window, cx| {
            window.activate_window();
            root.command_center.question_scroll.scroll_to_item(1);
            cx.notify();
        });
        cx.run_until_parked();
        cx.refresh().unwrap();
        let input = root.update_in(cx, |root, window, cx| {
            assert!(window.is_window_active());
            let mut target = question_target("request-b", "choice-b");
            target.terminal_session_id = session_id.clone();
            let input = root.command_center_question_input(
                &command_center_question_key(&session_id, "request-b"),
                "choice-b",
                target,
                window,
                cx,
            );
            input.read(cx).focus_handle(cx).focus(window, cx);
            input
        });
        // The real InputState focus subscription must select B, not a manually invoked event.
        cx.run_until_parked();
        cx.refresh().unwrap();
        cx.run_until_parked();
        root.update_in(cx, |root, window, cx| {
            assert!(input.read(cx).focus_handle(cx).is_focused(window));
            assert_eq!(
                root.command_center
                    .selection
                    .as_ref()
                    .unwrap()
                    .question_request_id
                    .as_deref(),
                Some("request-b"),
            );
            input.update(cx, |input, cx| {
                input.set_value("2 remains editable", window, cx)
            });
            root.on_command_center_question_previous(&CommandCenterQuestionPrevious, window, cx);
            assert_eq!(input.read(cx).value().as_str(), "2 remains editable");
            assert_eq!(
                root.command_center
                    .selection
                    .as_ref()
                    .unwrap()
                    .question_id
                    .as_deref(),
                Some("choice-b"),
            );
        });
    }

    #[gpui::test]
    fn explicit_question_navigation_moves_numeric_shortcut_focus(cx: &mut gpui::TestAppContext) {
        cx.update(gpui_component::init);
        let (_temp, mut view, target) = session_jump_fixture();
        let source = view.command_center_sources().pop().unwrap();
        let session_id = source.terminal_session_id;
        let mut snapshot = source.snapshot;
        snapshot.board.questions = vec![
            question_record("request-a", "choice-a", AgentQuestionState::Pending, true),
            question_record("request-b", "choice-b", AgentQuestionState::Pending, true),
        ];
        view.workspace
            .record_agent_snapshot(
                &target.project_id,
                &target.tab_id,
                &target.pane_id,
                snapshot,
            )
            .unwrap();
        let slot = Rc::new(std::cell::RefCell::new(None));
        let output = slot.clone();
        let (_, cx) = cx.add_window_view(|window, cx| {
            let root = cx.new(|_| view);
            *output.borrow_mut() = Some(root.clone());
            ComponentRoot::new(root, window, cx)
        });
        let root = slot.borrow_mut().take().unwrap();
        root.update_in(cx, |root, window, cx| {
            let mut first = question_target("request-a", "choice-a");
            first.terminal_session_id = session_id.clone();
            root.command_center_select_question(first, Some("choice-a"), cx);
            let first_focus = root.command_center_question_focus(
                &command_center_question_key(&session_id, "request-a"),
                "choice-a",
                cx,
            );
            first_focus.focus(window, cx);
            root.on_command_center_question_next(&CommandCenterQuestionNext, window, cx);
            assert_eq!(
                root.command_center
                    .selection
                    .as_ref()
                    .unwrap()
                    .question_request_id
                    .as_deref(),
                Some("request-b"),
            );
            let second_focus = root.command_center_question_focus(
                &command_center_question_key(&session_id, "request-b"),
                "choice-b",
                cx,
            );
            assert!(
                second_focus.is_focused(window),
                "numeric keys must target the newly selected question"
            );
            assert!(!first_focus.is_focused(window));
        });
    }

    #[gpui::test]
    fn pending_question_navigation_includes_read_only_questions_without_submitting(
        cx: &mut gpui::TestAppContext,
    ) {
        let temp = tempfile::tempdir().unwrap();
        let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
        let root = cx.new(|_| WorkbenchView::with_config_paths_for_test(paths));
        let source = question_source(vec![
            question_record("request-a", "choice-a", AgentQuestionState::Pending, true),
            question_record("request-b", "choice-b", AgentQuestionState::Pending, true),
            question_record(
                "request-child",
                "choice-child",
                AgentQuestionState::Pending,
                false,
            ),
        ]);
        root.update(cx, |root, cx| {
            let first = question_target("request-a", "choice-a");
            root.command_center_select_question(first, Some("choice-a"), cx);
            root.command_center_move_selected_question_in_sources(
                std::slice::from_ref(&source),
                false,
                cx,
            );
            assert_eq!(
                root.command_center
                    .selection
                    .as_ref()
                    .unwrap()
                    .question_request_id
                    .as_deref(),
                Some("request-b")
            );
            root.command_center_move_selected_question_in_sources(
                std::slice::from_ref(&source),
                false,
                cx,
            );
            assert_eq!(
                root.command_center
                    .selection
                    .as_ref()
                    .unwrap()
                    .question_request_id
                    .as_deref(),
                Some("request-child")
            );
            root.command_center_move_selected_question_in_sources(
                std::slice::from_ref(&source),
                true,
                cx,
            );
            assert_eq!(
                root.command_center
                    .selection
                    .as_ref()
                    .unwrap()
                    .question_request_id
                    .as_deref(),
                Some("request-b")
            );
        });
    }

    #[gpui::test]
    fn resolved_selection_is_retained_until_explicit_pending_navigation(
        cx: &mut gpui::TestAppContext,
    ) {
        let temp = tempfile::tempdir().unwrap();
        let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
        let root = cx.new(|_| WorkbenchView::with_config_paths_for_test(paths));
        let source = question_source(vec![
            question_record("request-a", "choice-a", AgentQuestionState::Answered, true),
            question_record("request-b", "choice-b", AgentQuestionState::Pending, true),
        ]);
        root.update(cx, |root, cx| {
            let resolved = question_target("request-a", "choice-a");
            root.command_center_select_question(resolved, Some("choice-a"), cx);
            root.command_center_ensure_selection(std::slice::from_ref(&source));
            assert!(!root.command_center.question_history_expanded);
            assert_eq!(
                root.command_center
                    .selection
                    .as_ref()
                    .unwrap()
                    .question_request_id
                    .as_deref(),
                Some("request-a")
            );
            root.command_center_move_selected_question_in_sources(
                std::slice::from_ref(&source),
                false,
                cx,
            );
            assert_eq!(
                root.command_center
                    .selection
                    .as_ref()
                    .unwrap()
                    .question_request_id
                    .as_deref(),
                Some("request-b")
            );
        });
    }

    #[gpui::test]
    fn execution_history_stays_available_without_displacing_current_activity(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(gpui_component::init);
        let (_temp, mut view, target) = session_jump_fixture();
        let source = view.command_center_sources().pop().unwrap();
        let session_id = source.terminal_session_id.clone();
        let mut snapshot = source.snapshot;
        snapshot.board.executions.push(AgentExecutionRecord {
            id: "running".into(),
            parent_id: None,
            name: "Current work".into(),
            task: None,
            state: AgentTurnState::Working,
            started_at: 3,
            updated_at: 3,
            ended_at: None,
            current_action: None,
            telemetry: Default::default(),
        });
        view.workspace
            .record_agent_snapshot(
                &target.project_id,
                &target.tab_id,
                &target.pane_id,
                snapshot,
            )
            .unwrap();
        let slot = Rc::new(std::cell::RefCell::new(None));
        let output = slot.clone();
        let (_, cx) = cx.add_window_view(|window, cx| {
            let root = cx.new(|_| view);
            *output.borrow_mut() = Some(root.clone());
            ComponentRoot::new(root, window, cx)
        });
        let root = slot.borrow_mut().take().unwrap();
        root.update_in(cx, |root, _, cx| {
            root.command_center_select(session_id.clone(), "running".into(), None, cx);
        });
        cx.run_until_parked();
        cx.refresh().unwrap();
        assert!(
            cx.debug_bounds("command-center-execution-row:running")
                .is_some()
        );
        assert!(
            cx.debug_bounds("command-center-execution-row:child-execution")
                .is_none()
        );
        let history = cx.debug_bounds("command-center-execution-history").unwrap();
        cx.simulate_click(history.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        cx.refresh().unwrap();
        let child = cx
            .debug_bounds("command-center-execution-row:child-execution")
            .unwrap();
        cx.simulate_click(child.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        cx.refresh().unwrap();
        let history = cx.debug_bounds("command-center-execution-history").unwrap();
        cx.simulate_click(history.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        cx.refresh().unwrap();
        assert!(
            cx.debug_bounds("command-center-execution-row:child-execution")
                .is_some()
        );
        root.update_in(cx, |root, _, cx| {
            assert!(!root.command_center.execution_history_expanded);
            assert_eq!(
                root.command_center.selection.as_ref().unwrap().execution_id,
                "child-execution",
            );
            root.command_center_select(session_id, "running".into(), None, cx);
        });
        cx.run_until_parked();
        cx.refresh().unwrap();
        assert!(
            cx.debug_bounds("command-center-execution-row:child-execution")
                .is_none()
        );
        assert!(
            cx.debug_bounds("command-center-execution-row:running")
                .is_some()
        );
    }

    #[gpui::test]
    fn explicit_session_link_focuses_exact_hosting_pane_and_preserves_drafts(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(gpui_component::init);
        let (_temp, view, scope) = session_jump_fixture();
        let slot = Rc::new(std::cell::RefCell::new(None));
        let output = slot.clone();
        let (_, cx) = cx.add_window_view(|window, cx| {
            let root = cx.new(|_| view);
            *output.borrow_mut() = Some(root.clone());
            ComponentRoot::new(root, window, cx)
        });
        let root = slot.borrow_mut().take().unwrap();
        root.update_in(cx, |root, window, cx| {
            let source = root.command_center_sources().pop().unwrap();
            let answer = root.command_center_question_input(
                "request",
                "choice",
                question_target("request", "choice"),
                window,
                cx,
            );
            answer.update(cx, |input, cx| input.set_value("Answer draft", window, cx));
            let prompt = root.command_center_composer_input("follow-up", window, cx);
            prompt.update(cx, |input, cx| input.set_value("Prompt draft", window, cx));
            root.command_center_select(
                source.terminal_session_id,
                "child-execution".into(),
                None,
                cx,
            );
            assert!(root.command_center_is_active());
            assert!(root.workspace.selected_project_id().is_none());
        });
        cx.run_until_parked();
        cx.refresh().unwrap();
        let button = cx.debug_bounds("command-center-open-session").unwrap();
        cx.simulate_click(button.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        cx.refresh().unwrap();
        root.update_in(cx, |root, window, cx| {
            assert!(!root.command_center_is_active());
            assert_eq!(
                root.workspace.selected_project_id(),
                Some(&scope.project_id),
            );
            assert_eq!(
                root.active_work_item(),
                Some(WorkItemId::Terminal(scope.tab_id.clone()))
            );
            assert_eq!(
                root.selected_focused_pane_id(),
                Some(scope.pane_id.as_str())
            );
            root.select_command_center();
            let answer = root.command_center_question_input(
                "request",
                "choice",
                question_target("request", "choice"),
                window,
                cx,
            );
            let prompt = root.command_center_composer_input("follow-up", window, cx);
            assert_eq!(answer.read(cx).value().as_str(), "Answer draft");
            assert_eq!(prompt.read(cx).value().as_str(), "Prompt draft");
            assert_eq!(
                root.command_center.selection.as_ref().unwrap().execution_id,
                "child-execution",
            );
            cx.notify();
        });
        cx.run_until_parked();
        cx.refresh().unwrap();
        assert!(cx.debug_bounds("command-center-surface").is_some());
    }

    #[gpui::test]
    fn closed_session_targets_do_not_activate_another_project_tab_or_pane(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(gpui_component::init);
        for removed in ["project", "tab", "pane"] {
            let (_temp, mut view, scope) = session_jump_fixture();
            let session_id = view
                .command_center_sources()
                .pop()
                .unwrap()
                .terminal_session_id;
            let project_id = scope.project_id.clone();
            view.workspace.select_project(&project_id).unwrap();
            view.workspace.select_tab(&scope.tab_id).unwrap();
            match removed {
                "project" => {
                    view.workspace.confirm_close_project(&project_id).unwrap();
                }
                "tab" => {
                    view.workspace.close_selected_tab().unwrap();
                }
                "pane" => {
                    view.workspace.focus_pane(&scope.pane_id).unwrap();
                    view.workspace.close_focused_pane().unwrap();
                }
                _ => unreachable!(),
            }
            view.select_command_center();
            let before = view.workspace.persisted_state();
            let slot = Rc::new(std::cell::RefCell::new(None));
            let output = slot.clone();
            let (_, cx) = cx.add_window_view(|window, cx| {
                let root = cx.new(|_| view);
                *output.borrow_mut() = Some(root.clone());
                ComponentRoot::new(root, window, cx)
            });
            let root = slot.borrow_mut().take().unwrap();
            root.update_in(cx, |root, window, cx| {
                assert!(!root.command_center_session_is_open(&scope));
                root.command_center_open_session(&scope, &session_id, window, cx);
                assert_eq!(root.workspace.persisted_state(), before, "{removed}");
                assert!(root.command_center_is_active());
                assert!(
                    root.command_center
                        .feedback
                        .values()
                        .any(|feedback| matches!(
                            feedback,
                            CommandCenterFeedback::Text(UiTextKey::CommandCenterSessionNotOpen)
                        ))
                );
            });
        }
    }

    #[gpui::test]
    fn language_and_target_switches_preserve_unsent_board_drafts(cx: &mut gpui::TestAppContext) {
        cx.update(gpui_component::init);
        let temp = tempfile::tempdir().unwrap();
        let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
        let slot = Rc::new(std::cell::RefCell::new(None));
        let output = slot.clone();
        let (_, cx) = cx.add_window_view(|window, cx| {
            let root = cx.new(|_| WorkbenchView::with_config_paths_for_test(paths));
            *output.borrow_mut() = Some(root.clone());
            ComponentRoot::new(root, window, cx)
        });
        let root = slot.borrow_mut().take().unwrap();
        root.update_in(cx, |root, window, cx| {
            root.set_language(LanguageSetting::English).unwrap();
            let answer = root.command_center_question_input(
                "request",
                "choice",
                question_target("request", "choice"),
                window,
                cx,
            );
            answer.update(cx, |input, cx| input.set_value("保留我的回答", window, cx));
            let prompt = root.command_center_composer_input("first", window, cx);
            prompt.update(cx, |input, cx| {
                input.set_value("Keep this follow-up", window, cx)
            });

            root.set_language(LanguageSetting::Chinese).unwrap();
            let answer = root.command_center_question_input(
                "request",
                "choice",
                question_target("request", "choice"),
                window,
                cx,
            );
            let prompt = root.command_center_composer_input("first", window, cx);
            assert_eq!(answer.read(cx).value().as_str(), "保留我的回答");
            assert_eq!(prompt.read(cx).value().as_str(), "Keep this follow-up");

            let other = root.command_center_composer_input("second", window, cx);
            other.update(cx, |input, cx| {
                input.set_value("第二个执行的草稿", window, cx)
            });
            root.set_language(LanguageSetting::English).unwrap();
            let restored = root.command_center_composer_input("first", window, cx);
            assert_eq!(restored.read(cx).value().as_str(), "Keep this follow-up");
            let other = root.command_center_composer_input("second", window, cx);
            assert_eq!(other.read(cx).value().as_str(), "第二个执行的草稿");
            let answer = root.command_center_question_input(
                "request",
                "choice",
                question_target("request", "choice"),
                window,
                cx,
            );
            assert_eq!(answer.read(cx).value().as_str(), "保留我的回答");
        });
    }
}
