use std::{
    collections::{HashMap, HashSet},
    path::{Component, PathBuf},
    sync::Arc,
    time::{Duration, UNIX_EPOCH},
};

use gpui::{Context, Task, Window};
use serde::{Deserialize, Serialize};
use yttt_client_core::ConnectionState as HostConnectionState;
use yttt_core::model::{
    ids::ProjectId,
    project::ProjectLocation,
    workspace::{RemoteResourceLoss, Workspace, WorkspaceState},
};
use yttt_protocol::{
    HostPath,
    project::{ContentRevision, ProjectFileFingerprint},
    workspace::{
        DraftBase, DraftContentRevision, DraftRef, MAX_DRAFT_CONTENT_BYTES,
        MAX_WORKSPACE_DRAFT_BYTES, WorkspaceId, WorkspaceOperationId, WorkspaceRequest,
        WorkspaceResponse, WorkspaceSnapshot,
    },
};

use crate::ui::editor::{
    CodeEditorConfig, CodeEditorLanguageMode, CodeEditorState, DiskFingerprint, DocumentId,
    EditorSelectionSnapshot, ProjectEditorDocument, ProjectEditorModel,
    ProjectEditorWorkspaceSnapshot,
};

use super::*;

const REMOTE_WORKSPACE_SCHEMA_VERSION: u16 = 1;
const MAX_REMOTE_WORKSPACE_SNAPSHOT_BYTES: usize = 1024 * 1024;
const WORKSPACE_PERSISTENCE_INTERVAL: Duration = Duration::from_millis(500);

type DraftUpload = (DraftRef, Arc<Vec<u8>>);
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum WorkspacePersistenceMode {
    #[default]
    Inactive,
    AwaitingControl,
    Loading,
    Restoring,
    Active,
    Observer,
    ControlLost,
}

struct PendingWorkspaceCommit {
    expected_revision: u64,
    operation_id: WorkspaceOperationId,
    snapshot: serde_json::Value,
    drafts: Arc<Vec<DraftUpload>>,
}

pub(super) struct WorkspacePersistenceState {
    view: Option<crate::session_coordinator::WorkspaceViewLease>,
    frozen_transfer: Option<String>,
    draft_cache: std::cell::RefCell<HashMap<DocumentId, (u64, DraftUpload)>>,
    confirmed_drafts: HashSet<[u8; 32]>,
    mode: WorkspacePersistenceMode,
    revision: Option<u64>,
    host_epoch: Option<u64>,
    initial_project: Option<PathBuf>,
    available_terminal_sessions: HashSet<String>,
    last_committed_snapshot: Option<serde_json::Value>,
    pending_commit: Option<PendingWorkspaceCommit>,
    control_request_in_flight: bool,
    commit_in_flight: bool,
    tick_task: Option<Task<()>>,
    pending_document_restores: usize,
    pending_editor_snapshot: Option<ProjectEditorWorkspaceSnapshot>,
    close_flush_requested: bool,
    close_prompt_open: bool,
    last_error: Option<String>,
    resource_loss_message: Option<String>,
}

impl Default for WorkspacePersistenceState {
    fn default() -> Self {
        Self {
            view: None,
            frozen_transfer: None,
            draft_cache: std::cell::RefCell::new(HashMap::new()),
            confirmed_drafts: HashSet::new(),
            mode: WorkspacePersistenceMode::Inactive,
            revision: None,
            host_epoch: None,
            initial_project: None,
            available_terminal_sessions: HashSet::new(),
            last_committed_snapshot: None,
            pending_commit: None,
            control_request_in_flight: false,
            commit_in_flight: false,
            tick_task: None,
            pending_document_restores: 0,
            pending_editor_snapshot: None,
            last_error: None,
            resource_loss_message: None,
            close_flush_requested: false,
            close_prompt_open: false,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct RemoteWorkspaceSnapshot {
    schema_version: u16,
    workspace: WorkspaceState,
    editor: ProjectEditorWorkspaceSnapshot,
    documents: Vec<RemoteDocumentSnapshot>,
}

impl RemoteWorkspaceSnapshot {
    fn empty() -> Self {
        Self {
            schema_version: REMOTE_WORKSPACE_SCHEMA_VERSION,
            workspace: Workspace::new().persisted_state(),
            editor: ProjectEditorWorkspaceSnapshot::default(),
            documents: Vec::new(),
        }
    }

    fn from_value(value: serde_json::Value) -> Result<Self, String> {
        if value.as_object().is_some_and(|object| object.is_empty()) {
            return Ok(Self::empty());
        }
        let snapshot: Self = serde_json::from_value(value)
            .map_err(|error| format!("remote workspace snapshot is invalid: {error}"))?;
        if snapshot.schema_version != REMOTE_WORKSPACE_SCHEMA_VERSION {
            return Err(format!(
                "remote workspace snapshot schema {} is unsupported",
                snapshot.schema_version
            ));
        }
        let workspace = Workspace::restore_persisted_state(snapshot.workspace.clone())
            .map_err(|error| format!("remote workspace core state is invalid: {error}"))?;
        let mut project_roots = HashMap::new();
        for project in workspace.opened_projects() {
            let root = project.location.local_path().ok_or_else(|| {
                format!(
                    "remote workspace project {} does not have a Host filesystem root",
                    project.id.as_str()
                )
            })?;
            project_roots.insert(project.id.clone(), root.clone());
        }
        snapshot
            .editor
            .validate(&project_roots, &terminal_ids_by_project(&workspace))
            .map_err(|error| format!("remote workspace editor state is invalid: {error}"))?;
        let projects = snapshot
            .workspace
            .opened_projects
            .iter()
            .map(|project| project.id.clone())
            .collect::<HashSet<_>>();
        if snapshot.editor.project_ids() != projects {
            return Err(
                "remote workspace editor sessions do not match the opened projects".to_string(),
            );
        }
        let mut document_ids = HashSet::new();
        for document in &snapshot.documents {
            if !projects.contains(&document.project_id) {
                return Err(format!(
                    "restored document belongs to unopened project {}",
                    document.project_id.as_str()
                ));
            }
            validate_relative_path(&document.relative_path)?;
            let root = snapshot
                .workspace
                .opened_projects
                .iter()
                .find(|project| project.id == document.project_id)
                .and_then(|project| project.location.local_path())
                .ok_or_else(|| {
                    format!(
                        "restored document project {} has no Host filesystem root",
                        document.project_id.as_str()
                    )
                })?;
            if !snapshot.editor.contains_document(&DocumentId {
                project_id: document.project_id.clone(),
                canonical_path: root.join(&document.relative_path),
            }) {
                return Err(format!(
                    "restored document {} is not open in its editor session",
                    document.relative_path.display()
                ));
            }
            if !document_ids.insert((document.project_id.clone(), document.relative_path.clone())) {
                return Err("remote workspace snapshot contains duplicate documents".to_string());
            }
        }
        Ok(snapshot)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct RemoteDocumentSnapshot {
    project_id: ProjectId,
    relative_path: PathBuf,
    selection: EditorSelectionSnapshot,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    draft_ref: Option<DraftRef>,
    #[serde(default, skip_serializing)]
    draft: Option<DraftContentRevision>,
}

struct PreparedRemoteRestore {
    snapshot: RemoteWorkspaceSnapshot,
    server_snapshot: serde_json::Value,
    revision: u64,
    host_epoch: u64,
    services: HashMap<ProjectId, ProjectServices>,
    available_terminal_sessions: HashSet<String>,
    unavailable_terminal_ids: HashMap<ProjectId, HashSet<String>>,
    losses: Vec<RemoteResourceLoss>,
}

impl WorkbenchView {
    pub(super) fn workspace_is_loading(&self) -> bool {
        matches!(
            self.workspace_persistence.mode,
            WorkspacePersistenceMode::AwaitingControl
                | WorkspacePersistenceMode::Loading
                | WorkspacePersistenceMode::Restoring
        )
    }

    pub(super) fn profile_control_banner(&self, cx: &mut Context<Self>) -> Div {
        let Some(runtime) = self.terminal.host_runtime.clone() else {
            return div();
        };
        let Some(status) = runtime.control_status() else {
            return div().child("连接已断开 · 未同步");
        };
        let appearance = self.theme_runtime();
        let observer = !runtime.is_controller();
        let identity = if runtime.is_remote() {
            format!(
                "远程 · {}",
                runtime
                    .remote_label()
                    .and_then(|label| label.rsplit(" · ").next())
                    .unwrap_or("Host")
            )
        } else {
            "本地".to_string()
        };
        let state = if self.workspace_persistence.mode == WorkspacePersistenceMode::ControlLost {
            " · 未同步，编辑已保留"
        } else if status.transfer.is_some() {
            " · 正在交接"
        } else if self.workspace_is_loading() {
            " · 正在恢复"
        } else if observer {
            " · 只读"
        } else if self.settings_save_pending() {
            " · 正在保存"
        } else {
            ""
        };
        let mut banner = div()
            .flex()
            .items_center()
            .gap(appearance.style.spacing.sm)
            .px(appearance.style.spacing.md)
            .text_sm()
            .debug_selector(|| "profile-control-status".to_string())
            .child(format!("{identity}{state}"));
        if observer {
            let force = status.transfer.as_ref().is_some_and(|transfer| {
                transfer.phase == yttt_protocol::session::TransferPhase::ForceConfirmationRequired
            });
            banner = banner.child(
                yttt_button(
                    "profile-request-control",
                    if force {
                        "强制接管…"
                    } else {
                        "在此继续"
                    },
                    YtttButtonVariant::Secondary,
                    appearance.ui,
                    appearance.style,
                    cx,
                )
                .on_click(
                    cx.listener(|root, _, window, cx| root.request_profile_control(window, cx)),
                ),
            );
        }
        banner
    }

    fn request_profile_control(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        use yttt_protocol::{
            Request, Response,
            session::{ProfileControlRequest, TransferPhase},
        };
        let Some(runtime) = self.terminal.host_runtime.clone() else {
            return;
        };
        let transfer = runtime
            .control_status()
            .and_then(|status| status.transfer)
            .filter(|transfer| transfer.phase == TransferPhase::ForceConfirmationRequired);
        let prompt = window.prompt(gpui::PromptLevel::Warning,
            if transfer.is_some() { "Force control of this profile?" } else { "Continue this entire profile here?" },
            Some(if transfer.is_some() { "Only the last confirmed state will be restored. Unpublished edits on the previous client may be missing." }
                else { "All workspaces transfer together. The previous client will publish its edits and become an observer. Running terminal processes are not restarted." }),
            &["Cancel", "Continue"], cx);
        cx.spawn_in(window, async move |this, cx| {
            let accepted = matches!(prompt.await, Ok(1));
            let request = match (accepted, transfer) {
                (true, Some(transfer)) => ProfileControlRequest::ConfirmForce {
                    transfer_id: transfer.id,
                },
                (true, None) => ProfileControlRequest::RequestControl,
                (false, Some(transfer)) => ProfileControlRequest::Cancel {
                    transfer_id: transfer.id,
                },
                (false, None) => return,
            };
            let result = runtime
                .request(Request::ProfileControl(request))
                .recv_async()
                .await;
            let _ = this.update_in(cx, |root, _, cx| {
                match result {
                    Ok(Ok(Response::ProfileControl(_))) => {}
                    Ok(Err(error)) => root.set_workspace_persistence_error(error.to_string()),
                    Err(error) => root.set_workspace_persistence_error(error.to_string()),
                    Ok(Ok(_)) => {
                        root.set_workspace_persistence_error("Unexpected control response".into())
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

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
            match runtime.claim_workspace_view(restore_existing) {
                Ok(view) => self.workspace_persistence.view = Some(view),
                Err(error) => {
                    self.set_workspace_persistence_error(error);
                    return;
                }
            }
        }
        let workspace_id = self
            .workspace_persistence
            .view
            .as_ref()
            .expect("claimed view")
            .id()
            .clone();

        if self.workspace_persistence.mode == WorkspacePersistenceMode::Inactive {
            self.workspace_persistence.initial_project = self
                .workspace
                .opened_projects()
                .first()
                .and_then(|project| project.location.local_path())
                .cloned();
            self.workspace_persistence.mode = WorkspacePersistenceMode::AwaitingControl;
            self.start_workspace_persistence_tick(window, cx);
        }
        self.request_workspace_control_and_open(runtime, workspace_id, window, cx);
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
                    })
                    .is_err()
                {
                    break;
                }
            }
        }));
    }

    fn tick_workspace_persistence(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(runtime) = self.terminal.host_runtime.clone() else {
            self.workspace_persistence.mode = WorkspacePersistenceMode::AwaitingControl;
            self.workspace_persistence.host_epoch = None;
            return;
        };
        let Some(workspace_id) = self
            .workspace_persistence
            .view
            .as_ref()
            .map(|view| view.id().clone())
        else {
            self.workspace_persistence.mode = WorkspacePersistenceMode::ControlLost;
            self.set_workspace_persistence_error(
                "Remote Host lost its workspace identity".to_string(),
            );
            return;
        };
        let Some(host_epoch) = remote_host_epoch(&runtime) else {
            self.workspace_persistence.mode = WorkspacePersistenceMode::AwaitingControl;
            self.workspace_persistence.host_epoch = None;
            return;
        };
        if !runtime.is_controller() {
            if self.workspace_persistence.mode == WorkspacePersistenceMode::Active {
                let unpublished =
                    self.build_remote_workspace_snapshot(cx)
                        .ok()
                        .is_none_or(|snapshot| {
                            self.workspace_persistence.last_committed_snapshot.as_ref()
                                != Some(&snapshot.0)
                        });
                self.workspace_persistence.mode = if unpublished {
                    WorkspacePersistenceMode::ControlLost
                } else {
                    WorkspacePersistenceMode::Observer
                };
                self.workspace_persistence.pending_commit = None;
            }
            if self.workspace_persistence.mode == WorkspacePersistenceMode::Observer {
                self.request_workspace_control_and_open(runtime, workspace_id, window, cx);
            }
            return;
        }
        if self.workspace_persistence.mode == WorkspacePersistenceMode::ControlLost {
            return;
        }
        if self.workspace_persistence.mode == WorkspacePersistenceMode::Observer {
            self.workspace_persistence.mode = WorkspacePersistenceMode::AwaitingControl;
        }
        let transfer = runtime.preparing_transfer();
        if transfer != self.workspace_persistence.frozen_transfer {
            self.workspace_persistence.frozen_transfer = transfer.clone();
            if transfer.is_some() {
                window.blur();
            }
            cx.notify();
        }
        if self.workspace_persistence.mode == WorkspacePersistenceMode::Active
            && self.workspace_persistence.host_epoch != Some(host_epoch)
        {
            self.workspace_persistence.mode = WorkspacePersistenceMode::AwaitingControl;
            self.workspace_persistence.host_epoch = None;
        }
        if self.workspace_persistence.mode == WorkspacePersistenceMode::AwaitingControl {
            self.request_workspace_control_and_open(runtime, workspace_id, window, cx);
            return;
        }
        if self.workspace_persistence.mode == WorkspacePersistenceMode::Active {
            self.commit_remote_workspace_if_changed(runtime, workspace_id, window, cx);
            let publication = self.build_remote_workspace_snapshot(cx);
            let error = publication
                .as_ref()
                .err()
                .cloned()
                .or_else(|| self.workspace_persistence.last_error.clone())
                .or_else(|| self.settings.settings_save_error.clone());
            let clean = !self.settings_save_pending()
                && publication.is_ok_and(|snapshot| {
                    self.workspace_persistence.last_committed_snapshot.as_ref() == Some(&snapshot.0)
                })
                && !self.workspace_persistence.commit_in_flight
                && self.workspace_persistence.pending_commit.is_none();
            if let Some(view) = &self.workspace_persistence.view {
                view.published(
                    transfer,
                    clean
                        .then_some(self.workspace_persistence.revision)
                        .flatten(),
                    error,
                );
            }
        }
    }

    fn request_workspace_control_and_open(
        &mut self,
        runtime: Arc<crate::host_runtime::DesktopHostRuntime>,
        workspace_id: WorkspaceId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.workspace_persistence.control_request_in_flight
            || self.workspace_persistence.mode == WorkspacePersistenceMode::ControlLost
        {
            return;
        }
        let known_revision = self.workspace_persistence.revision;
        let known_epoch = self.workspace_persistence.host_epoch;
        let preserve_local = self.workspace_persistence.mode != WorkspacePersistenceMode::Observer
            && known_revision.is_some()
            && self
                .build_remote_workspace_snapshot(cx)
                .ok()
                .is_none_or(|snapshot| {
                    self.workspace_persistence.last_committed_snapshot.as_ref() != Some(&snapshot.0)
                });
        self.workspace_persistence.control_request_in_flight = true;
        self.workspace_persistence.mode = WorkspacePersistenceMode::Loading;
        let request_workspace_id = workspace_id.clone();
        let local_import = !runtime.is_remote();
        let open_task = cx.background_spawn(async move {
            acquire_control_and_open(&runtime, request_workspace_id.clone()).and_then(
                |(server_snapshot, revision)| {
                    if known_revision == Some(revision)
                        && known_epoch == remote_host_epoch(&runtime)
                    {
                        Ok(None)
                    } else {
                        prepare_remote_restore(
                            &runtime,
                            &request_workspace_id,
                            server_snapshot,
                            revision,
                        )
                        .map(Some)
                    }
                },
            )
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = open_task.await;
            let _ = this.update_in(cx, |root, window, cx| {
                root.workspace_persistence.control_request_in_flight = false;
                match result {
                    Ok(None) => {
                        root.workspace_persistence.mode = root.restored_workspace_mode();
                    }
                    Ok(Some(prepared)) if local_import && prepared.revision == 0 && root.workspace_persistence.revision.is_none() => {
                        root.workspace_persistence.revision = Some(0);
                        root.workspace_persistence.host_epoch = Some(prepared.host_epoch);
                        root.workspace_persistence.mode = root.restored_workspace_mode();
                        root.clear_workspace_persistence_error();
                    }
                    Ok(Some(prepared)) if root.workspace_persistence.revision.is_none() || !preserve_local => {
                        root.install_remote_workspace_restore(prepared, window, cx);
                    }
                    Ok(Some(prepared)) if root.workspace_persistence.pending_commit.is_some() => {
                        root.workspace_persistence.revision = Some(prepared.revision);
                        root.workspace_persistence.host_epoch = Some(prepared.host_epoch);
                        root.workspace_persistence.available_terminal_sessions =
                            prepared.available_terminal_sessions.clone();
                        root.project.services = prepared.services;
                        root.workspace_persistence.mode = root.restored_workspace_mode();
                        root.clear_workspace_persistence_error();
                        root.reconcile_remote_host_resources(
                            &prepared.available_terminal_sessions,
                            window,
                            cx,
                        );
                    }
                    Ok(Some(prepared))
                        if root.workspace_persistence.revision == Some(prepared.revision) =>
                    {
                        root.workspace_persistence.host_epoch = Some(prepared.host_epoch);
                        root.workspace_persistence.available_terminal_sessions =
                            prepared.available_terminal_sessions.clone();
                        root.project.services = prepared.services;
                        root.workspace_persistence.mode = root.restored_workspace_mode();
                        root.clear_workspace_persistence_error();
                        root.reconcile_remote_host_resources(
                            &prepared.available_terminal_sessions,
                            window,
                            cx,
                        );
                    }
                    Ok(Some(prepared)) => {
                        root.workspace_persistence.mode = WorkspacePersistenceMode::ControlLost;
                        root.set_workspace_persistence_error(format!(
                            "Remote workspace changed from revision {} to {} while this Client was disconnected; control was not resumed",
                            root.workspace_persistence.revision.unwrap_or_default(),
                            prepared.revision
                        ));
                    }
                    Err(error) => {
                        root.workspace_persistence.mode = if is_workspace_control_conflict(&error) {
                            WorkspacePersistenceMode::ControlLost
                        } else {
                            WorkspacePersistenceMode::AwaitingControl
                        };
                        root.set_workspace_persistence_error(format!(
                            "Remote workspace control is unavailable: {error}"
                        ));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn install_remote_workspace_restore(
        &mut self,
        prepared: PreparedRemoteRestore,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let workspace =
            match Workspace::restore_persisted_state(prepared.snapshot.workspace.clone()) {
                Ok(workspace) => workspace,
                Err(error) => {
                    self.workspace_persistence.mode = WorkspacePersistenceMode::ControlLost;
                    self.set_workspace_persistence_error(format!(
                        "Remote workspace state could not be restored: {error}"
                    ));
                    return;
                }
            };
        self.workspace = workspace;
        self.workspace_persistence.draft_cache.borrow_mut().clear();
        self.reset_remote_agent_snapshots();
        self.project.services = prepared.services;
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

        let terminal_ids = terminal_ids_by_project(&self.workspace);
        if let Err(error) = self
            .project
            .project_editor_runtime
            .workspace_mut()
            .restore_snapshot(
                prepared.snapshot.editor.clone(),
                &terminal_ids,
                &prepared.unavailable_terminal_ids,
            )
        {
            self.workspace_persistence.mode = WorkspacePersistenceMode::ControlLost;
            self.set_workspace_persistence_error(format!(
                "Remote editor layout could not be restored: {error}"
            ));
            return;
        }

        self.workspace_persistence.revision = Some(prepared.revision);
        self.workspace_persistence.host_epoch = Some(prepared.host_epoch);
        self.workspace_persistence.available_terminal_sessions =
            prepared.available_terminal_sessions.clone();
        self.workspace_persistence.confirmed_drafts = prepared
            .snapshot
            .documents
            .iter()
            .filter_map(|document| {
                document
                    .draft_ref
                    .as_ref()
                    .map(|reference| reference.content_sha256)
            })
            .collect();
        self.workspace_persistence.last_committed_snapshot = Some(prepared.server_snapshot);
        self.workspace_persistence.pending_commit = None;
        self.clear_workspace_persistence_error();
        self.workspace_persistence.pending_document_restores = prepared.snapshot.documents.len();
        self.workspace_persistence.pending_editor_snapshot = Some(prepared.snapshot.editor.clone());
        self.workspace_persistence.mode = WorkspacePersistenceMode::Restoring;
        self.report_lost_remote_resources(&prepared.losses);

        if prepared.snapshot.documents.is_empty() {
            self.finish_remote_document_restore(cx);
            return;
        }
        for document in prepared.snapshot.documents {
            self.spawn_remote_document_restore(document, window, cx);
        }
    }

    fn spawn_remote_document_restore(
        &mut self,
        document: RemoteDocumentSnapshot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(request) =
            self.begin_project_file_open(&document.project_id, &document.relative_path)
        else {
            self.set_workspace_persistence_error(format!(
                "Could not queue restored editor file {}",
                document.relative_path.display()
            ));
            self.finish_remote_document_restore(cx);
            return;
        };
        let Some(services) = self.project.services.get(&document.project_id).cloned() else {
            self.cancel_project_file_open(&request);
            self.set_workspace_persistence_error(format!(
                "Remote project service is unavailable for restored file {}",
                document.relative_path.display()
            ));
            self.finish_remote_document_restore(cx);
            return;
        };
        let relative_path = request.relative_path.clone();
        let load_task = cx.background_spawn(async move { services.read_file(&relative_path) });
        cx.spawn_in(window, async move |this, cx| {
            let result = load_task.await;
            let _ = this.update_in(cx, |root, window, cx| {
                match result {
                    Ok(loaded) => {
                        root.apply_project_file_open_success(&request, loaded, window, cx);
                        root.apply_remote_document_state(
                            &request.document_id,
                            &document,
                            window,
                            cx,
                        );
                    }
                    Err(error) => {
                        root.apply_project_file_open_error(&request, error.to_string());
                        if document.draft.is_some() {
                            root.restore_missing_remote_draft(&document, window, cx);
                        } else {
                            root.set_workspace_persistence_error(format!(
                                "Restored editor file {} is unavailable: {error}",
                                document.relative_path.display()
                            ));
                        }
                    }
                }
                root.finish_remote_document_restore(cx);
                cx.notify();
            });
        })
        .detach();
    }

    fn apply_remote_document_state(
        &mut self,
        document_id: &DocumentId,
        snapshot: &RemoteDocumentSnapshot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(document) = self
            .project
            .project_editor_runtime
            .document(document_id)
            .cloned()
        else {
            self.set_workspace_persistence_error(format!(
                "Restored editor document {} was not created",
                snapshot.relative_path.display()
            ));
            return;
        };
        let mut conflict_message = None;
        document.update(cx, |document, document_cx| {
            if let Some(draft) = snapshot.draft.as_ref() {
                let matches_base = draft_matches_document(draft, document);
                if !matches_base {
                    conflict_message = Some(format!(
                        "Recovered draft for {} conflicts with the current remote file; review before saving",
                        snapshot.relative_path.display()
                    ));
                }
                document.restore_recovered_draft(
                    draft.content.clone(),
                    draft.revision,
                    &snapshot.selection,
                    conflict_message.clone(),
                    window,
                    document_cx,
                );
            } else {
                document.restore_selection(&snapshot.selection, window, document_cx);
            }
        });
        if let Some(message) = conflict_message {
            self.set_workspace_persistence_error(message);
        }
    }

    fn restore_missing_remote_draft(
        &mut self,
        snapshot: &RemoteDocumentSnapshot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(project_root) = self
            .workspace
            .project(&snapshot.project_id)
            .and_then(|project| project.location.local_path())
            .cloned()
        else {
            self.set_workspace_persistence_error(format!(
                "Cannot recover draft {} because its project is unavailable",
                snapshot.relative_path.display()
            ));
            return;
        };
        let document_id = DocumentId {
            project_id: snapshot.project_id.clone(),
            canonical_path: project_root.join(&snapshot.relative_path),
        };
        if self
            .project
            .project_editor_runtime
            .document(&document_id)
            .is_some()
        {
            self.apply_remote_document_state(&document_id, snapshot, window, cx);
            return;
        }
        let language_mode = if self.app_settings.editor.auto_detect_language {
            CodeEditorLanguageMode::Auto
        } else {
            CodeEditorLanguageMode::from(self.app_settings.editor.default_language.clone())
        };
        let title = snapshot
            .relative_path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| snapshot.relative_path.to_string_lossy().into_owned());
        let config = CodeEditorConfig::new(title.clone(), language_mode)
            .with_editor_settings(&self.app_settings.editor);
        let mut model = ProjectEditorModel::new(
            document_id.clone(),
            CodeEditorState::new(&document_id.canonical_path, config, String::new()),
            missing_disk_fingerprint(),
        );
        model.mark_missing_on_disk();
        let appearance = EditorAppearance::from(&self.app_settings.editor);
        let markdown_config = self.markdown_document_config();
        let vim_enabled = self.app_settings.vim.mode != VimModeSetting::Disabled;
        let editor = cx.new(|document_cx| {
            ProjectEditorDocument::new_with_markdown_config(
                model,
                appearance,
                markdown_config,
                window,
                document_cx,
            )
            .with_breadcrumb_header(snapshot.relative_path.to_string_lossy().into_owned())
            .with_vim_mode(vim_enabled, window, document_cx)
        });
        let subscription = cx.subscribe_in(&editor, window, Self::on_project_editor_document_event);
        self.project.project_editor_runtime.insert_document(
            document_id.clone(),
            editor,
            subscription,
        );
        self.apply_remote_document_state(&document_id, snapshot, window, cx);
        self.set_workspace_persistence_error(format!(
            "Recovered draft {} has no current remote base file; it remains unsaved",
            snapshot.relative_path.display()
        ));
    }

    fn finish_remote_document_restore(&mut self, cx: &mut Context<Self>) {
        self.workspace_persistence.pending_document_restores = self
            .workspace_persistence
            .pending_document_restores
            .saturating_sub(1);
        if self.workspace_persistence.pending_document_restores != 0 {
            return;
        }
        let Some(editor_snapshot) = self.workspace_persistence.pending_editor_snapshot.take()
        else {
            self.workspace_persistence.mode = self.restored_workspace_mode();
            return;
        };
        let terminal_ids = terminal_ids_by_project(&self.workspace);
        let unavailable = unavailable_terminal_ids(
            &self.workspace,
            &self.workspace_persistence.available_terminal_sessions,
        );
        if let Err(error) = self
            .project
            .project_editor_runtime
            .workspace_mut()
            .restore_snapshot(editor_snapshot, &terminal_ids, &unavailable)
        {
            self.workspace_persistence.mode = WorkspacePersistenceMode::ControlLost;
            self.set_workspace_persistence_error(format!(
                "Remote editor layout could not be finalized: {error}"
            ));
            return;
        }
        self.workspace_persistence.mode = self.restored_workspace_mode();
        if let Some(path) = self.workspace_persistence.initial_project.take()
            && self.workspace.opened_projects().is_empty()
            && let Err(error) = self.open_project_path(path)
        {
            self.set_workspace_persistence_error(format!(
                "Could not open the initial remote project: {error}"
            ));
        }
        cx.notify();
    }

    fn reset_remote_agent_snapshots(&mut self) {
        let snapshots = self
            .workspace
            .opened_projects()
            .iter()
            .flat_map(|project| {
                project.tab_states.iter().flat_map(move |tab| {
                    tab.pane_states.iter().filter_map(move |pane| {
                        pane.agent_snapshot.clone().map(|snapshot| {
                            (
                                AgentPaneAddress::new(
                                    project.id.as_str(),
                                    &tab.tab_id,
                                    &pane.pane_id,
                                ),
                                snapshot,
                            )
                        })
                    })
                })
            })
            .collect();
        self.agent_manager.reset_for_host_restore(snapshots);
    }

    fn reconcile_remote_host_resources(
        &mut self,
        available_terminal_sessions: &HashSet<String>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        self.workspace_persistence.available_terminal_sessions =
            available_terminal_sessions.clone();
        self.terminal.terminal_panes.clear();
        self.terminal.terminal_pane_subscriptions.clear();
        self.terminal.pending_terminal_focus = None;
        let losses = self
            .workspace
            .reconcile_host_resources(available_terminal_sessions);
        self.reset_remote_agent_snapshots();
        let terminal_ids = terminal_ids_by_project(&self.workspace);
        let unavailable = unavailable_terminal_ids(&self.workspace, available_terminal_sessions);
        self.project
            .project_editor_runtime
            .workspace_mut()
            .set_unavailable_terminal_ids(&unavailable, &terminal_ids);
        self.report_lost_remote_resources(&losses);
    }

    fn report_lost_remote_resources(&mut self, losses: &[RemoteResourceLoss]) {
        if losses.is_empty() {
            return;
        }
        self.set_workspace_resource_loss(format!(
            "Remote Host no longer has {} restored terminal resource{}; they were not restarted",
            losses.len(),
            if losses.len() == 1 { "" } else { "s" }
        ));
    }

    /// Attempts a final CAS-backed remote snapshot flush before closing.
    ///
    /// Returning `false` means the caller must keep the window open while a
    /// commit is in flight. Unconfirmed changes require explicit discard confirmation.
    fn confirm_close_without_remote_save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.workspace_persistence.close_flush_requested = false;
        if self.workspace_persistence.close_prompt_open {
            return;
        }
        self.workspace_persistence.close_prompt_open = true;
        let answer = window.prompt(
            gpui::PromptLevel::Warning,
            "Remote changes have not been saved",
            self.workspace_persistence.last_error.as_deref(),
            &["Keep window open", "Close without saving"],
            cx,
        );
        cx.spawn_in(window, async move |this, cx| {
            let answer = answer.await;
            let _ = this.update_in(cx, |root, window, _| {
                root.workspace_persistence.close_prompt_open = false;
                if matches!(answer, Ok(1)) {
                    window.remove_window();
                }
            });
        })
        .detach();
    }

    pub(crate) fn flush_workspace_persistence_on_close(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.workspace_persistence.mode == WorkspacePersistenceMode::Inactive {
            return true;
        }
        let Some(runtime) = self.terminal.host_runtime.clone() else {
            self.set_workspace_persistence_error(
                "Remote workspace cannot be flushed because the Host is unavailable".to_string(),
            );
            self.confirm_close_without_remote_save(window, cx);
            return false;
        };
        if self.workspace_persistence.mode == WorkspacePersistenceMode::Observer {
            return true;
        }
        if self.workspace_persistence.mode != WorkspacePersistenceMode::Active {
            self.set_workspace_persistence_error(
                "Remote workspace changes could not be flushed before closing because exclusive Host control is unavailable"
                    .to_string(),
            );
            self.confirm_close_without_remote_save(window, cx);
            return false;
        }
        let current = match self.build_remote_workspace_snapshot(cx) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                self.set_workspace_persistence_error(error);
                self.confirm_close_without_remote_save(window, cx);
                return false;
            }
        };
        if !self.workspace_persistence.commit_in_flight
            && self.workspace_persistence.pending_commit.is_none()
            && self.workspace_persistence.last_committed_snapshot.as_ref() == Some(&current.0)
        {
            return true;
        }
        let Some(workspace_id) = self
            .workspace_persistence
            .view
            .as_ref()
            .map(|view| view.id().clone())
        else {
            self.set_workspace_persistence_error(
                "Remote workspace cannot be flushed because its ID is unavailable".to_string(),
            );
            self.confirm_close_without_remote_save(window, cx);
            return false;
        };
        self.workspace_persistence.close_flush_requested = true;
        self.commit_remote_workspace_if_changed(runtime, workspace_id, window, cx);
        if !self.workspace_persistence.commit_in_flight {
            self.confirm_close_without_remote_save(window, cx);
        }
        false
    }

    fn finish_workspace_persistence_close_if_ready(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.workspace_persistence.close_flush_requested
            || self.workspace_persistence.commit_in_flight
        {
            return;
        }
        if self.workspace_persistence.mode != WorkspacePersistenceMode::Active
            || self.workspace_persistence.last_error.is_some()
        {
            self.confirm_close_without_remote_save(window, cx);
            return;
        }
        let current = match self.build_remote_workspace_snapshot(cx) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                self.set_workspace_persistence_error(error);
                self.confirm_close_without_remote_save(window, cx);
                return;
            }
        };
        if self.workspace_persistence.pending_commit.is_none()
            && self.workspace_persistence.last_committed_snapshot.as_ref() == Some(&current.0)
        {
            self.workspace_persistence.close_flush_requested = false;
            window.remove_window();
            return;
        }
        let Some(runtime) = self.terminal.host_runtime.clone() else {
            self.confirm_close_without_remote_save(window, cx);
            return;
        };
        let Some(workspace_id) = self
            .workspace_persistence
            .view
            .as_ref()
            .map(|view| view.id().clone())
        else {
            self.confirm_close_without_remote_save(window, cx);
            return;
        };
        self.commit_remote_workspace_if_changed(runtime, workspace_id, window, cx);
    }

    fn commit_remote_workspace_if_changed(
        &mut self,
        runtime: Arc<crate::host_runtime::DesktopHostRuntime>,
        workspace_id: WorkspaceId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.workspace_persistence.commit_in_flight {
            return;
        }
        let Some(current_revision) = self.workspace_persistence.revision else {
            self.workspace_persistence.mode = WorkspacePersistenceMode::AwaitingControl;
            return;
        };
        let pending = if let Some(pending) = self.workspace_persistence.pending_commit.as_ref() {
            PendingWorkspaceCommit {
                expected_revision: pending.expected_revision,
                operation_id: pending.operation_id.clone(),
                snapshot: pending.snapshot.clone(),
                drafts: pending.drafts.clone(),
            }
        } else {
            let (snapshot, drafts) = match self.build_remote_workspace_snapshot(cx) {
                Ok(snapshot) => snapshot,
                Err(error) => {
                    self.set_workspace_persistence_error(error);
                    return;
                }
            };
            if self.workspace_persistence.last_committed_snapshot.as_ref() == Some(&snapshot) {
                return;
            }
            let bytes = match serde_json::to_vec(&snapshot) {
                Ok(bytes) => bytes,
                Err(error) => {
                    self.set_workspace_persistence_error(format!(
                        "Remote workspace snapshot cannot be serialized: {error}"
                    ));
                    return;
                }
            };
            if bytes.len() > MAX_REMOTE_WORKSPACE_SNAPSHOT_BYTES {
                self.set_workspace_persistence_error(format!(
                    "Remote workspace snapshot is {} bytes and exceeds the {} byte recovery limit; changes remain unsaved",
                    bytes.len(),
                    MAX_REMOTE_WORKSPACE_SNAPSHOT_BYTES
                ));
                return;
            }
            let operation_id =
                match WorkspaceOperationId::new(format!("workspace-{}", uuid::Uuid::new_v4())) {
                    Ok(operation_id) => operation_id,
                    Err(error) => {
                        self.set_workspace_persistence_error(format!(
                            "Remote workspace operation ID is invalid: {error}"
                        ));
                        return;
                    }
                };
            let pending = PendingWorkspaceCommit {
                expected_revision: current_revision,
                operation_id,
                snapshot,
                drafts: Arc::new(drafts),
            };
            self.workspace_persistence.pending_commit = Some(PendingWorkspaceCommit {
                expected_revision: pending.expected_revision,
                operation_id: pending.operation_id.clone(),
                snapshot: pending.snapshot.clone(),
                drafts: pending.drafts.clone(),
            });
            pending
        };
        let expected_revision = pending.expected_revision;
        let snapshot = match WorkspaceSnapshot::new(pending.snapshot.clone()) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                self.set_workspace_persistence_error(format!(
                    "Remote workspace snapshot is invalid: {error}"
                ));
                return;
            }
        };
        self.workspace_persistence.commit_in_flight = true;
        let request_workspace_id = workspace_id.clone();
        let operation_id = pending.operation_id.clone();
        let drafts = pending.drafts.clone();
        let confirmed_drafts = self.workspace_persistence.confirmed_drafts.clone();
        let commit_task = cx.background_spawn(async move {
            runtime.workspace_request(WorkspaceRequest::Register {
                workspace_id: request_workspace_id.clone(),
                name: request_workspace_id.as_str().to_string(),
            })?;
            let mut references = Vec::with_capacity(drafts.len());
            for (reference, content) in drafts.iter() {
                if !confirmed_drafts.contains(&reference.content_sha256) {
                    runtime.workspace_request(WorkspaceRequest::PutDraft {
                        workspace_id: request_workspace_id.clone(),
                        reference: reference.clone(),
                        content: content.as_ref().clone(),
                    })?;
                }
                references.push(reference.clone());
            }
            runtime.workspace_request(WorkspaceRequest::Commit {
                workspace_id: request_workspace_id,
                expected_revision,
                operation_id,
                snapshot,
                drafts: references,
            })
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = commit_task.await;
            let _ = this.update_in(cx, |root, window, cx| {
                root.workspace_persistence.commit_in_flight = false;
                match result {
                    Ok(WorkspaceResponse::Committed {
                        workspace_id: committed_workspace_id,
                        revision,
                        snapshot,
                    }) if committed_workspace_id == workspace_id => {
                        root.workspace_persistence.revision = Some(revision);
                        root.workspace_persistence.confirmed_drafts = pending.drafts.iter().map(|(reference, _)| reference.content_sha256).collect();
                        root.workspace_persistence.last_committed_snapshot =
                            Some(snapshot.into_value());
                        root.workspace_persistence.pending_commit = None;
                        root.clear_workspace_persistence_error();
                    }
                    Ok(response) => {
                        root.workspace_persistence.mode = WorkspacePersistenceMode::ControlLost;
                        root.set_workspace_persistence_error(format!(
                            "Remote Host returned an unexpected workspace commit response: {response:?}"
                        ));
                    }
                    Err(error) if is_workspace_control_conflict(&error) => {
                        root.workspace_persistence.mode = WorkspacePersistenceMode::ControlLost;
                        root.set_workspace_persistence_error(format!(
                            "Remote workspace commit was rejected; control was lost: {error}"
                        ));
                    }
                    Err(error) if is_remote_connection_error(&error) => {
                        root.workspace_persistence.mode = WorkspacePersistenceMode::AwaitingControl;
                        root.workspace_persistence.host_epoch = None;
                        root.set_workspace_persistence_error(format!(
                            "Remote workspace commit is pending until reconnection: {error}"
                        ));
                    }
                    Err(error) => {
                        root.set_workspace_persistence_error(format!(
                            "Remote workspace commit failed; prior snapshot remains intact: {error}"
                        ));
                    }
                }
                cx.notify();
                root.finish_workspace_persistence_close_if_ready(window, cx);
            });
        })
        .detach();
    }

    fn build_remote_workspace_snapshot(
        &self,
        cx: &Context<Self>,
    ) -> Result<(serde_json::Value, Vec<DraftUpload>), String> {
        let mut documents = Vec::new();
        let mut bodies = Vec::new();
        let mut total_bytes = 0usize;
        let mut cache = self.workspace_persistence.draft_cache.borrow_mut();
        cache.retain(|id, _| {
            self.project
                .project_editor_runtime
                .document(id)
                .is_some_and(|document| document.read(cx).model().is_dirty())
        });
        for project in self.workspace.opened_projects() {
            let root = project.location.local_path().ok_or_else(|| {
                format!(
                    "remote workspace project {} does not have a Host filesystem root",
                    project.id.as_str()
                )
            })?;
            let session = self
                .project
                .project_editor_runtime
                .workspace()
                .session(&project.id)
                .ok_or_else(|| {
                    format!(
                        "remote workspace project {} has no editor session",
                        project.id.as_str()
                    )
                })?;
            for document_id in session.file_ids() {
                let relative_path = document_id
                    .canonical_path
                    .strip_prefix(root)
                    .map_err(|_| {
                        format!(
                            "open document {} is outside project {}",
                            document_id.canonical_path.display(),
                            project.id.as_str()
                        )
                    })?
                    .to_path_buf();
                validate_relative_path(&relative_path)?;
                let (selection, draft_ref) = if let Some(document) =
                    self.project.project_editor_runtime.document(document_id)
                {
                    let document = document.read(cx);
                    let reference = if document.model().is_dirty() {
                        let generation = document.model().generation();
                        if cache
                            .get(document_id)
                            .is_none_or(|(cached, _)| *cached != generation)
                        {
                            let draft = DraftContentRevision {
                                revision: generation,
                                base: DraftBase::File {
                                    path: HostPath::from_path(&document_id.canonical_path)
                                        .map_err(|error| error.to_string())?,
                                    base_fingerprint: disk_fingerprint_to_host(
                                        document.model().disk_fingerprint(),
                                    ),
                                },
                                content: document.current_text(cx),
                            };
                            if draft.content.len() > MAX_DRAFT_CONTENT_BYTES {
                                return Err(
                                    "Document draft exceeds the 6 MiB recovery limit".to_string()
                                );
                            }
                            let key = serde_json::to_vec(&(&project.id, &relative_path))
                                .map_err(|error| error.to_string())?;
                            let reference = DraftRef::for_document(&key, &draft);
                            cache.insert(
                                document_id.clone(),
                                (
                                    generation,
                                    (reference, Arc::new(draft.content.into_bytes())),
                                ),
                            );
                        }
                        let (_, (reference, content)) =
                            cache.get(document_id).expect("captured draft generation");
                        total_bytes = total_bytes.saturating_add(content.len());
                        if total_bytes > MAX_WORKSPACE_DRAFT_BYTES {
                            return Err(
                                "Workspace drafts exceed the 64 MiB recovery limit".to_string()
                            );
                        }
                        bodies.push((reference.clone(), content.clone()));
                        Some(reference.clone())
                    } else {
                        None
                    };
                    (document.selection_snapshot(cx), reference)
                } else {
                    (EditorSelectionSnapshot::default(), None)
                };
                documents.push(RemoteDocumentSnapshot {
                    project_id: project.id.clone(),
                    relative_path,
                    selection,
                    draft_ref,
                    draft: None,
                });
            }
        }
        documents.sort_by(|left, right| {
            left.project_id
                .as_str()
                .cmp(right.project_id.as_str())
                .then_with(|| left.relative_path.cmp(&right.relative_path))
        });
        serde_json::to_value(RemoteWorkspaceSnapshot {
            schema_version: REMOTE_WORKSPACE_SCHEMA_VERSION,
            workspace: self.workspace.persisted_state(),
            editor: self.project.project_editor_runtime.workspace().snapshot(),
            documents,
        })
        .map(|snapshot| (snapshot, bodies))
        .map_err(|error| format!("Remote workspace snapshot cannot be encoded: {error}"))
    }

    fn restored_workspace_mode(&self) -> WorkspacePersistenceMode {
        if self
            .terminal
            .host_runtime
            .as_ref()
            .is_some_and(|runtime| runtime.is_controller())
        {
            WorkspacePersistenceMode::Active
        } else {
            WorkspacePersistenceMode::Observer
        }
    }

    fn set_workspace_resource_loss(&mut self, message: String) {
        if self.workspace_persistence.resource_loss_message.as_deref() == Some(message.as_str()) {
            return;
        }
        if let Some(previous) = self
            .workspace_persistence
            .resource_loss_message
            .replace(message.clone())
        {
            self.load_error = remove_load_message(self.load_error.take(), &previous);
        }
        self.load_error = combine_load_messages(self.load_error.take(), Some(message));
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
    let error = load_error?;
    if error == message {
        return None;
    }
    let prefix = format!("{message}; ");
    if let Some(remainder) = error.strip_prefix(&prefix) {
        return Some(remainder.to_string());
    }
    let suffix = format!("; {message}");
    if let Some(remainder) = error.strip_suffix(&suffix) {
        return Some(remainder.to_string());
    }
    let middle = format!("; {message}; ");
    if let Some(index) = error.find(&middle) {
        let mut retained = error[..index].to_string();
        retained.push_str(&error[index + message.len() + 2..]);
        return Some(retained);
    }
    Some(error)
}

fn acquire_control_and_open(
    runtime: &Arc<crate::host_runtime::DesktopHostRuntime>,
    workspace_id: WorkspaceId,
) -> Result<(serde_json::Value, u64), String> {
    match runtime.workspace_request(WorkspaceRequest::Open { workspace_id })? {
        WorkspaceResponse::Opened {
            snapshot, revision, ..
        } => Ok((snapshot.into_value(), revision)),
        response => Err(format!(
            "Remote Host returned an unexpected workspace open response: {response:?}"
        )),
    }
}

fn prepare_remote_restore(
    runtime: &Arc<crate::host_runtime::DesktopHostRuntime>,
    workspace_id: &WorkspaceId,
    server_snapshot: serde_json::Value,
    revision: u64,
) -> Result<PreparedRemoteRestore, String> {
    let mut snapshot = RemoteWorkspaceSnapshot::from_value(server_snapshot.clone())?;
    for document in &mut snapshot.documents {
        if let Some(reference) = document.draft_ref.as_ref() {
            match runtime.workspace_request(WorkspaceRequest::GetDraft {
                workspace_id: workspace_id.clone(),
                reference: reference.clone(),
            })? {
                WorkspaceResponse::Draft {
                    reference: received,
                    content,
                } if received == *reference => {
                    document.draft = Some(DraftContentRevision {
                        revision: reference.revision,
                        base: reference.base.clone(),
                        content: String::from_utf8(content).map_err(|error| error.to_string())?,
                    });
                }
                _ => return Err("Host returned an unexpected draft body".to_string()),
            }
        }
    }
    let mut workspace = Workspace::restore_persisted_state(snapshot.workspace.clone())
        .map_err(|error| format!("remote workspace core state is invalid: {error}"))?;
    let host_epoch = remote_host_epoch(runtime)
        .ok_or_else(|| "Remote Host is not ready for resource reconciliation".to_string())?;
    let catalog = runtime
        .resource_catalog()
        .ok_or_else(|| "Remote Host resource catalog is unavailable".to_string())?;
    let available_terminal_sessions = catalog
        .terminals
        .iter()
        .map(|terminal| terminal.session_id.as_str().to_string())
        .collect::<HashSet<_>>();
    let losses = workspace.reconcile_host_resources(&available_terminal_sessions);
    let unavailable_terminal_ids =
        unavailable_terminal_ids(&workspace, &available_terminal_sessions);
    snapshot.workspace = workspace.persisted_state();

    let mut services = HashMap::new();
    for project in &snapshot.workspace.opened_projects {
        let ProjectLocation::Local { path } = &project.location else {
            return Err(format!(
                "remote workspace project {} uses a legacy SSH location",
                project.id.as_str()
            ));
        };
        let service = ProjectServices::host(runtime.clone(), project.id.clone(), path.clone())
            .map_err(|error| {
                format!(
                    "Remote Host project registration failed for {}: {error}",
                    path.display()
                )
            })?;
        services.insert(project.id.clone(), service);
    }
    Ok(PreparedRemoteRestore {
        snapshot,
        server_snapshot,
        revision,
        host_epoch,
        services,
        available_terminal_sessions,
        unavailable_terminal_ids,
        losses,
    })
}

fn terminal_ids_by_project(workspace: &Workspace) -> HashMap<ProjectId, Vec<String>> {
    workspace
        .opened_projects()
        .iter()
        .map(|project| {
            (
                project.id.clone(),
                project
                    .layout
                    .tabs
                    .iter()
                    .map(|tab| tab.id.clone())
                    .collect(),
            )
        })
        .collect()
}

fn unavailable_terminal_ids(
    workspace: &Workspace,
    available_terminal_sessions: &HashSet<String>,
) -> HashMap<ProjectId, HashSet<String>> {
    workspace
        .opened_projects()
        .iter()
        .filter_map(|project| {
            let unavailable = project
                .tab_states
                .iter()
                .filter(|tab| {
                    tab.pane_states.iter().any(|pane| {
                        pane.process_state == yttt_core::model::workspace::PaneProcessState::Exited
                            && !available_terminal_sessions.contains(&format!(
                                "{}:{}:{}",
                                project.id, tab.tab_id, pane.pane_id
                            ))
                    })
                })
                .map(|tab| tab.tab_id.clone())
                .collect::<HashSet<_>>();
            (!unavailable.is_empty()).then_some((project.id.clone(), unavailable))
        })
        .collect()
}

fn disk_fingerprint_to_host(fingerprint: &DiskFingerprint) -> ProjectFileFingerprint {
    ProjectFileFingerprint {
        exists: fingerprint.exists,
        byte_len: fingerprint.byte_len,
        modified_nanos: fingerprint.modified.and_then(|modified| {
            modified
                .duration_since(UNIX_EPOCH)
                .ok()
                .map(|duration| duration.as_nanos())
        }),
        content_hash: fingerprint.content_hash,
        revision: ContentRevision {
            workspace_epoch: fingerprint.workspace_epoch,
            revision_number: fingerprint.revision_number,
            content_sha256: fingerprint.content_sha256,
        },
    }
}

fn draft_matches_document(draft: &DraftContentRevision, document: &ProjectEditorDocument) -> bool {
    let DraftBase::File {
        path,
        base_fingerprint,
    } = &draft.base
    else {
        return false;
    };
    HostPath::from_path(&document.model().document_id().canonical_path)
        .ok()
        .as_ref()
        == Some(path)
        && disk_fingerprint_to_host(document.model().disk_fingerprint()) == *base_fingerprint
}

fn missing_disk_fingerprint() -> DiskFingerprint {
    DiskFingerprint {
        exists: false,
        byte_len: 0,
        modified: None,
        content_hash: 0,
        workspace_epoch: 0,
        revision_number: 0,
        content_sha256: [0; 32],
    }
}

fn validate_relative_path(path: &PathBuf) -> Result<(), String> {
    if path.as_os_str().is_empty()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(format!(
            "remote workspace document path {} is not project-relative",
            path.display()
        ));
    }
    Ok(())
}

fn remote_host_epoch(runtime: &crate::host_runtime::DesktopHostRuntime) -> Option<u64> {
    match runtime.state() {
        HostConnectionState::Ready { host_epoch, .. } => Some(host_epoch),
        _ => None,
    }
}

fn is_workspace_control_conflict(error: &str) -> bool {
    let error = error.to_ascii_lowercase();
    error.contains("conflict")
        || error.contains("workspace control")
        || error.contains("permission denied")
}

fn is_remote_connection_error(error: &str) -> bool {
    let error = error.to_ascii_lowercase();
    error.contains("not connected")
        || error.contains("disconnected")
        || error.contains("connection")
        || error.contains("supervisor stopped")
}
