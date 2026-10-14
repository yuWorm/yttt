use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
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
    AgentBoardCommand, AgentBoardTask, AgentExecutionRecord, AgentMessageRole, AgentPlanStatus,
    AgentProcessState, AgentQuestionAnswer, AgentQuestionRecord, AgentQuestionState, AgentSnapshot,
    AgentTurnState,
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
}

#[derive(Default)]
struct CommandCenterQuestionDraft {
    selected_options: BTreeMap<String, BTreeSet<String>>,
    custom_inputs: HashMap<String, Entity<InputState>>,
    input_subscriptions: HashMap<String, Subscription>,
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
        let mut board = div()
            .id("command-center-projects")
            .flex()
            .flex_wrap()
            .gap(px(10.0))
            .min_w_0()
            .min_h_0()
            .flex_1()
            .overflow_y_scroll()
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
                    .min_h(px(31.0))
                    .py(px(4.0))
                    .px(px(12.0))
                    .gap_x(px(10.0))
                    .gap_y(px(2.0))
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
                            .text_color(theme.text_muted)
                            .child(self.ui_text.get(UiTextKey::CommandCenterSubtitle)),
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
                    .flex_none()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .min_h(px(23.0))
                    .py(px(3.0))
                    .px(px(12.0))
                    .gap_x(px(14.0))
                    .gap_y(px(2.0))
                    .border_b(px(1.0))
                    .border_color(theme.border_variant)
                    .text_xs()
                    .text_color(theme.text_subtle)
                    .child(
                        div()
                            .whitespace_normal()
                            .child(self.ui_text.get(UiTextKey::CommandCenterStablePositions)),
                    )
                    .child(
                        div()
                            .whitespace_normal()
                            .child(self.ui_text.get(UiTextKey::CommandCenterSelectionHint)),
                    ),
            )
            .child(
                div()
                    .flex()
                    .min_h_0()
                    .flex_basis(relative(0.63))
                    .flex_shrink(1.0)
                    .child(board),
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
                        source.terminal_session_id == selection.terminal_session_id
                            && (source
                                .snapshot
                                .board
                                .executions
                                .iter()
                                .any(|execution| execution.id == selection.execution_id)
                                || source.snapshot.board.tasks.iter().any(|task| {
                                    task.execution_id == selection.execution_id
                                        && selection.task_id.as_deref()
                                            == Some(task.item.id.as_str())
                                })
                                || source.snapshot.board.questions.iter().any(|question| {
                                    question.request.execution_id == selection.execution_id
                                })
                                || (selection.execution_id == source.terminal_session_id
                                    && source.snapshot.board.executions.is_empty()))
                    })
                });
        if selection_is_current {
            return;
        }
        self.command_center.selection = sources.first().map(|source| {
            let task = source.snapshot.board.tasks.first();
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
            }
        });
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
        let task_count = sources
            .iter()
            .map(|source| source.snapshot.board.tasks.len())
            .sum::<usize>();
        let completed = sources
            .iter()
            .flat_map(|source| source.snapshot.board.tasks.iter())
            .filter(|task| task.item.status == AgentPlanStatus::Completed)
            .count();
        let plan_missing = sources
            .iter()
            .any(|source| source.snapshot.board.tasks.is_empty());
        let selected_project = self
            .command_center
            .selection
            .as_ref()
            .is_some_and(|selection| {
                sources
                    .iter()
                    .any(|source| source.terminal_session_id == selection.terminal_session_id)
            });
        let mut task_list = div()
            .id(format!("command-center-tasks:{project_id}"))
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .overflow_y_scroll();
        for source in sources {
            if source.snapshot.board.tasks.is_empty() {
                let execution_id = source.terminal_session_id.clone();
                let session_id = source.terminal_session_id.clone();
                let is_selected = self
                    .command_center
                    .selection
                    .as_ref()
                    .is_some_and(|selection| {
                        selection.terminal_session_id == session_id
                            && selection.execution_id == execution_id
                            && selection.task_id.is_none()
                    });
                let summary = source.snapshot.primary_text();
                task_list =
                    task_list.child(
                        div()
                            .id(format!(
                                "command-center-summary:{}",
                                source.terminal_session_id
                            ))
                            .cursor_pointer()
                            .flex()
                            .items_center()
                            .min_h(px(23.0))
                            .px(px(5.0))
                            .gap(px(6.0))
                            .border_l(px(2.0))
                            .border_color(if is_selected {
                                theme.accent
                            } else {
                                theme.app_background
                            })
                            .bg(if is_selected {
                                theme.selection
                            } else {
                                theme.ghost_element_background
                            })
                            .on_click(cx.listener(move |root, _, _window, cx| {
                                root.command_center_select(
                                    session_id.clone(),
                                    execution_id.clone(),
                                    None,
                                    cx,
                                );
                            }))
                            .child(
                                div()
                                    .w(px(12.0))
                                    .text_xs()
                                    .text_color(command_center_turn_color(
                                        source.snapshot.turn_state,
                                        theme,
                                    ))
                                    .child(command_center_turn_symbol(source.snapshot.turn_state)),
                            )
                            .child(div().flex_1().min_w_0().truncate().text_xs().child(summary))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme.text_subtle)
                                    .child(source.snapshot.provider_id.to_string()),
                            )
                            .child(div().text_xs().text_color(theme.text_subtle).child(
                                command_center_age(self.ui_text, source.snapshot.updated_at),
                            )),
                    );
            } else {
                for task in &source.snapshot.board.tasks {
                    task_list =
                        task_list.child(self.command_center_task_row(source, task, theme, cx));
                }
            }
        }
        div()
            .flex()
            .flex_col()
            .flex_basis(relative(0.32))
            .flex_shrink(1.0)
            .min_w(px(250.0))
            .h(px(184.0))
            .p(px(7.0))
            .border(px(1.0))
            .border_color(if selected_project {
                theme.accent
            } else {
                theme.border
            })
            .bg(theme.surface)
            .child(
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
            .child(
                div()
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_between()
                    .mb(px(4.0))
                    .text_xs()
                    .text_color(theme.text_muted)
                    .child(if task_count == 0 {
                        self.ui_text
                            .get(UiTextKey::CommandCenterNoReportedTasks)
                            .to_string()
                    } else {
                        command_center_completed_tasks(self.ui_text, completed, task_count)
                    })
                    .child(self.ui_text.get(if plan_missing {
                        UiTextKey::CommandCenterProviderPlanNotReported
                    } else {
                        UiTextKey::CommandCenterProviderTaskPlan
                    })),
            )
            .child(task_list)
    }

    fn command_center_task_row(
        &self,
        source: &CommandCenterSource,
        task: &AgentBoardTask,
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
        let dimmed = task.retired
            || matches!(
                task.item.status,
                AgentPlanStatus::Completed | AgentPlanStatus::Abandoned
            );
        div()
            .id(format!(
                "command-center-task:{session_id}:{execution_id}:{task_id}"
            ))
            .cursor_pointer()
            .flex()
            .items_center()
            .min_h(px(22.0))
            .px(px(5.0))
            .gap(px(6.0))
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
            .opacity(if dimmed { 0.58 } else { 1.0 })
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
                    .text_xs()
                    .text_color(theme.text_subtle)
                    .child(self.ui_text.get(if task.accepted_at.is_some() {
                        UiTextKey::CommandCenterTaskAccepted
                    } else if task.retired {
                        UiTextKey::CommandCenterTaskRetired
                    } else {
                        command_center_plan_label(task.item.status)
                    })),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(theme.text_subtle)
                    .child(command_center_age(self.ui_text, task.updated_at)),
            )
    }

    fn command_center_questions_panel(
        &mut self,
        sources: &[CommandCenterSource],
        theme: WorkbenchTheme,
        style: UiStyle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let question_count = sources
            .iter()
            .map(|source| source.snapshot.board.questions.len())
            .sum::<usize>();
        let mut records = div()
            .id("command-center-questions")
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .overflow_y_scroll();
        if question_count == 0 {
            records = records.child(command_center_empty_panel(
                self.ui_text.get(UiTextKey::CommandCenterNoQuestions),
                theme,
            ));
        } else {
            for source in sources {
                for record in &source.snapshot.board.questions {
                    records = records.child(
                        self.command_center_question_card(source, record, theme, style, window, cx),
                    );
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
        let selection_session_id = session_id.clone();
        let selection_execution_id = execution_id.clone();
        let selected = self
            .command_center
            .selection
            .as_ref()
            .is_some_and(|selection| {
                selection.terminal_session_id == session_id
                    && selection.execution_id == execution_id
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
                        root.command_center_select(
                            selection_session_id.clone(),
                            selection_execution_id.clone(),
                            None,
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
            let selected_options = self
                .command_center
                .question_drafts
                .get(&draft_key)
                .and_then(|draft| draft.selected_options.get(&question_id))
                .cloned()
                .unwrap_or_default();
            let custom_input =
                self.command_center_question_input(&draft_key, &question_id, window, cx);
            let mut question_body = div()
                .flex()
                .flex_col()
                .gap(px(3.0))
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.text_muted)
                        .children(question.header.iter().cloned()),
                )
                .child(div().text_sm().child(question.question.clone()));
            let multi = question.multi;
            let preview_label = self.ui_text.get(UiTextKey::CommandCenterOptionPreview);
            let mut options = div().flex().flex_wrap().gap(px(4.0));
            for (option_index, option) in question.options.iter().enumerate() {
                let option_label = option.label.clone();
                let option_question_id = question.id.clone();
                let option_draft_key = draft_key.clone();
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
                    .on_click(cx.listener(move |root, _, _window, cx| {
                        root.command_center_toggle_option(
                            &option_draft_key,
                            &option_question_id,
                            option_label.clone(),
                            multi,
                            cx,
                        );
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
        let mut panel = command_center_panel(
            self.ui_text.get(UiTextKey::CommandCenterLinkedConversation),
            format!("{} · {}", source.project_name, execution_id),
            theme,
        )
        .flex_basis(relative(1.12 / 3.28))
        .child(
            div()
                .flex_none()
                .px(px(8.0))
                .py(px(5.0))
                .border_b(px(1.0))
                .border_color(theme.border_variant)
                .text_xs()
                .text_color(theme.text_muted)
                .child(title)
                .child(self.command_center_session_link(source, theme, style, cx)),
        )
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
        let mut rows = div()
            .id("command-center-executions")
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .p(px(4.0));
        let mut reported_count = 0usize;
        let mut has_rows = false;
        for source in sources {
            if source.snapshot.board.executions.is_empty() {
                has_rows = true;
                rows = rows.child(self.command_center_execution_row(
                    source,
                    None,
                    source.terminal_session_id.clone(),
                    self.ui_text.get(UiTextKey::CommandCenterExecutionSummary),
                    source.snapshot.turn_state,
                    source.snapshot.state_started_at,
                    None,
                    theme,
                    cx,
                ));
            } else {
                for execution in &source.snapshot.board.executions {
                    has_rows = true;
                    reported_count += 1;
                    rows = rows.child(self.command_center_execution_row(
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
        }
        if !has_rows {
            rows = rows.child(command_center_empty_panel(
                self.ui_text.get(UiTextKey::CommandCenterNoExecutions),
                theme,
            ));
        }
        command_center_panel(
            self.ui_text.get(UiTextKey::CommandCenterExecutionRecords),
            command_center_count(
                self.ui_text,
                UiTextKey::CommandCenterReportedCount,
                reported_count,
            ),
            theme,
        )
        .flex_basis(relative(0.78 / 3.28))
        .child(
            div()
                .flex_none()
                .flex()
                .flex_wrap()
                .gap(px(6.0))
                .justify_between()
                .px(px(8.0))
                .py(px(4.0))
                .border_b(px(1.0))
                .border_color(theme.border_variant)
                .text_xs()
                .text_color(theme.text_subtle)
                .child(self.ui_text.get(UiTextKey::CommandCenterAgentTask))
                .child(self.ui_text.get(UiTextKey::CommandCenterState))
                .child(self.ui_text.get(UiTextKey::CommandCenterDuration)),
        )
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
        div()
            .id(format!(
                "command-center-execution:{session_id}:{execution_id}"
            ))
            .cursor_pointer()
            .flex()
            .items_center()
            .min_h(px(23.0))
            .px(px(4.0))
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
            .on_click(cx.listener(move |root, _, _window, cx| {
                root.command_center_select(session_id.clone(), execution_id.clone(), None, cx);
            }))
            .child(
                div()
                    .w(px(25.0))
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
                    .child(task.to_string()),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(command_center_turn_color(state, theme))
                    .child(self.ui_text.get(command_center_turn_label(state))),
            )
            .child(
                div()
                    .w(px(37.0))
                    .text_xs()
                    .text_color(theme.text_subtle)
                    .child(command_center_duration(self.ui_text, started_at, ended_at)),
            )
    }

    fn command_center_question_input(
        &mut self,
        request_id: &str,
        question_id: &str,
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
                .entry(request_id.to_string())
                .or_default();
            let locale_changed = draft.input_locale.replace(locale) != Some(locale);
            let existing = draft.custom_inputs.get(question_id).cloned();
            let inputs_to_refresh = locale_changed
                .then(|| draft.custom_inputs.values().cloned().collect::<Vec<_>>())
                .unwrap_or_default();
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
            cx.subscribe_in(&input, window, Self::on_command_center_question_input_event);
        let draft = self
            .command_center
            .question_drafts
            .get_mut(request_id)
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
        _input: &Entity<InputState>,
        event: &InputEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if matches!(event, InputEvent::Change) {
            cx.notify();
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
        if matches!(event, InputEvent::Change)
            && let Some(target) = self.command_center.composer_target.clone()
        {
            self.command_center
                .prompt_drafts
                .insert(target, input.read(cx).value().to_string());
            cx.notify();
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
        });
        self.command_center.pending_terminal_termination = None;
        cx.notify();
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

fn command_center_completed_tasks(ui_text: UiText, completed: usize, total: usize) -> String {
    let completed = completed.to_string();
    let total = total.to_string();
    command_center_template(
        ui_text.get(UiTextKey::CommandCenterCompletedTasks),
        &[("completed", &completed), ("total", &total)],
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

fn command_center_duration(ui_text: UiText, started_at: u64, ended_at: Option<u64>) -> String {
    if started_at == 0 {
        return "—".to_string();
    }
    let millis = ended_at
        .unwrap_or_else(command_center_now_millis)
        .saturating_sub(started_at);
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
            let answer = root.command_center_question_input("request", "choice", window, cx);
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
            let answer = root.command_center_question_input("request", "choice", window, cx);
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
            let answer = root.command_center_question_input("request", "choice", window, cx);
            answer.update(cx, |input, cx| input.set_value("保留我的回答", window, cx));
            let prompt = root.command_center_composer_input("first", window, cx);
            prompt.update(cx, |input, cx| {
                input.set_value("Keep this follow-up", window, cx)
            });

            root.set_language(LanguageSetting::Chinese).unwrap();
            let answer = root.command_center_question_input("request", "choice", window, cx);
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
            let answer = root.command_center_question_input("request", "choice", window, cx);
            assert_eq!(answer.read(cx).value().as_str(), "保留我的回答");
        });
    }
}
