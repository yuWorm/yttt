//! Binds a work window to its Host workspace.
//!
//! The Host owns the layout: a window opens its workspace, mirrors every
//! layout the Host broadcasts, and changes it only through intents. Unsaved
//! document edits live on the Host under per-document edit leases
//! (`documents`). Closing the window takes the workspace out of the next
//! startup once the Host stores every edit.

mod agent_state;
mod documents;
#[cfg(all(test, unix))]
mod documents_tests;
mod host_mirror;
#[cfg(all(test, unix))]
mod host_mirror_tests;

use std::{
    collections::{HashMap, HashSet},
    fs,
    io::{self, Write as _},
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, UNIX_EPOCH},
};

use gpui::{App, Context, Entity, Task, Window};
use yttt_client_core::ConnectionState as HostConnectionState;
use yttt_core::model::ids::ProjectId;
use yttt_protocol::{
    HostPath,
    project::{ContentRevision, ProjectFileFingerprint},
    workspace::{
        AgentStateRef, DocumentState, DraftBase, DraftContentRevision, DraftRef,
        MAX_DRAFT_CONTENT_BYTES, WorkspaceId, WorkspaceRequest, WorkspaceResponse,
    },
};

use crate::{
    model::{
        editor::DocumentId,
        host_workspace::{HostWorkspace, HostWorkspaceState},
    },
    ui::editor::{DiskFingerprint, ProjectEditorDocument},
};

use super::*;

const WORKSPACE_PERSISTENCE_INTERVAL: Duration = Duration::from_millis(500);
/// How long a remote session is down before the window says so, so a quick
/// reconnect does not flash the notice.
const HOST_OUTAGE_NOTICE_DELAY_MILLIS: u64 = 1_000;
/// How long a reconnecting session is down before the notice covers the
/// window and offers to close it. Until then the window stays readable under
/// a dimmed layer, since most drops (a wake, a network switch) recover sooner.
const HOST_OUTAGE_DIALOG_DELAY_MILLIS: u64 = 15_000;

/// How the window shows a remote session that is down.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum HostOutageNotice {
    /// Dimmed content and a small notice; input waits for the session.
    Brief,
    /// A dialog with the reason, a reconnect and closing the window.
    Dialog,
}

/// What the window shows of a remote Host link in trouble.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum HostLinkNotice {
    /// The session is still up, but the Host has not answered a heartbeat
    /// for `waited`, longer than the link counts as steady. Dimmed like a
    /// short outage, since the window shows nothing new until it answers.
    Stalled { waited: Duration },
    /// The session is down.
    Outage(crate::host_runtime::HostOutage, HostOutageNotice),
}

/// The notice for the link now: `outage` is the session being down for
/// `down_for` milliseconds, `health` the link of a session still up, and
/// `shown` whether the window shows a notice already, so a stall that turns
/// into an outage stays dimmed instead of flashing clear for the delay.
fn host_link_notice(
    outage: Option<crate::host_runtime::HostOutage>,
    down_for: u64,
    health: Option<yttt_client_core::LinkHealth>,
    shown: bool,
) -> Option<HostLinkNotice> {
    if let Some(outage) = outage {
        let notice =
            host_outage_notice(&outage, down_for).or(shown.then_some(HostOutageNotice::Brief))?;
        return Some(HostLinkNotice::Outage(outage, notice));
    }
    let health = health.filter(|health| health.unstable)?;
    Some(HostLinkNotice::Stalled {
        waited: health.waiting_for.unwrap_or_default(),
    })
}

/// The notice for an outage `down_for` milliseconds old, if it is time for one.
fn host_outage_notice(
    outage: &crate::host_runtime::HostOutage,
    down_for: u64,
) -> Option<HostOutageNotice> {
    if outage.attempt.is_none() || down_for >= HOST_OUTAGE_DIALOG_DELAY_MILLIS {
        Some(HostOutageNotice::Dialog)
    } else {
        (down_for >= HOST_OUTAGE_NOTICE_DELAY_MILLIS).then_some(HostOutageNotice::Brief)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum WorkspacePersistenceMode {
    /// No Host: the window edits its own workspace.
    #[default]
    Inactive,
    /// Waiting for the Host to open the workspace.
    Loading,
    /// Mirroring the Host workspace.
    Active,
    /// The Host workspace cannot be shown; retried on request.
    RestoreFailed,
}

#[derive(Default)]
pub(super) struct WorkspacePersistenceState {
    view: Option<crate::session_coordinator::WorkspaceViewLease>,
    mode: WorkspacePersistenceMode,
    /// The Host layout revision the window shows.
    revision: Option<u64>,
    host_epoch: Option<u64>,
    /// The connection the workspace was last opened on; a new connection
    /// opens it again to read the document states it missed.
    connection_sequence: Option<u64>,
    require_existing_workspace: bool,
    initial_project: Option<PathBuf>,
    pub(super) startup_projects: Vec<PathBuf>,
    open_in_flight: bool,
    tick_task: Option<Task<()>>,
    host_layout_listener: Option<host_mirror::HostLayoutListener>,
    documents: documents::DocumentSyncState,
    /// The window closes once every document edit is on the Host.
    close_requested: bool,
    close_prompt_open: bool,
    closing: bool,
    last_error: Option<String>,
    /// The window shows that its remote Host link is in trouble.
    outage_shown: bool,
    /// What it shows has included the session being down, so its end is a
    /// recovery worth announcing.
    outage_was_down: bool,
    /// The kind of notice last logged, so each change is logged once.
    outage_notice_kind: Option<&'static str>,
    /// A workspace from history that replaces this window's.
    pending_switch: Option<WorkspaceId>,
    /// Agent states opened as summaries whose boards are not read yet.
    unread_agent_boards: Vec<AgentStateRef>,
    agent_boards_in_flight: bool,
}

/// A workspace as the Host returned it, with the Agent states of its panes.
struct OpenedWorkspace {
    layout: HostWorkspaceState,
    documents: Vec<DocumentState>,
    agent_states: agent_state::OpenedAgentStates,
    host_epoch: u64,
    connection_sequence: u64,
}

enum OpenWorkspaceError {
    /// The saved workspace no longer exists.
    Deleted,
    Failed(String),
}

impl WorkbenchView {
    pub fn has_restorable_workspace(&self) -> bool {
        self.terminal.host_runtime.is_some()
    }

    pub(crate) fn set_workspace_view(
        &mut self,
        view: crate::session_coordinator::WorkspaceViewLease,
    ) {
        self.workspace_persistence.view = Some(view);
        self.workspace_persistence.require_existing_workspace = true;
    }

    pub(crate) fn restore_last_session(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_workspace_history(window, cx);
    }

    pub(super) fn workspace_allows_mutation(&self) -> bool {
        !self.workspace_persistence.closing
            && matches!(
                self.workspace_persistence.mode,
                WorkspacePersistenceMode::Inactive | WorkspacePersistenceMode::Active
            )
    }

    pub(super) fn workspace_is_loading(&self) -> bool {
        self.workspace_persistence.mode == WorkspacePersistenceMode::Loading
    }

    pub(super) fn workspace_restore_failed(&self) -> bool {
        self.workspace_persistence.mode == WorkspacePersistenceMode::RestoreFailed
    }

    /// Opens projects the desktop shell asked for in this window; before the
    /// Host workspace shows, they wait for it like the startup projects.
    pub(crate) fn open_requested_projects(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        self.workspace_persistence.startup_projects.extend(paths);
        if self.workspace_persistence.mode == WorkspacePersistenceMode::Active {
            self.open_startup_projects();
        }
        cx.notify();
    }

    /// Replaces this window's workspace with one from history once the next
    /// tick has the window: the chosen workspace opens where this window is,
    /// and this window closes as the user would close it, so its documents
    /// reach the Host and its workspace returns to history.
    pub(crate) fn request_workspace_switch(&mut self, workspace_id: WorkspaceId) {
        self.workspace_persistence.pending_switch = Some(workspace_id);
    }

    fn finish_workspace_switch(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(workspace_id) = self.workspace_persistence.pending_switch.take() else {
            return;
        };
        if let Err(error) =
            crate::ui::app::open_history_workspace(workspace_id, window.window_bounds(), cx)
        {
            self.palette.workspace_history.error = Some(format!(
                "{}: {error}",
                self.ui_text
                    .get(UiTextKey::WorkbenchI18nWorkspaceHistoryOpenError)
            ));
            cx.notify();
            return;
        }
        if self.flush_workspace_persistence_on_close(window, cx) {
            window.remove_window();
        }
    }

    /// Tells the Host that its own desktop shows this window's workspace, so a
    /// remote Client opens it on arrival. Remote windows never report.
    pub(super) fn report_active_workspace(&self) {
        let Some(runtime) = self.terminal.host_runtime.as_ref() else {
            return;
        };
        if runtime.is_remote()
            || self.workspace_persistence.mode != WorkspacePersistenceMode::Active
        {
            return;
        }
        let Some(view) = self.workspace_persistence.view.as_ref() else {
            return;
        };
        let _ = runtime.request_detached(yttt_protocol::Request::Workspace(
            WorkspaceRequest::SetActive {
                workspace_id: view.id().clone(),
            },
        ));
    }

    /// Why the remote Host link is in trouble and how to show it, once it
    /// has been long enough to say.
    pub(super) fn visible_host_outage(&self) -> Option<HostLinkNotice> {
        let runtime = self.terminal.host_runtime.as_ref()?;
        let outage = runtime.outage();
        let down_for = outage.as_ref().map_or(0, |outage| {
            super::bars::current_unix_millis().saturating_sub(outage.since_unix_millis)
        });
        host_link_notice(
            outage,
            down_for,
            runtime.link_health(),
            self.workspace_persistence.outage_shown,
        )
    }

    /// Redraws while the outage notice shows, to count its time, and once
    /// more when it goes away.
    fn refresh_host_outage(&mut self, cx: &mut Context<Self>) {
        let notice = self.visible_host_outage();
        let shown = notice.is_some();
        if shown || self.workspace_persistence.outage_shown {
            cx.notify();
        }
        let state = &mut self.workspace_persistence;
        let kind = notice.as_ref().map(|notice| match notice {
            HostLinkNotice::Stalled { .. } => "stalled",
            HostLinkNotice::Outage(_, HostOutageNotice::Brief) => "brief",
            HostLinkNotice::Outage(_, HostOutageNotice::Dialog) => "dialog",
        });
        if kind != state.outage_notice_kind {
            tracing::info!(notice = kind.unwrap_or("cleared"), "Host link notice");
            state.outage_notice_kind = kind;
        }
        state.outage_was_down |= matches!(notice, Some(HostLinkNotice::Outage(..)));
        // A stall that ends without losing the session has nothing to announce.
        let recovered = state.outage_shown && !shown && state.outage_was_down;
        if !shown {
            state.outage_was_down = false;
        }
        state.outage_shown = shown;
        if recovered {
            self.announce_host_recovery();
        }
    }

    /// Says the session is back and how long it was down, once the outage
    /// notice goes away.
    fn announce_host_recovery(&mut self) {
        let Some(runtime) = self.terminal.host_runtime.clone() else {
            return;
        };
        let Some(down_for) = runtime.last_recovery().filter(|_| runtime.is_connected()) else {
            return;
        };
        let host = runtime
            .remote_label()
            .and_then(|label| label.rsplit(" · ").next())
            .unwrap_or(self.ui_text.get(UiTextKey::TrayHost))
            .to_string();
        let elapsed = super::bars::format_state_duration(
            u64::try_from(down_for.as_millis()).unwrap_or(u64::MAX),
            &self.ui_text,
        );
        self.queue_status_notification(
            self.ui_text
                .get(UiTextKey::HostReconnectedTitle)
                .replace("{host}", &host),
            self.ui_text
                .get(UiTextKey::HostReconnectedDetail)
                .replace("{elapsed}", &elapsed),
        );
    }

    pub(super) fn workspace_persistence_error(&self) -> Option<&str> {
        self.workspace_persistence.last_error.as_deref()
    }

    pub(super) fn workspace_restore_failure_message(&self) -> Option<&str> {
        self.workspace_restore_failed()
            .then_some(self.workspace_persistence.last_error.as_deref())
            .flatten()
    }

    /// The Host this window works on and what keeps it from being in sync.
    pub(super) fn workspace_status_banner(&self, cx: &mut Context<Self>) -> Div {
        let Some(runtime) = self.terminal.host_runtime.clone() else {
            return div();
        };
        let appearance = self.theme_runtime();
        let identity = if runtime.is_remote() {
            let host = runtime
                .remote_label()
                .and_then(|label| label.rsplit(" · ").next())
                .unwrap_or(self.ui_text.get(UiTextKey::TrayHost));
            self.ui_text
                .get(UiTextKey::WorkspaceStatusRemoteIdentity)
                .replace("{host}", host)
        } else {
            self.ui_text
                .get(UiTextKey::WorkspaceStatusLocalIdentity)
                .to_string()
        };
        let state = if !runtime.is_connected() {
            self.ui_text.get(UiTextKey::WorkspaceStatusDisconnected)
        } else if self.workspace_restore_failed() {
            self.ui_text.get(UiTextKey::WorkspaceStatusRestoreFailed)
        } else if self.workspace_is_loading() {
            self.ui_text.get(UiTextKey::WorkspaceStatusRestoring)
        } else if self.settings_save_pending() || self.has_unsynced_documents(cx) {
            self.ui_text.get(UiTextKey::WorkspaceStatusSaving)
        } else {
            ""
        };
        let observed = self.terminals_observed_elsewhere(cx);
        let control = (observed > 0).then(|| {
            self.ui_text
                .get(UiTextKey::WorkspaceStatusTerminalsControlledElsewhere)
                .replace("{count}", &observed.to_string())
        });
        div()
            .flex()
            .items_center()
            .gap(appearance.style.spacing.sm)
            .px(appearance.style.spacing.md)
            .text_sm()
            .debug_selector(|| "workspace-status".to_string())
            .child(format!("{identity}{state}{}", control.unwrap_or_default()))
            .child(
                yttt_button(
                    "workspace-history",
                    self.ui_text.get(UiTextKey::WorkspaceStatusHistory),
                    YtttButtonVariant::Ghost,
                    appearance.ui,
                    appearance.style,
                )
                .on_click(
                    cx.listener(|root, _, window, cx| root.open_workspace_history(window, cx)),
                ),
            )
    }

    pub(super) fn forget_workspace_history(
        &mut self,
        entry: yttt_protocol::workspace::WorkspaceSummary,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(runtime) = self.terminal.host_runtime.clone() else {
            return;
        };
        if entry.restore_on_startup {
            return;
        }
        let workspace_id = entry.workspace_id.clone();
        let answer = window.prompt(
            gpui::PromptLevel::Warning,
            self.ui_text.get(UiTextKey::WorkspaceHistoryRemoveTitle),
            Some(self.ui_text.get(UiTextKey::WorkspaceHistoryRemoveWarning)),
            &[
                self.ui_text.get(UiTextKey::Cancel),
                self.ui_text.get(UiTextKey::WorkspaceHistoryRemove),
            ],
            cx,
        );
        let text = self.ui_text;
        cx.spawn_in(window, async move |this, cx| {
            if !matches!(answer.await, Ok(1)) {
                return;
            }
            let result = cx
                .background_executor()
                .spawn(async move {
                    if runtime.workspace_has_view(&entry.workspace_id) {
                        return Err(text
                            .get(UiTextKey::PersistenceWorkspaceAlreadyOpen)
                            .to_string());
                    }
                    runtime.workspace_request(WorkspaceRequest::Forget {
                        workspace_id: entry.workspace_id,
                        expected_revision: entry.revision,
                    })
                })
                .await;
            let _ = this.update_in(cx, |root, _window, cx| {
                match result {
                    Ok(WorkspaceResponse::Forgotten { .. }) => {
                        root.remove_workspace_history_entry(&workspace_id);
                    }
                    result => {
                        root.palette.workspace_history.error = Some(
                            root.ui_text
                                .get(UiTextKey::PersistenceRemoveHistory)
                                .replace("{detail}", &format!("{result:?}")),
                        )
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Claims a workspace for this window and opens it on the Host.
    pub(crate) fn start_workspace_persistence(
        &mut self,
        restore_existing: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(runtime) = self.terminal.host_runtime.clone() else {
            return;
        };
        if self.workspace_persistence.view.is_none() {
            self.workspace_persistence.require_existing_workspace =
                restore_existing && runtime.pending_workspace_count() > 0;
            match runtime.claim_workspace_view(restore_existing) {
                Ok(view) => self.workspace_persistence.view = Some(view),
                Err(error) => {
                    self.set_workspace_persistence_error(error);
                    return;
                }
            }
        }
        if self.workspace_persistence.mode != WorkspacePersistenceMode::Inactive {
            return;
        }
        self.workspace_persistence.initial_project = self
            .workspace
            .opened_projects()
            .first()
            .and_then(|project| project.location.local_path())
            .cloned();
        self.workspace_persistence.mode = WorkspacePersistenceMode::Loading;
        self.start_workspace_persistence_tick(window, cx);
        self.open_host_workspace(window, cx);
    }

    pub(super) fn retry_failed_workspace_restore(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.workspace_persistence.mode != WorkspacePersistenceMode::RestoreFailed {
            return;
        }
        self.workspace_persistence.mode = WorkspacePersistenceMode::Loading;
        self.clear_workspace_persistence_error();
        self.open_host_workspace(window, cx);
        cx.notify();
    }

    fn start_workspace_persistence_tick(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.workspace_persistence.tick_task.is_some() {
            return;
        }
        self.workspace_persistence.tick_task = Some(cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(WORKSPACE_PERSISTENCE_INTERVAL)
                    .await;
                if this
                    .update_in(cx, |root, window, cx| {
                        root.tick_workspace_persistence(window, cx);
                        root.refresh_host_outage(cx);
                        root.flush_pending_status_notifications(window, cx);
                        root.finish_workspace_switch(window, cx);
                    })
                    .is_err()
                {
                    break;
                }
            }
        }));
    }

    fn tick_workspace_persistence(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.workspace_persistence.closing
            || matches!(
                self.workspace_persistence.mode,
                WorkspacePersistenceMode::Inactive | WorkspacePersistenceMode::RestoreFailed
            )
        {
            return;
        }
        let Some(runtime) = self.terminal.host_runtime.clone() else {
            return;
        };
        let HostConnectionState::Ready {
            host_epoch,
            connection_sequence,
        } = runtime.state()
        else {
            return;
        };
        let Some(workspace_id) = self
            .workspace_persistence
            .view
            .as_ref()
            .map(|view| view.id().clone())
        else {
            return;
        };
        self.ensure_host_layout_listener(&runtime, &workspace_id, window, cx);
        self.ensure_document_listener(&runtime, &workspace_id, window, cx);
        if self.workspace_persistence.mode == WorkspacePersistenceMode::Loading
            || self.workspace_persistence.host_epoch != Some(host_epoch)
            || self.workspace_persistence.connection_sequence != Some(connection_sequence)
        {
            self.open_host_workspace(window, cx);
            return;
        }
        self.adopt_host_workspace_layout(window, cx);
        self.open_startup_projects();
        self.sync_documents(window, cx);
        if self.workspace_persistence.close_requested {
            self.finish_close_when_synced(window, cx);
        }
    }

    /// Opens the requested projects once the workspace shows the Host
    /// layout; the window's first project reopens only into an empty one.
    fn open_startup_projects(&mut self) {
        let mut paths = std::mem::take(&mut self.workspace_persistence.startup_projects);
        if paths.is_empty() && self.workspace.opened_projects().is_empty() {
            paths.extend(self.workspace_persistence.initial_project.take());
        }
        for path in paths {
            if let Err(error) = self.open_project_path(path) {
                self.set_workspace_persistence_error(
                    self.ui_text
                        .get(UiTextKey::PersistenceOpenInitialProject)
                        .replace("{error}", &error.to_string()),
                );
            }
        }
    }

    /// Reads the boards of the Agent states this window opened as summaries,
    /// once something shows boards. A failed read is tried again the next time.
    pub(super) fn read_unread_agent_boards(&mut self, cx: &mut Context<Self>) {
        let persistence = &mut self.workspace_persistence;
        if persistence.unread_agent_boards.is_empty() || persistence.agent_boards_in_flight {
            return;
        }
        let (Some(runtime), Some(workspace_id)) = (
            self.terminal.host_runtime.clone(),
            persistence.view.as_ref().map(|view| view.id().clone()),
        ) else {
            return;
        };
        let references = std::mem::take(&mut persistence.unread_agent_boards);
        let host_epoch = persistence.host_epoch;
        persistence.agent_boards_in_flight = true;
        let task = cx.background_spawn({
            let workspace_id = workspace_id.clone();
            let references = references.clone();
            async move {
                let started = std::time::Instant::now();
                let read = read_agent_states(&runtime, &workspace_id, references);
                match &read {
                    Ok(bodies) => tracing::info!(
                        workspace = workspace_id.as_str(),
                        states = bodies.len(),
                        bytes = bodies
                            .iter()
                            .map(|(_, content)| content.len())
                            .sum::<usize>(),
                        millis = started.elapsed().as_millis() as u64,
                        "Agent boards read"
                    ),
                    Err(error) => tracing::warn!(
                        workspace = workspace_id.as_str(),
                        %error,
                        "Agent boards were not read; trying again when the command center opens"
                    ),
                }
                read
            }
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |root, cx| {
                let persistence = &mut root.workspace_persistence;
                persistence.agent_boards_in_flight = false;
                let current = persistence.host_epoch == host_epoch
                    && persistence
                        .view
                        .as_ref()
                        .is_some_and(|view| *view.id() == workspace_id);
                if !current {
                    return;
                }
                let bodies = match result {
                    Ok(bodies) => bodies,
                    Err(_) => {
                        persistence.unread_agent_boards = references;
                        return;
                    }
                };
                for (reference, content) in bodies {
                    let Ok(stored) = agent_state::decode(&reference, &content) else {
                        continue;
                    };
                    let address = AgentPaneAddress::new(
                        reference.project_id.as_str(),
                        &reference.tab_id,
                        &reference.pane_id,
                    );
                    if let Some(mut snapshot) = root.pane_agent_snapshot(&address).cloned() {
                        agent_state::add_stored_board(&mut snapshot, stored);
                        let _ = root.record_agent_runtime_snapshot(address, snapshot);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Opens the workspace on the Host and shows it: the first time, or for
    /// a Host that restarted, as a whole; otherwise only the document states,
    /// since the layout mirror already follows the Host.
    fn open_host_workspace(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.workspace_persistence.open_in_flight {
            return;
        }
        let Some(runtime) = self.terminal.host_runtime.clone() else {
            return;
        };
        let Some(workspace_id) = self
            .workspace_persistence
            .view
            .as_ref()
            .map(|view| view.id().clone())
        else {
            return;
        };
        self.workspace_persistence.open_in_flight = true;
        let require_existing = self.workspace_persistence.require_existing_workspace;
        let task_runtime = runtime.clone();
        let task = cx.background_spawn(async move {
            open_workspace(&task_runtime, workspace_id, require_existing)
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |root, window, cx| {
                root.workspace_persistence.open_in_flight = false;
                if root.workspace_persistence.closing {
                    return;
                }
                match result {
                    Ok(opened) => root.show_opened_workspace(opened, &runtime, window, cx),
                    Err(OpenWorkspaceError::Deleted) => root.set_remote_workspace_restore_failure(
                        root.ui_text
                            .get(UiTextKey::PersistenceWorkspaceDeleted)
                            .to_string(),
                    ),
                    Err(OpenWorkspaceError::Failed(error)) => {
                        root.set_workspace_persistence_error(
                            root.ui_text
                                .get(UiTextKey::PersistenceOpenFailed)
                                .replace("{error}", &error),
                        );
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn show_opened_workspace(
        &mut self,
        opened: OpenedWorkspace,
        runtime: &Arc<crate::host_runtime::DesktopHostRuntime>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let first = self.workspace_persistence.revision.is_none();
        let same_host = self.workspace_persistence.host_epoch == Some(opened.host_epoch);
        if first || !same_host {
            let mut layout = opened.layout;
            let (restored, unread_boards) = match &opened.agent_states {
                agent_state::OpenedAgentStates::Bodies(bodies) => (
                    agent_state::restore(
                        &mut layout.workspace,
                        &bodies
                            .iter()
                            .map(|(reference, _)| reference.clone())
                            .collect::<Vec<_>>(),
                        |reference| {
                            bodies
                                .iter()
                                .find(|(body, _)| body == reference)
                                .map(|(_, content)| content.clone())
                                .ok_or_else(|| "Host Agent state body is missing".into())
                        },
                    ),
                    Vec::new(),
                ),
                agent_state::OpenedAgentStates::Summaries(summaries) => (
                    agent_state::restore_summaries(&mut layout.workspace, summaries),
                    summaries
                        .iter()
                        .map(|summary| summary.reference.clone())
                        .collect(),
                ),
            };
            let restored = restored
                .and_then(|()| HostWorkspace::restore(layout).map_err(|error| error.to_string()));
            let host = match restored {
                Ok(host) => host,
                Err(error) => {
                    self.set_remote_workspace_restore_failure(
                        self.ui_text
                            .get(UiTextKey::PersistenceStateRestoreFailed)
                            .replace("{error}", &error),
                    );
                    return;
                }
            };
            if first {
                self.reset_window_workspace();
            }
            self.workspace_persistence.unread_agent_boards = unread_boards;
            self.workspace_persistence.host_epoch = Some(opened.host_epoch);
            // A command center already open does not open again to read them.
            if self.command_center_is_active() {
                self.read_unread_agent_boards(cx);
            }
            self.replace_with_host_layout(
                host,
                host_mirror::HostAgentStates::Included,
                runtime,
                window,
                cx,
            );
        }
        self.workspace_persistence.host_epoch = Some(opened.host_epoch);
        self.workspace_persistence.connection_sequence = Some(opened.connection_sequence);
        self.workspace_persistence.mode = WorkspacePersistenceMode::Active;
        self.clear_workspace_persistence_error();
        self.report_active_workspace();
        self.record_all_document_states(opened.documents);
        self.adopt_host_workspace_layout(window, cx);
        self.open_startup_projects();
        self.sync_documents(window, cx);
    }

    /// Drops what the window showed before it opened its Host workspace.
    fn reset_window_workspace(&mut self) {
        self.workspace = Workspace::new();
        self.project.services.clear();
        self.project.project_editor_runtime = ProjectEditorRuntime::default();
        self.project.layout_source_messages.clear();
        self.project.pending_editor_focus_document_id = None;
        self.project.pending_project_tree_focus = false;
        self.project.pending_project_tree_loads.clear();
        self.project.project_git_statuses.clear();
        self.project.project_tree_clipboard = None;
        self.active_project_file_watcher = None;
        self.terminal.terminal_panes.clear();
        self.terminal.terminal_pane_subscriptions.clear();
        self.terminal.pending_terminal_focus = None;
        self.terminal.pending_host_agent_snapshots.clear();
        self.onboarding = None;
    }

    /// Closes the window once the Host stores every document edit, taking
    /// the workspace out of the next startup.
    pub(crate) fn flush_workspace_persistence_on_close(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.workspace_persistence.closing {
            return false;
        }
        if self.settings_save_pending() || self.has_failed_settings_save() {
            self.set_workspace_persistence_error(
                self.ui_text
                    .get(UiTextKey::WorkspaceSettingsPending)
                    .to_string(),
            );
            self.confirm_close_without_sync(window, cx);
            return false;
        }
        if self.workspace_persistence.mode == WorkspacePersistenceMode::Inactive {
            return self.request_window_close(cx) && self.record_closed_window(cx);
        }
        if self.workspace_persistence.mode != WorkspacePersistenceMode::Active
            || !self
                .terminal
                .host_runtime
                .as_ref()
                .is_some_and(|runtime| runtime.is_connected())
        {
            self.set_workspace_persistence_error(
                self.ui_text
                    .get(UiTextKey::WorkspaceFlushHostUnavailable)
                    .to_string(),
            );
            self.confirm_close_without_sync(window, cx);
            return false;
        }
        self.workspace_persistence.close_requested = true;
        self.sync_documents(window, cx);
        self.finish_close_when_synced(window, cx);
        false
    }

    fn finish_close_when_synced(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(error) = self.document_sync_error().map(str::to_string) {
            self.workspace_persistence.close_requested = false;
            self.set_workspace_persistence_error(error);
            self.confirm_close_without_sync(window, cx);
        } else if !self.has_unsynced_documents(cx) {
            self.workspace_persistence.close_requested = false;
            self.close_host_workspace(false, window, cx);
        }
    }

    fn confirm_close_without_sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.workspace_persistence.close_prompt_open {
            return;
        }
        self.workspace_persistence.close_prompt_open = true;
        let answer = window.prompt(
            gpui::PromptLevel::Warning,
            self.ui_text.get(UiTextKey::WorkspaceUnsavedTitle),
            self.workspace_persistence.last_error.as_deref(),
            &[
                self.ui_text.get(UiTextKey::WorkspaceKeepOpen),
                self.ui_text.get(UiTextKey::WorkspaceCloseUnsaved),
            ],
            cx,
        );
        cx.spawn_in(window, async move |this, cx| {
            let answer = answer.await;
            let _ = this.update_in(cx, |root, window, cx| {
                root.workspace_persistence.close_prompt_open = false;
                if matches!(answer, Ok(1)) {
                    root.close_host_workspace(true, window, cx);
                }
            });
        })
        .detach();
    }

    /// Takes the workspace out of the next startup and closes the window.
    /// `force` closes the window even when the Host cannot be told.
    fn close_host_workspace(&mut self, force: bool, window: &mut Window, cx: &mut Context<Self>) {
        let target = self.terminal.host_runtime.clone().zip(
            self.workspace_persistence
                .view
                .as_ref()
                .map(|view| view.id().clone()),
        );
        let Some((runtime, workspace_id)) = target.filter(|(runtime, _)| runtime.is_connected())
        else {
            if force && self.record_closed_window(cx) {
                window.remove_window();
            }
            return;
        };
        self.workspace_persistence.closing = true;
        cx.notify();
        let task = cx.background_spawn(async move { leave_startup(&runtime, workspace_id) });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |root, window, cx| match result {
                Ok(()) => {
                    if root.record_closed_window(cx) {
                        window.remove_window();
                    }
                }
                Err(_) if force => {
                    if root.record_closed_window(cx) {
                        window.remove_window();
                    }
                }
                Err(error) => {
                    root.workspace_persistence.closing = false;
                    root.set_workspace_persistence_error(
                        root.ui_text
                            .get(UiTextKey::PersistenceCloseSavedWorkspace)
                            .replace("{detail}", &error),
                    );
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn record_closed_window(&mut self, cx: &mut Context<Self>) -> bool {
        if cx.has_global::<crate::remote_restore::RemoteRestoreGlobal>()
            && let Err(error) = cx
                .global_mut::<crate::remote_restore::RemoteRestoreGlobal>()
                .close_window()
        {
            self.set_workspace_persistence_error(
                self.ui_text
                    .get(UiTextKey::PersistenceUpdateClientRestore)
                    .replace("{error}", &error.to_string()),
            );
            cx.notify();
            return false;
        }
        true
    }

    /// Sends a layout intent for the window's Host workspace. `Ok(false)`
    /// means the window has no Host workspace and edits its own; `Err` means
    /// the Host did not apply the intent, so the window must not either.
    pub(super) fn publish_workspace_intent(
        &mut self,
        intent: WorkspaceIntent,
    ) -> Result<bool, String> {
        if self.workspace_persistence.mode == WorkspacePersistenceMode::Inactive {
            return Ok(false);
        }
        let Some(runtime) = self.terminal.host_runtime.clone() else {
            return Ok(false);
        };
        let Some(workspace_id) = self
            .workspace_persistence
            .view
            .as_ref()
            .map(|view| view.id().clone())
        else {
            return Ok(false);
        };
        if self.workspace_persistence.mode != WorkspacePersistenceMode::Active {
            return Err(self
                .ui_text
                .get(UiTextKey::WorkspaceFlushHostUnavailable)
                .to_string());
        }
        match runtime.workspace_request(WorkspaceRequest::Apply {
            workspace_id: workspace_id.clone(),
            intent,
            expected_revision: None,
        })? {
            WorkspaceResponse::Applied {
                workspace_id: applied_workspace_id,
                revision,
            } if applied_workspace_id == workspace_id => {
                // Contiguous: the Host applied the intent to the layout this
                // window shows. Otherwise another Client's change came first,
                // and its broadcast replaces the local layout.
                if self.workspace_persistence.revision.map(|held| held + 1) == Some(revision) {
                    self.workspace_persistence.revision = Some(revision);
                }
                Ok(true)
            }
            response => Err(format!("unexpected workspace response: {response:?}")),
        }
    }

    fn set_remote_workspace_restore_failure(&mut self, error: String) {
        self.workspace_persistence.mode = WorkspacePersistenceMode::RestoreFailed;
        self.set_workspace_persistence_error(error);
    }

    fn set_workspace_persistence_error(&mut self, message: String) {
        if self.workspace_persistence.last_error.as_deref() == Some(message.as_str()) {
            return;
        }
        if let Some(previous) = self
            .workspace_persistence
            .last_error
            .replace(message.clone())
        {
            self.load_error = remove_load_message(self.load_error.take(), &previous);
        }
        self.load_error = combine_load_messages(self.load_error.take(), Some(message));
    }

    fn clear_workspace_persistence_error(&mut self) {
        let Some(message) = self.workspace_persistence.last_error.take() else {
            return;
        };
        self.load_error = remove_load_message(self.load_error.take(), &message);
    }
}

fn remove_load_message(load_error: Option<String>, message: &str) -> Option<String> {
    let remaining = load_error?
        .lines()
        .filter(|line| *line != message)
        .collect::<Vec<_>>()
        .join("\n");
    (!remaining.is_empty()).then_some(remaining)
}

/// Opens `workspace_id` on the Host, registering a new one unless it must
/// exist, and reads the Agent states of its panes.
fn open_workspace(
    runtime: &crate::host_runtime::DesktopHostRuntime,
    workspace_id: WorkspaceId,
    require_existing: bool,
) -> Result<OpenedWorkspace, OpenWorkspaceError> {
    let failed = OpenWorkspaceError::Failed;
    let started = std::time::Instant::now();
    if !require_existing {
        runtime
            .workspace_request(WorkspaceRequest::Register {
                workspace_id: workspace_id.clone(),
                name: workspace_id.as_str().to_string(),
            })
            .map_err(failed)?;
    }
    let request = if require_existing {
        WorkspaceRequest::OpenExisting {
            workspace_id: workspace_id.clone(),
        }
    } else {
        WorkspaceRequest::Open {
            workspace_id: workspace_id.clone(),
        }
    };
    let (layout, documents, references, summaries) =
        match runtime.request_blocking_typed(yttt_protocol::Request::Workspace(request)) {
            Ok(yttt_protocol::Response::Workspace(WorkspaceResponse::Opened {
                layout,
                documents,
                agent_states,
                agent_summaries,
            })) => (layout, documents, agent_states, agent_summaries),
            Err(yttt_client_core::ClientCoreError::Protocol(failure))
                if failure.code == yttt_protocol::FailureCode::NotFound =>
            {
                return Err(OpenWorkspaceError::Deleted);
            }
            Ok(response) => {
                return Err(failed(format!(
                    "unexpected workspace open response: {response:?}"
                )));
            }
            Err(error) => return Err(failed(error.to_string())),
        };
    if require_existing {
        // A workspace reopened from history returns to the startup set.
        let WorkspaceResponse::Workspaces(entries) = runtime
            .workspace_request(WorkspaceRequest::List)
            .map_err(failed)?
        else {
            return Err(failed("Host did not return its workspace index".into()));
        };
        if let Some(entry) = entries
            .into_iter()
            .find(|entry| entry.workspace_id == workspace_id && !entry.restore_on_startup)
        {
            runtime
                .workspace_request(WorkspaceRequest::SetRestoreOnStartup {
                    workspace_id: workspace_id.clone(),
                    expected_revision: entry.revision,
                    restore: true,
                })
                .map_err(failed)?;
        }
    }
    // Over a remote link the boards, nearly all of each state, wait until
    // something shows them.
    let opened_millis = started.elapsed().as_millis() as u64;
    let agent_states =
        if runtime.is_remote() && agent_state::summaries_cover(&references, &summaries) {
            agent_state::OpenedAgentStates::Summaries(summaries)
        } else {
            agent_state::OpenedAgentStates::Bodies(
                read_agent_states(runtime, &workspace_id, references).map_err(failed)?,
            )
        };
    let (summaries, bodies, body_bytes) = match &agent_states {
        agent_state::OpenedAgentStates::Summaries(summaries) => (summaries.len(), 0, 0),
        agent_state::OpenedAgentStates::Bodies(bodies) => (
            0,
            bodies.len(),
            bodies
                .iter()
                .map(|(_, content)| content.len())
                .sum::<usize>(),
        ),
    };
    tracing::info!(
        workspace = workspace_id.as_str(),
        opened_millis,
        total_millis = started.elapsed().as_millis() as u64,
        agent_summaries = summaries,
        agent_states = bodies,
        agent_state_bytes = body_bytes,
        "workspace opened"
    );
    let HostConnectionState::Ready {
        host_epoch,
        connection_sequence,
    } = runtime.state()
    else {
        return Err(failed("the Host disconnected".into()));
    };
    Ok(OpenedWorkspace {
        layout,
        documents,
        agent_states,
        host_epoch,
        connection_sequence,
    })
}

fn read_agent_states(
    runtime: &crate::host_runtime::DesktopHostRuntime,
    workspace_id: &WorkspaceId,
    references: Vec<AgentStateRef>,
) -> Result<agent_state::AgentStateBodies, String> {
    agent_state::read_published(references, |reference| {
        runtime.request_blocking_typed(yttt_protocol::Request::Workspace(
            WorkspaceRequest::GetAgentState {
                workspace_id: workspace_id.clone(),
                reference: reference.clone(),
            },
        ))
    })
}

/// Takes `workspace_id` out of the next startup. The layout revision can move
/// between reading and writing it, so a conflict is retried once.
fn leave_startup(
    runtime: &crate::host_runtime::DesktopHostRuntime,
    workspace_id: WorkspaceId,
) -> Result<(), String> {
    let mut attempts = 0;
    loop {
        attempts += 1;
        let WorkspaceResponse::Workspaces(entries) =
            runtime.workspace_request(WorkspaceRequest::List)?
        else {
            return Err("Host did not return its workspace index".into());
        };
        let Some(entry) = entries
            .into_iter()
            .find(|entry| entry.workspace_id == workspace_id)
        else {
            return Ok(());
        };
        if !entry.restore_on_startup {
            return Ok(());
        }
        match runtime.workspace_request(WorkspaceRequest::SetRestoreOnStartup {
            workspace_id: workspace_id.clone(),
            expected_revision: entry.revision,
            restore: false,
        }) {
            Ok(_) => return Ok(()),
            Err(error) if attempts >= 2 => return Err(error),
            Err(_) => {}
        }
    }
}

fn remote_host_epoch(runtime: &crate::host_runtime::DesktopHostRuntime) -> Option<u64> {
    match runtime.state() {
        HostConnectionState::Ready { host_epoch, .. } => Some(host_epoch),
        _ => None,
    }
}

impl WorkbenchView {
    /// Running terminal panes of this window that are read-only because
    /// another Client holds their terminal.
    fn terminals_observed_elsewhere(&self, cx: &App) -> usize {
        self.terminal
            .terminal_panes
            .values()
            .filter(|pane| pane.read(cx).observes_another_controller(cx))
            .count()
    }
}

#[cfg(test)]
mod outage_notice_tests {
    use super::*;

    fn outage(attempt: Option<u32>) -> crate::host_runtime::HostOutage {
        crate::host_runtime::HostOutage {
            reason: "lost".into(),
            since_unix_millis: 0,
            attempt,
        }
    }

    #[test]
    fn a_short_outage_dims_the_window_before_a_dialog_covers_it() {
        let reconnecting = outage(Some(2));
        assert_eq!(host_outage_notice(&reconnecting, 500), None);
        assert_eq!(
            host_outage_notice(&reconnecting, HOST_OUTAGE_NOTICE_DELAY_MILLIS),
            Some(HostOutageNotice::Brief)
        );
        assert_eq!(
            host_outage_notice(&reconnecting, HOST_OUTAGE_DIALOG_DELAY_MILLIS - 1),
            Some(HostOutageNotice::Brief)
        );
        assert_eq!(
            host_outage_notice(&reconnecting, HOST_OUTAGE_DIALOG_DELAY_MILLIS),
            Some(HostOutageNotice::Dialog)
        );
        // A Client that gave up says so at once.
        assert_eq!(
            host_outage_notice(&outage(None), 0),
            Some(HostOutageNotice::Dialog)
        );
    }

    #[test]
    fn a_host_that_stops_answering_dims_the_window_before_the_session_is_lost() {
        let health = |waited: u64, unstable: bool| yttt_client_core::LinkHealth {
            round_trip: Some(Duration::from_millis(40)),
            unstable,
            waiting_for: Some(Duration::from_secs(waited)),
        };
        assert_eq!(host_link_notice(None, 0, None, false), None);
        assert_eq!(
            host_link_notice(None, 0, Some(health(3, false)), false),
            None
        );
        assert_eq!(
            host_link_notice(None, 0, Some(health(7, true)), false),
            Some(HostLinkNotice::Stalled {
                waited: Duration::from_secs(7)
            })
        );
        // Once lost, it stays dimmed through the delay a fresh outage waits.
        assert_eq!(
            host_link_notice(Some(outage(Some(1))), 100, None, true),
            Some(HostLinkNotice::Outage(
                outage(Some(1)),
                HostOutageNotice::Brief
            ))
        );
        assert_eq!(
            host_link_notice(Some(outage(Some(1))), 100, None, false),
            None
        );
    }
}
