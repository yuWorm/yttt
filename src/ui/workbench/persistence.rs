use sha2::{Digest as _, Sha256};
use std::{
    collections::{HashMap, HashSet},
    fs,
    io::{self, Write as _},
    path::{Component, Path, PathBuf},
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

const RECOVERABLE_WORKSPACE_DRAFT_SCHEMA_VERSION: u16 = 1;

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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct PendingSettingsRecovery {
    candidate: serde_json::Value,
    confirmed: serde_json::Value,
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
    pub(super) startup_projects: Vec<PathBuf>,
    pub(super) available_terminal_sessions: HashSet<String>,
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
    recoverable_draft_path: Option<PathBuf>,
    pending_settings_recovery: Option<PendingSettingsRecovery>,
    recovered_settings_recovery: Option<PendingSettingsRecovery>,
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
            startup_projects: Vec::new(),
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
            recoverable_draft_path: None,
            pending_settings_recovery: None,
            recovered_settings_recovery: None,
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct RecoverableWorkspaceScope {
    profile_id: String,
    environment_id: String,
    workspace_id: WorkspaceId,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct RecoverableWorkspaceDraft {
    schema_version: u16,
    scope: RecoverableWorkspaceScope,
    host_epoch: Option<u64>,
    snapshot: serde_json::Value,
    drafts: Vec<RecoverableDraftContent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pending_settings: Option<PendingSettingsRecovery>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct RecoverableDraftContent {
    reference: DraftRef,
    content: Vec<u8>,
}

struct PreparedRemoteRestore {
    snapshot: RemoteWorkspaceSnapshot,
    server_snapshot: serde_json::Value,
    revision: u64,
    host_epoch: u64,
    services: HashMap<ProjectId, ProjectServices>,
    available_terminal_sessions: HashSet<String>,
    losses: Vec<RemoteResourceLoss>,
}

impl WorkbenchView {
    pub fn has_restorable_workspace(&self) -> bool {
        self.terminal
            .host_runtime
            .as_ref()
            .is_some_and(|runtime| runtime.pending_workspace_count() > 0)
    }

    pub(crate) fn restore_last_session(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.workspace.opened_projects().is_empty() || self.workspace_is_loading() {
            return;
        }
        let Some(runtime) = self.terminal.host_runtime.clone() else {
            return;
        };
        if runtime.pending_workspace_count() == 0 {
            return;
        }
        if self.workspace_persistence.commit_in_flight
            || self.workspace_persistence.control_request_in_flight
            || self.settings_save_pending()
            || self.has_failed_settings_save()
            || self.has_unpublished_workspace_snapshot(cx)
        {
            self.set_workspace_persistence_error(
                "Wait for pending changes to be saved before restoring another workspace".into(),
            );
            return;
        }
        // Claim before releasing the empty view, so it cannot claim itself.
        let view = match runtime.claim_workspace_view(true) {
            Ok(view) => view,
            Err(error) => {
                self.set_workspace_persistence_error(error);
                return;
            }
        };
        let workspace_id = view.id().clone();
        self.workspace_persistence = WorkspacePersistenceState {
            view: Some(view),
            mode: WorkspacePersistenceMode::AwaitingControl,
            ..WorkspacePersistenceState::default()
        };
        self.onboarding = None;
        self.discover_recoverable_workspace_draft(&runtime, &workspace_id);
        self.start_workspace_persistence_tick(window, cx);
        self.request_workspace_control_and_open(runtime, workspace_id, window, cx);
        cx.notify();
    }

    pub(super) fn workspace_is_loading(&self) -> bool {
        matches!(
            self.workspace_persistence.mode,
            WorkspacePersistenceMode::AwaitingControl
                | WorkspacePersistenceMode::Loading
                | WorkspacePersistenceMode::Restoring
        )
    }
    pub(crate) fn retain_pending_settings_recovery(
        &mut self,
        candidate: serde_json::Value,
        confirmed: serde_json::Value,
    ) {
        self.workspace_persistence.pending_settings_recovery = Some(PendingSettingsRecovery {
            candidate,
            confirmed,
        });
    }

    pub(crate) fn clear_pending_settings_recovery(
        &mut self,
        candidate: serde_json::Value,
        confirmed: serde_json::Value,
        cx: &mut Context<Self>,
    ) {
        let recovery = PendingSettingsRecovery {
            candidate,
            confirmed,
        };
        if self
            .workspace_persistence
            .pending_settings_recovery
            .as_ref()
            == Some(&recovery)
        {
            self.workspace_persistence.pending_settings_recovery = None;
        }
        if self
            .workspace_persistence
            .recovered_settings_recovery
            .as_ref()
            == Some(&recovery)
        {
            self.workspace_persistence.recovered_settings_recovery = None;
        }
        let Some(path) = self.workspace_persistence.recoverable_draft_path.clone() else {
            return;
        };
        let task = cx.background_spawn(async move {
            let mut draft = load_recoverable_workspace_draft(&path)?;
            if draft.pending_settings.as_ref() == Some(&recovery) {
                draft.pending_settings = None;
                save_recoverable_workspace_draft(&path, &draft)?;
            }
            Ok::<_, String>(())
        });
        cx.spawn(async move |this, cx| {
            if let Err(error) = task.await {
                let _ = this.update(cx, |root, cx| {
                    root.set_workspace_persistence_error(format!(
                        "Device-private settings recovery could not be cleared: {error}"
                    ));
                    cx.notify();
                });
            }
        })
        .detach();
    }

    pub(crate) fn take_recoverable_settings_recovery(
        &mut self,
    ) -> Option<(serde_json::Value, serde_json::Value)> {
        self.workspace_persistence
            .recovered_settings_recovery
            .take()
            .map(|recovery| (recovery.candidate, recovery.confirmed))
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
        } else if self.workspace_persistence.recoverable_draft_path.is_some() {
            self.ui_text
                .get(UiTextKey::RemoteDeviceDraftRecoveryAvailable)
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
        if self.workspace_persistence.recoverable_draft_path.is_some() {
            banner = banner.child(
                yttt_button(
                    "recover-device-drafts",
                    self.ui_text.get(UiTextKey::RemoteRecoverDeviceDrafts),
                    YtttButtonVariant::Secondary,
                    appearance.ui,
                    appearance.style,
                    cx,
                )
                .on_click(cx.listener(|root, _, window, cx| {
                    root.restore_recoverable_workspace_drafts(window, cx)
                })),
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
        self.discover_recoverable_workspace_draft(&runtime, &workspace_id);

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
            self.workspace_persistence.pending_commit = None;
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
            if self.workspace_persistence.mode == WorkspacePersistenceMode::Active
                && self.has_unpublished_workspace_snapshot(cx)
            {
                self.workspace_persistence.mode = WorkspacePersistenceMode::ControlLost;
                self.preserve_unpublished_workspace_drafts(
                    &runtime,
                    &workspace_id,
                    "The Host disconnected before local edits were published",
                    cx,
                );
            } else if self.workspace_persistence.mode != WorkspacePersistenceMode::ControlLost {
                self.workspace_persistence.mode = WorkspacePersistenceMode::AwaitingControl;
            }
            self.workspace_persistence.pending_commit = None;
            return;
        };
        if !runtime.is_controller() {
            let preserves_local_drafts = self.workspace_persistence.mode
                == WorkspacePersistenceMode::Active
                || (self.workspace_persistence.mode == WorkspacePersistenceMode::Observer
                    && self.workspace_persistence.recoverable_draft_path.is_some());
            if preserves_local_drafts {
                let unpublished = self.has_unpublished_workspace_snapshot(cx);
                self.workspace_persistence.pending_commit = None;
                if unpublished {
                    self.workspace_persistence.mode = WorkspacePersistenceMode::ControlLost;
                    self.preserve_unpublished_workspace_drafts(
                        &runtime,
                        &workspace_id,
                        if self.workspace_persistence.recoverable_draft_path.is_some() {
                            "Recovered Device-private drafts remain local while this Client observes"
                        } else {
                            "Host control was forced to another Client"
                        },
                        cx,
                    );
                } else if self.workspace_persistence.mode == WorkspacePersistenceMode::Active {
                    self.workspace_persistence.mode = WorkspacePersistenceMode::Observer;
                }
            }
            if matches!(
                self.workspace_persistence.mode,
                WorkspacePersistenceMode::Observer | WorkspacePersistenceMode::AwaitingControl
            ) {
                self.request_workspace_control_and_open(runtime, workspace_id, window, cx);
            }
            return;
        }
        if self.workspace_persistence.mode == WorkspacePersistenceMode::ControlLost {
            if let Some(transfer) = runtime.preparing_transfer()
                && let Some(view) = &self.workspace_persistence.view
            {
                view.published(
                    Some(transfer),
                    None,
                    self.workspace_persistence
                        .last_error
                        .clone()
                        .or_else(|| Some("workspace publication lost control".to_string())),
                );
            }
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
            self.workspace_persistence.pending_commit = None;
            if self.has_unpublished_workspace_snapshot(cx) {
                self.workspace_persistence.mode = WorkspacePersistenceMode::ControlLost;
                self.preserve_unpublished_workspace_drafts(
                    &runtime,
                    &workspace_id,
                    "Remote Host epoch changed before local edits were published",
                    cx,
                );
                return;
            }
            self.workspace_persistence.mode = WorkspacePersistenceMode::AwaitingControl;
            self.workspace_persistence.host_epoch = None;
        }
        if self.workspace_persistence.mode == WorkspacePersistenceMode::AwaitingControl {
            self.request_workspace_control_and_open(runtime, workspace_id, window, cx);
            return;
        }
        if self.workspace_persistence.mode == WorkspacePersistenceMode::Active {
            if self.shared_mutation_allowed() {
                let mut paths = std::mem::take(&mut self.workspace_persistence.startup_projects);
                if paths.is_empty() && self.workspace.opened_projects().is_empty() {
                    paths.extend(self.workspace_persistence.initial_project.take());
                }
                for path in paths {
                    if let Err(error) = self.open_project_path(path) {
                        self.set_workspace_persistence_error(format!(
                            "Could not open the requested initial project: {error}"
                        ));
                    }
                }
            }
            self.commit_remote_workspace_if_changed(runtime, workspace_id, window, cx);
            let publication = self.build_remote_workspace_snapshot(cx);
            let error = publication
                .as_ref()
                .err()
                .cloned()
                .or_else(|| self.workspace_persistence.last_error.clone())
                .or_else(|| self.settings.settings_save_error.clone());
            let clean = !self.settings_save_pending()
                && self
                    .workspace_persistence
                    .pending_settings_recovery
                    .is_none()
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
            && self.has_unpublished_workspace_snapshot(cx);
        self.workspace_persistence.control_request_in_flight = true;
        // Polling an observer snapshot must not replace an already usable view with a loader.
        if known_revision.is_none() {
            self.workspace_persistence.mode = WorkspacePersistenceMode::Loading;
        }
        let request_workspace_id = workspace_id.clone();
        let local_import = !runtime.is_remote();
        let runtime_for_open = runtime.clone();
        let open_task = cx.background_spawn(async move {
            acquire_control_and_open(&runtime_for_open, request_workspace_id.clone()).and_then(
                |(server_snapshot, revision)| {
                    if known_revision == Some(revision)
                        && known_epoch == remote_host_epoch(&runtime_for_open)
                    {
                        Ok(None)
                    } else {
                        prepare_remote_restore(
                            &runtime_for_open,
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
                    Ok(None)
                        if !mutation_belongs_to_current_host(
                            known_epoch,
                            root.terminal
                                .host_runtime
                                .as_deref()
                                .and_then(remote_host_epoch),
                        ) =>
                    {
                        root.workspace_persistence.mode = WorkspacePersistenceMode::AwaitingControl;
                        root.workspace_persistence.pending_commit = None;
                    }
                    Ok(None) => {
                        root.workspace_persistence.mode = root.restored_workspace_mode();
                    }
                    Ok(Some(prepared))
                        if !mutation_belongs_to_current_host(
                            Some(prepared.host_epoch),
                            root.terminal
                                .host_runtime
                                .as_deref()
                                .and_then(remote_host_epoch),
                        ) =>
                    {
                        root.workspace_persistence.mode = WorkspacePersistenceMode::AwaitingControl;
                        root.workspace_persistence.pending_commit = None;
                    }
                    Ok(Some(prepared))
                        if preserve_local && known_epoch != Some(prepared.host_epoch) =>
                    {
                        root.workspace_persistence.mode = WorkspacePersistenceMode::ControlLost;
                        root.workspace_persistence.pending_commit = None;
                        root.preserve_unpublished_workspace_drafts(
                            &runtime,
                            &workspace_id,
                            "Remote Host epoch changed while this Client was disconnected",
                            cx,
                        );
                    }
                    Ok(Some(prepared))
                        if local_import
                            && prepared.revision == 0
                            && root.workspace_persistence.revision.is_none() =>
                    {
                        root.workspace_persistence.revision = Some(0);
                        root.workspace_persistence.host_epoch = Some(prepared.host_epoch);
                        root.workspace_persistence.mode = root.restored_workspace_mode();
                        root.clear_workspace_persistence_error();
                    }
                    Ok(Some(prepared))
                        if root.workspace_persistence.revision.is_none() || !preserve_local =>
                    {
                        root.install_remote_workspace_restore(prepared, window, cx);
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
                        root.preserve_unpublished_workspace_drafts(
                            &runtime,
                            &workspace_id,
                            &format!(
                                "Remote workspace changed from revision {} to {} while this Client was disconnected",
                                root.workspace_persistence.revision.unwrap_or_default(),
                                prepared.revision
                            ),
                            cx,
                        );
                    }
                    Err(error) if is_workspace_control_conflict(&error) => {
                        root.workspace_persistence.mode = WorkspacePersistenceMode::ControlLost;
                        if preserve_local {
                            root.preserve_unpublished_workspace_drafts(
                                &runtime,
                                &workspace_id,
                                &format!("Remote Host rejected workspace publication: {error}"),
                                cx,
                            );
                        } else {
                            root.set_workspace_persistence_error(format!(
                                "Remote workspace control is unavailable: {error}"
                            ));
                        }
                    }
                    Err(error) => {
                        root.workspace_persistence.mode = WorkspacePersistenceMode::AwaitingControl;
                        root.workspace_persistence.pending_commit = None;
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
            .restore_snapshot(prepared.snapshot.editor.clone(), &terminal_ids)
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
        if document.draft.is_none()
            && crate::ui::editor::preview::is_image_path(&document.relative_path)
            && !crate::ui::editor::preview::is_svg_path(&document.relative_path)
        {
            self.open_file_preview(document.project_id, document.relative_path, window, cx);
            self.finish_remote_document_restore(cx);
            return;
        }
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
        let project_path = self
            .workspace
            .project(&document.project_id)
            .and_then(|project| project.location.local_path())
            .cloned();
        let config_paths = self.config_paths.clone();
        let relative_path = request.relative_path.clone();
        let load_task = cx.background_spawn(async move {
            super::project_files::load_project_document_with_overrides(
                &services,
                &config_paths,
                project_path.as_deref(),
                &relative_path,
            )
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = load_task.await;
            let _ = this.update_in(cx, |root, window, cx| {
                match result {
                    Ok((loaded, overrides)) => {
                        root.apply_project_file_open_success(
                            &request, loaded, overrides, window, cx,
                        );
                        root.apply_remote_document_state(
                            &request.document_id,
                            &document,
                            window,
                            cx,
                        );
                    }
                    Err(error) => {
                        let error = match error {
                            super::project_files::ProjectDocumentLoadError::File(error) => {
                                error.to_string()
                            }
                            super::project_files::ProjectDocumentLoadError::ProjectSettings(
                                error,
                            ) => error.to_string(),
                        };
                        root.apply_project_file_open_error(&request, error.clone());
                        if document.draft.is_some() {
                            root.restore_missing_remote_draft(&document, window, cx);
                        } else {
                            root.open_unavailable_file(&request, error, window, cx);
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
        if let Err(error) = self
            .project
            .project_editor_runtime
            .workspace_mut()
            .restore_snapshot(editor_snapshot, &terminal_ids)
        {
            self.workspace_persistence.mode = WorkspacePersistenceMode::ControlLost;
            self.set_workspace_persistence_error(format!(
                "Remote editor layout could not be finalized: {error}"
            ));

            return;
        }
        self.workspace_persistence.mode = self.restored_workspace_mode();
        cx.notify();
    }
    fn restore_recoverable_workspace_drafts(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(path) = self.workspace_persistence.recoverable_draft_path.clone() else {
            return;
        };
        let Some(runtime) = self.terminal.host_runtime.clone() else {
            self.set_workspace_persistence_error(
                "Cannot recover Device-private drafts while the Host is unavailable".to_string(),
            );
            return;
        };
        let Some(workspace_id) = self
            .workspace_persistence
            .view
            .as_ref()
            .map(|view| view.id().clone())
        else {
            self.set_workspace_persistence_error(
                "Cannot recover Device-private drafts because the workspace identity is unavailable"
                    .to_string(),
            );
            return;
        };
        let expected_scope = match recoverable_workspace_scope(&runtime, &workspace_id) {
            Ok(scope) => scope,
            Err(error) => {
                self.set_workspace_persistence_error(format!(
                    "Cannot recover Device-private drafts: {error}"
                ));
                return;
            }
        };
        let draft = match load_recoverable_workspace_draft(&path) {
            Ok(draft) if draft.scope == expected_scope => draft,
            Ok(_) => {
                self.set_workspace_persistence_error(
                    "Device-private draft recovery belongs to a different Host workspace"
                        .to_string(),
                );
                return;
            }
            Err(error) => {
                self.set_workspace_persistence_error(format!(
                    "Cannot recover Device-private drafts: {error}"
                ));
                return;
            }
        };
        let settings_recovery = draft.pending_settings.clone();
        let snapshot = match RemoteWorkspaceSnapshot::from_value(draft.snapshot) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                self.set_workspace_persistence_error(format!(
                    "Cannot recover Device-private drafts: {error}"
                ));
                return;
            }
        };
        let mut contents = HashMap::new();
        for body in draft.drafts {
            let Ok(content) = String::from_utf8(body.content) else {
                self.set_workspace_persistence_error(
                    "Cannot recover Device-private drafts with invalid text".to_string(),
                );
                return;
            };
            contents.insert(
                body.reference.document_id.clone(),
                (body.reference, content),
            );
        }
        let mut recovered = 0;
        let mut unavailable_projects: usize = 0;
        for mut document in snapshot.documents {
            let Some(reference) = document.draft_ref.as_ref() else {
                continue;
            };
            let Some((body_reference, content)) = contents.remove(&reference.document_id) else {
                self.set_workspace_persistence_error(
                    "Cannot recover Device-private drafts because a document body is missing"
                        .to_string(),
                );
                return;
            };
            if body_reference != *reference {
                self.set_workspace_persistence_error(
                    "Cannot recover Device-private drafts because a document body does not match its manifest"
                        .to_string(),
                );
                return;
            }
            if self.workspace.project(&document.project_id).is_none() {
                unavailable_projects = unavailable_projects.saturating_add(1);
                continue;
            }
            document.draft = Some(DraftContentRevision {
                revision: reference.revision,
                base: reference.base.clone(),
                content,
            });
            self.workspace_persistence.pending_document_restores = self
                .workspace_persistence
                .pending_document_restores
                .saturating_add(1);
            self.spawn_remote_document_restore(document, window, cx);
            recovered += 1;
        }
        if recovered == 0 {
            self.set_workspace_persistence_error(
                "No Device-private drafts match projects currently open from this Host workspace"
                    .to_string(),
            );
        } else {
            self.set_workspace_persistence_error(format!(
                "Recovering {recovered} Device-private draft{} without replacing the Host workspace{}",
                if recovered == 1 { "" } else { "s" },
                if unavailable_projects == 0 {
                    String::new()
                } else {
                    format!("; {unavailable_projects} project{} remain recoverable", if unavailable_projects == 1 { "" } else { "s" })
                }
            ));
        }
        if settings_recovery.is_some() {
            self.workspace_persistence.recovered_settings_recovery = settings_recovery;
            self.restore_recovered_settings_draft();
        }
        cx.notify();
    }

    fn discover_recoverable_workspace_draft(
        &mut self,
        runtime: &crate::host_runtime::DesktopHostRuntime,
        workspace_id: &WorkspaceId,
    ) {
        let scope = match recoverable_workspace_scope(runtime, workspace_id) {
            Ok(scope) => scope,
            Err(error) => {
                self.set_workspace_persistence_error(format!(
                    "Local draft recovery is unavailable: {error}"
                ));
                return;
            }
        };
        let path = match recoverable_workspace_draft_path(&self.config_paths, runtime, workspace_id)
        {
            Ok(Some(path)) => path,
            Ok(None) => return,
            Err(error) => {
                self.set_workspace_persistence_error(format!(
                    "Local draft recovery is unavailable: {error}"
                ));
                return;
            }
        };
        if !path.exists() {
            return;
        }
        match load_recoverable_workspace_draft(&path) {
            Ok(draft) if draft.scope == scope => {
                self.workspace_persistence.recovered_settings_recovery =
                    draft.pending_settings.clone();
                self.workspace_persistence.recoverable_draft_path = Some(path);
            }
            Ok(_) => {
                self.set_workspace_persistence_error(
                    "Local draft recovery scope does not match this Host workspace".to_string(),
                );
            }
            Err(error) => {
                self.set_workspace_persistence_error(format!(
                    "Local draft recovery could not be read: {error}"
                ));
            }
        }
    }

    fn has_unpublished_workspace_snapshot(&self, cx: &Context<Self>) -> bool {
        self.workspace_persistence
            .pending_settings_recovery
            .is_some()
            || self
                .build_remote_workspace_snapshot(cx)
                .map_or(true, |snapshot| {
                    self.workspace_persistence.last_committed_snapshot.as_ref() != Some(&snapshot.0)
                })
    }

    fn preserve_unpublished_workspace_drafts(
        &mut self,
        runtime: &crate::host_runtime::DesktopHostRuntime,
        workspace_id: &WorkspaceId,
        reason: &str,
        cx: &Context<Self>,
    ) {
        let (snapshot, drafts) = match self.build_workspace_snapshot(cx, None) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                self.set_workspace_persistence_error(format!(
                    "{reason}; unpublished edits remain open, but their local recovery snapshot could not be prepared: {error}"
                ));
                return;
            }
        };
        let path = match recoverable_workspace_draft_path(&self.config_paths, runtime, workspace_id)
        {
            Ok(Some(path)) => path,
            Ok(None) => {
                self.set_workspace_persistence_error(format!(
                    "{reason}; unpublished edits remain open, but Device-private recovery storage is unavailable"
                ));
                return;
            }
            Err(error) => {
                self.set_workspace_persistence_error(format!(
                    "{reason}; unpublished edits remain open, but Device-private recovery storage could not be located: {error}"
                ));
                return;
            }
        };
        let scope = match recoverable_workspace_scope(runtime, workspace_id) {
            Ok(scope) => scope,
            Err(error) => {
                self.set_workspace_persistence_error(format!(
                    "{reason}; unpublished edits remain open, but Device-private recovery storage could not be scoped: {error}"
                ));
                return;
            }
        };
        let draft = RecoverableWorkspaceDraft {
            schema_version: RECOVERABLE_WORKSPACE_DRAFT_SCHEMA_VERSION,
            scope,
            host_epoch: remote_host_epoch(runtime).or(self.workspace_persistence.host_epoch),
            snapshot,
            drafts: drafts
                .into_iter()
                .map(|(reference, content)| RecoverableDraftContent {
                    reference,
                    content: content.as_ref().clone(),
                })
                .collect(),
            pending_settings: self.workspace_persistence.pending_settings_recovery.clone(),
        };
        match save_recoverable_workspace_draft(&path, &draft) {
            Ok(()) => {
                self.workspace_persistence.recoverable_draft_path = Some(path);
                self.set_workspace_persistence_error(format!(
                    "{reason}; unpublished edits were saved to Device-private recovery storage and will not replace the Host workspace automatically"
                ));
            }
            Err(error) => {
                self.set_workspace_persistence_error(format!(
                    "{reason}; unpublished edits remain open, but Device-private recovery storage failed: {error}"
                ));
            }
        }
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
        self.report_lost_remote_resources(&losses);
    }

    fn report_lost_remote_resources(&mut self, losses: &[RemoteResourceLoss]) {
        if losses.is_empty() {
            return;
        }
        self.set_workspace_resource_loss(format!(
            "Host lost {} terminal process{}; restoring shells and Agent sessions without replaying other commands",
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
        if self
            .workspace_persistence
            .pending_settings_recovery
            .is_some()
            && let Some(runtime) = self.terminal.host_runtime.clone()
            && let Some(workspace_id) = self
                .workspace_persistence
                .view
                .as_ref()
                .map(|view| view.id().clone())
        {
            self.preserve_unpublished_workspace_drafts(
                &runtime,
                &workspace_id,
                "Settings have not been confirmed by the Host",
                cx,
            );
        }
        self.workspace_persistence.close_prompt_open = true;
        let answer = window.prompt(
            gpui::PromptLevel::Warning,
            "Changes have not been saved",
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
        if self.settings_save_pending() || self.has_failed_settings_save() {
            self.set_workspace_persistence_error(
                "Settings are still pending or failed to save; keep this window open to retry or copy the draft"
                    .to_string(),
            );
            self.confirm_close_without_remote_save(window, cx);
            return false;
        }
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
            || self.settings_save_pending()
            || self.has_failed_settings_save()
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
        let Some(request_host_epoch) = remote_host_epoch(&runtime) else {
            self.workspace_persistence.mode = WorkspacePersistenceMode::AwaitingControl;
            self.workspace_persistence.pending_commit = None;
            return;
        };
        if self.workspace_persistence.host_epoch != Some(request_host_epoch) {
            self.workspace_persistence.mode = WorkspacePersistenceMode::AwaitingControl;
            self.workspace_persistence.pending_commit = None;
            return;
        }
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
        let runtime_for_commit = runtime.clone();
        let commit_task = cx.background_spawn(async move {
            runtime_for_commit.workspace_request(WorkspaceRequest::Register {
                workspace_id: request_workspace_id.clone(),
                name: request_workspace_id.as_str().to_string(),
            })?;
            let mut references = Vec::with_capacity(drafts.len());
            for (reference, content) in drafts.iter() {
                if !confirmed_drafts.contains(&reference.content_sha256) {
                    runtime_for_commit.workspace_request(WorkspaceRequest::PutDraft {
                        workspace_id: request_workspace_id.clone(),
                        reference: reference.clone(),
                        content: content.as_ref().clone(),
                    })?;
                }
                references.push(reference.clone());
            }
            runtime_for_commit.workspace_request(WorkspaceRequest::Commit {
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
                let response_is_current = mutation_belongs_to_current_host(
                    Some(request_host_epoch),
                    root.terminal
                        .host_runtime
                        .as_deref()
                        .and_then(remote_host_epoch),
                ) && root.workspace_persistence.mode == WorkspacePersistenceMode::Active
                    && root
                        .terminal
                        .host_runtime
                        .as_ref()
                        .is_some_and(|runtime| runtime.is_controller());
                if !response_is_current {
                    root.workspace_persistence.pending_commit = None;
                    let forced_loss = root.workspace_persistence.mode
                        == WorkspacePersistenceMode::Active
                        && root
                            .terminal
                            .host_runtime
                            .as_ref()
                            .is_some_and(|runtime| !runtime.is_controller());
                    if forced_loss {
                        root.workspace_persistence.mode = WorkspacePersistenceMode::ControlLost;
                        root.preserve_unpublished_workspace_drafts(
                            &runtime,
                            &workspace_id,
                            "Host control was forced to another Client while publishing",
                            cx,
                        );
                    } else if root.workspace_persistence.mode != WorkspacePersistenceMode::ControlLost {
                        root.workspace_persistence.mode = WorkspacePersistenceMode::ControlLost;
                        root.preserve_unpublished_workspace_drafts(
                            &runtime,
                            &workspace_id,
                            "Ignored a stale workspace publication response; local edits were not replayed",
                            cx,
                        );
                    }
                } else {
                    match result {
                        Ok(WorkspaceResponse::Committed {
                            workspace_id: committed_workspace_id,
                            revision,
                            snapshot,
                        }) if committed_workspace_id == workspace_id => {
                            root.workspace_persistence.revision = Some(revision);
                            root.workspace_persistence.confirmed_drafts = pending
                                .drafts
                                .iter()
                                .map(|(reference, _)| reference.content_sha256)
                                .collect();
                            root.workspace_persistence.last_committed_snapshot =
                                Some(snapshot.into_value());
                            root.workspace_persistence.pending_commit = None;
                            root.clear_workspace_persistence_error();
                        }
                        Ok(response) => {
                            root.workspace_persistence.mode = WorkspacePersistenceMode::ControlLost;
                            root.preserve_unpublished_workspace_drafts(
                                &runtime,
                                &workspace_id,
                                &format!(
                                    "Remote Host returned an unexpected workspace commit response: {response:?}"
                                ),
                                cx,
                            );
                        }
                        Err(error) if is_workspace_control_conflict(&error) => {
                            root.workspace_persistence.mode = WorkspacePersistenceMode::ControlLost;
                            root.preserve_unpublished_workspace_drafts(
                                &runtime,
                                &workspace_id,
                                &format!(
                                    "Remote workspace commit was rejected; control was lost: {error}"
                                ),
                                cx,
                            );
                        }
                        Err(error) if is_remote_connection_error(&error) => {
                            root.workspace_persistence.mode = WorkspacePersistenceMode::ControlLost;
                            root.workspace_persistence.pending_commit = None;
                            root.preserve_unpublished_workspace_drafts(
                                &runtime,
                                &workspace_id,
                                &format!("The Host disconnected while publishing: {error}"),
                                cx,
                            );
                        }
                        Err(error) => {
                            root.set_workspace_persistence_error(format!(
                                "Remote workspace commit failed; prior snapshot remains intact: {error}"
                            ));
                        }
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
        self.build_workspace_snapshot(
            cx,
            Some((MAX_DRAFT_CONTENT_BYTES, MAX_WORKSPACE_DRAFT_BYTES)),
        )
    }

    fn build_workspace_snapshot(
        &self,
        cx: &Context<Self>,
        limits: Option<(usize, usize)>,
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
            let root = match &project.location {
                ProjectLocation::Local { path } => path.as_path(),
                ProjectLocation::Ssh { root, .. } => Path::new(root.as_str()),
            };
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
                                    path: HostPath::from_client_path(&document_id.canonical_path)
                                        .map_err(|error| error.to_string())?,
                                    base_fingerprint: disk_fingerprint_to_host(
                                        document.model().disk_fingerprint(),
                                    ),
                                },
                                content: document.current_text(cx),
                            };
                            if limits.is_some_and(|(document_limit, _)| {
                                draft.content.len() > document_limit
                            }) {
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
                        if limits.is_some_and(|(_, workspace_limit)| total_bytes > workspace_limit)
                        {
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

fn recoverable_workspace_scope(
    runtime: &crate::host_runtime::DesktopHostRuntime,
    workspace_id: &WorkspaceId,
) -> Result<RecoverableWorkspaceScope, String> {
    let device_paths = crate::config::scope::device_preferences_config_paths()
        .ok_or_else(|| "the Device profile is not bound".to_string())?;
    let profile = device_paths
        .profile()
        .ok_or_else(|| "the Device profile is not bound".to_string())?;
    let storage = runtime.environment_storage();
    let environment = storage.environment();
    if environment.environment_id.is_empty() {
        return Err("the Host returned an empty environment identity".to_string());
    }
    Ok(RecoverableWorkspaceScope {
        profile_id: profile.id().as_str().to_string(),
        environment_id: environment.environment_id.clone(),
        workspace_id: workspace_id.clone(),
    })
}

fn recoverable_workspace_draft_path(
    paths: &crate::config::paths::AppConfigPaths,
    runtime: &crate::host_runtime::DesktopHostRuntime,
    workspace_id: &WorkspaceId,
) -> Result<Option<PathBuf>, String> {
    let Some(root) = paths.device_state_dir() else {
        return Ok(None);
    };
    let scope = recoverable_workspace_scope(runtime, workspace_id)?;
    Ok(Some(recoverable_workspace_draft_file(&root, &scope)))
}

fn recoverable_workspace_draft_file(root: &Path, scope: &RecoverableWorkspaceScope) -> PathBuf {
    let key = serde_json::to_vec(scope).expect("recovery scope is serializable");
    let digest = format!("{:x}", Sha256::digest(key));
    root.join("draft-recovery").join(format!("{digest}.json"))
}

fn save_recoverable_workspace_draft(
    path: &Path,
    draft: &RecoverableWorkspaceDraft,
) -> Result<(), String> {
    let source = serde_json::to_vec(draft)
        .map_err(|error| format!("could not encode local recovery drafts: {error}"))?;
    write_device_private_file(path, &source)
        .map_err(|error| format!("could not write {}: {error}", path.display()))
}

fn load_recoverable_workspace_draft(path: &Path) -> Result<RecoverableWorkspaceDraft, String> {
    let source =
        fs::read(path).map_err(|error| format!("could not read {}: {error}", path.display()))?;
    let draft: RecoverableWorkspaceDraft = serde_json::from_slice(&source)
        .map_err(|error| format!("could not decode {}: {error}", path.display()))?;
    if draft.schema_version != RECOVERABLE_WORKSPACE_DRAFT_SCHEMA_VERSION {
        return Err(format!(
            "{} uses unsupported schema {}",
            path.display(),
            draft.schema_version
        ));
    }
    let snapshot = RemoteWorkspaceSnapshot::from_value(draft.snapshot.clone())?;
    let mut expected = HashMap::new();
    for document in &snapshot.documents {
        if let Some(reference) = &document.draft_ref
            && expected
                .insert(reference.document_id.clone(), reference)
                .is_some()
        {
            return Err(format!(
                "{} contains duplicate recovered draft identities",
                path.display()
            ));
        }
    }
    for item in &draft.drafts {
        let actual: [u8; 32] = Sha256::digest(&item.content).into();
        let matches_manifest = expected
            .remove(&item.reference.document_id)
            .is_some_and(|reference| reference == &item.reference);
        if item.reference.bytes != item.content.len() as u64
            || item.reference.content_sha256 != actual
            || std::str::from_utf8(&item.content).is_err()
            || !matches_manifest
        {
            return Err(format!(
                "{} contains an invalid recovered draft body",
                path.display()
            ));
        }
    }
    if !expected.is_empty() {
        return Err(format!(
            "{} is missing recovered draft bodies",
            path.display()
        ));
    }
    Ok(draft)
}

fn write_device_private_file(path: &Path, source: &[u8]) -> io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "Device-private recovery path has no parent directory",
        )
    })?;
    fs::create_dir_all(parent)?;
    let mut temporary = tempfile::Builder::new()
        .prefix(".yttt-draft-recovery-")
        .suffix(".tmp")
        .tempfile_in(parent)?;
    temporary.write_all(source)?;
    temporary.as_file_mut().sync_all()?;
    temporary.persist(path).map_err(|error| error.error)?;

    #[cfg(unix)]
    fs::File::open(parent)?.sync_all()?;

    Ok(())
}

fn mutation_belongs_to_current_host(
    expected_epoch: Option<u64>,
    current_epoch: Option<u64>,
) -> bool {
    expected_epoch.is_some() && expected_epoch == current_epoch
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
    HostPath::from_client_path(&document.model().document_id().canonical_path)
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

fn validate_relative_path(path: &Path) -> Result<(), String> {
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

#[cfg(test)]
mod tests {
    use super::*;
    #[gpui::test]
    fn cold_workspace_restore_preserves_mixed_tabs_and_file_contents(
        cx: &mut gpui::TestAppContext,
    ) {
        use crate::ui::editor::{WorkAreaDropEdge, WorkAreaDropPlacement, WorkItemId};

        cx.update(gpui_component::init);
        let temp = tempfile::tempdir().unwrap();
        let project_path = temp.path().join("project");
        fs::create_dir(&project_path).unwrap();
        let project_path = project_path.canonicalize().unwrap();
        fs::write(project_path.join("note.txt"), "saved file contents\n").unwrap();
        let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
        let mut workspace = Workspace::new();
        let project_id = workspace
            .open_project(
                ProjectDescriptor::new(
                    ProjectId::new("restore-project"),
                    ProjectLocation::local(project_path.clone()),
                ),
                dev_fixture_layout(),
            )
            .unwrap();
        workspace
            .mark_pane_running(&project_id, "dev", "shell")
            .unwrap();
        workspace
            .mark_pane_running(&project_id, "agent", "codex")
            .unwrap();
        let source = cx.new(|_| {
            WorkbenchView::with_workspace_for_test_and_config_paths(workspace, paths.clone())
        });
        let (server_snapshot, expected_area, document_id) = source.update(cx, |root, cx| {
            let session = root
                .project
                .project_editor_runtime
                .workspace_mut()
                .session_mut(&project_id)
                .unwrap();
            let terminals = vec!["dev".to_string(), "agent".to_string()];
            session.reconcile_work_area(&terminals);
            let document = session.open_file(project_path.join("note.txt"));
            let group = session.active_group_id();
            assert!(session.drop_work_item(
                &WorkItemId::File(document.clone()),
                group,
                group,
                WorkAreaDropPlacement::Edge(WorkAreaDropEdge::Right),
                &terminals,
            ));
            let area = session.work_area().clone();
            (
                root.build_remote_workspace_snapshot(cx).unwrap().0,
                area,
                document,
            )
        });
        let mut snapshot = RemoteWorkspaceSnapshot::from_value(server_snapshot.clone()).unwrap();
        let mut restored = Workspace::restore_persisted_state(snapshot.workspace).unwrap();
        let losses = restored.reconcile_host_resources(&HashSet::new());
        snapshot.workspace = restored.persisted_state();
        let services = HashMap::from([(
            project_id.clone(),
            ProjectServices::local_for_test(project_path),
        )]);
        let target_slot = std::rc::Rc::new(std::cell::RefCell::new(None));
        let window_slot = target_slot.clone();
        let (_component, cx) = cx.add_window_view(|window, cx| {
            let target = cx.new(|_| WorkbenchView::with_config_paths_for_test(paths));
            *window_slot.borrow_mut() = Some(target.clone());
            gpui_component::Root::new(target, window, cx)
        });
        let target = target_slot.borrow_mut().take().unwrap();
        target.update_in(cx, |root, window, cx| {
            root.install_remote_workspace_restore(
                PreparedRemoteRestore {
                    snapshot,
                    server_snapshot,
                    revision: 1,
                    host_epoch: 2,
                    services,
                    available_terminal_sessions: HashSet::new(),
                    losses,
                },
                window,
                cx,
            );
        });
        cx.run_until_parked();
        target.read_with(cx, |root, cx| {
            let session = root
                .project
                .project_editor_runtime
                .workspace()
                .session(&project_id)
                .unwrap();
            assert_eq!(session.work_area(), &expected_area);
            assert_eq!(
                session.active_work_item(),
                Some(&WorkItemId::File(document_id.clone()))
            );
            let document = root
                .project
                .project_editor_runtime
                .document(&document_id)
                .unwrap_or_else(|| {
                    panic!(
                        "document restore failed: {:?}, pending: {}",
                        root.load_error, root.workspace_persistence.pending_document_restores
                    )
                });
            assert_eq!(document.read(cx).current_text(cx), "saved file contents\n");
            assert_eq!(
                root.workspace
                    .project(&project_id)
                    .unwrap()
                    .layout
                    .tabs
                    .len(),
                2
            );
        });
    }

    #[gpui::test]
    fn settings_only_draft_prevents_a_clean_workspace_handoff(cx: &mut gpui::TestAppContext) {
        let temp = tempfile::tempdir().unwrap();
        let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
        let root = cx.new(|_| WorkbenchView::with_config_paths(paths));
        root.update(cx, |root, cx| {
            root.workspace_persistence.last_committed_snapshot =
                Some(root.build_remote_workspace_snapshot(cx).unwrap().0);
            assert!(!root.has_unpublished_workspace_snapshot(cx));
            let candidate = serde_json::json!({"editor": {"tab_size": 8}});
            let confirmed = serde_json::json!({"editor": {"tab_size": 4}});
            root.retain_pending_settings_recovery(candidate.clone(), confirmed.clone());
            assert!(root.has_unpublished_workspace_snapshot(cx));
            root.clear_pending_settings_recovery(candidate, confirmed, cx);
            assert!(!root.has_unpublished_workspace_snapshot(cx));
        });
    }

    fn scope(
        profile_id: &str,
        environment_id: &str,
        workspace_id: &str,
    ) -> RecoverableWorkspaceScope {
        RecoverableWorkspaceScope {
            profile_id: profile_id.to_string(),
            environment_id: environment_id.to_string(),
            workspace_id: WorkspaceId::new(workspace_id).unwrap(),
        }
    }

    #[test]
    fn forced_loss_drafts_are_private_to_host_profile_and_workspace() {
        let root = tempfile::tempdir().unwrap();
        let recovery_scope = scope("desktop", "host-a", "workspace-a");
        let path = recoverable_workspace_draft_file(root.path(), &recovery_scope);

        assert_ne!(
            path,
            recoverable_workspace_draft_file(
                root.path(),
                &scope("desktop", "host-b", "workspace-a")
            )
        );
        assert_ne!(
            path,
            recoverable_workspace_draft_file(
                root.path(),
                &scope("other-device", "host-a", "workspace-a")
            )
        );
        assert_ne!(
            path,
            recoverable_workspace_draft_file(
                root.path(),
                &scope("desktop", "host-a", "workspace-b")
            )
        );

        let draft = RecoverableWorkspaceDraft {
            schema_version: RECOVERABLE_WORKSPACE_DRAFT_SCHEMA_VERSION,
            scope: recovery_scope.clone(),
            host_epoch: Some(9),
            snapshot: serde_json::to_value(RemoteWorkspaceSnapshot::empty()).unwrap(),
            drafts: Vec::new(),
            pending_settings: Some(PendingSettingsRecovery {
                candidate: serde_json::json!({"terminal": {"scrollback": 20_000}}),
                confirmed: serde_json::json!({"terminal": {"scrollback": 1_000}}),
            }),
        };
        save_recoverable_workspace_draft(&path, &draft).unwrap();

        let restored = load_recoverable_workspace_draft(&path).unwrap();
        assert_eq!(restored.scope, recovery_scope);
        assert_eq!(restored.host_epoch, Some(9));
        assert_eq!(
            restored
                .pending_settings
                .as_ref()
                .map(|settings| &settings.candidate),
            Some(&serde_json::json!({"terminal": {"scrollback": 20_000}}))
        );
    }

    #[test]
    fn reconnect_rejects_stale_epoch_publication_responses() {
        assert!(mutation_belongs_to_current_host(Some(7), Some(7)));
        assert!(!mutation_belongs_to_current_host(Some(7), Some(8)));
        assert!(!mutation_belongs_to_current_host(Some(7), None));
        assert!(!mutation_belongs_to_current_host(None, Some(7)));
    }

    #[test]
    fn host_rejection_is_classified_as_a_control_conflict() {
        assert!(is_workspace_control_conflict("workspace control conflict"));
        assert!(is_workspace_control_conflict("permission denied"));
        assert!(!is_workspace_control_conflict(
            "temporary transport timeout"
        ));
    }
}
