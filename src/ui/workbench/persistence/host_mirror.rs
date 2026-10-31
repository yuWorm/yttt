//! Mirrors the Host layout of the open workspace: every accepted intent, from
//! this Client or another, replaces this Client's workspace and editor work
//! areas.

use super::*;
use crate::model::host_workspace::{HostWorkspace, pane_changes};
use crate::ui::editor::ProjectFileLoadRequest;

/// Whether the layout being adopted comes with the workspace's Agent states.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum HostAgentStates {
    /// Read when the workspace opened; they replace what this Client held.
    Included,
    /// The Host publishes layouts without them.
    Unpublished,
}

/// Wakes the workbench when the Host layout of `workspace_id` changes.
pub(super) struct HostLayoutListener {
    workspace_id: WorkspaceId,
    changes: flume::Receiver<()>,
    _task: Task<()>,
}

impl WorkbenchView {
    /// Listens for Host layout changes of `workspace_id`, replacing a listener
    /// of another workspace or of a Host connection that ended.
    pub(super) fn ensure_host_layout_listener(
        &mut self,
        runtime: &crate::host_runtime::DesktopHostRuntime,
        workspace_id: &WorkspaceId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .workspace_persistence
            .host_layout_listener
            .as_ref()
            .is_some_and(|listener| {
                &listener.workspace_id == workspace_id && !listener.changes.is_disconnected()
            })
        {
            return;
        }
        let changes = runtime.workspace_layout_changes(workspace_id.clone());
        let receiver = changes.clone();
        let task = cx.spawn_in(window, async move |this, cx| {
            while receiver.recv_async().await.is_ok() {
                if this
                    .update_in(cx, |root, window, cx| {
                        root.adopt_host_workspace_layout(window, cx)
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        self.workspace_persistence.host_layout_listener = Some(HostLayoutListener {
            workspace_id: workspace_id.clone(),
            changes,
            _task: task,
        });
    }

    /// Replaces the local layout with a newer Host layout of the same Host.
    /// A Host that restarted is opened again instead, which also reads the
    /// Agent states and documents.
    pub(super) fn adopt_host_workspace_layout(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let persistence = &self.workspace_persistence;
        if persistence.closing || persistence.mode != WorkspacePersistenceMode::Active {
            return;
        }
        let Some(held_revision) = persistence.revision else {
            return;
        };
        let Some(runtime) = self.terminal.host_runtime.clone() else {
            return;
        };
        let Some(workspace_id) = persistence.view.as_ref().map(|view| view.id().clone()) else {
            return;
        };
        if persistence.host_epoch.is_none() || persistence.host_epoch != remote_host_epoch(&runtime)
        {
            return;
        }
        let Some(layout) = runtime
            .workspace_layout(&workspace_id)
            .filter(|layout| layout.revision > held_revision)
        else {
            return;
        };
        match HostWorkspace::restore(layout) {
            Ok(host) => self.replace_with_host_layout(
                host,
                HostAgentStates::Unpublished,
                &runtime,
                window,
                cx,
            ),
            Err(error) => self.set_workspace_persistence_error(
                self.ui_text
                    .get(UiTextKey::PersistenceStateRestoreFailed)
                    .replace("{error}", &error.to_string()),
            ),
        }
    }

    /// Makes `host` the local workspace and editor work areas: drops the
    /// terminal views of removed panes and the closed projects, registers
    /// the projects another Client opened, and loads the files it opened
    /// without changing the active item. Unsaved edits of files the layout
    /// closed are kept as Device copies.
    pub(super) fn replace_with_host_layout(
        &mut self,
        host: HostWorkspace,
        agent_states: HostAgentStates,
        runtime: &Arc<crate::host_runtime::DesktopHostRuntime>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let open_files = host
            .editor()
            .sessions()
            .flat_map(|(_, session)| session.file_ids())
            .cloned()
            .collect::<HashSet<_>>();
        let closed_edits = self
            .workspace
            .opened_projects()
            .iter()
            .flat_map(|project| {
                self.project
                    .project_editor_runtime
                    .documents_for_project(&project.id)
            })
            .filter(|(id, document)| {
                !open_files.contains(*id) && document.read(cx).model().is_dirty()
            })
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        self.preserve_document_copies(&closed_edits, window, cx);

        let state = host.state();
        let mut workspace = host.workspace().clone();
        if agent_states == HostAgentStates::Unpublished {
            workspace.keep_agent_snapshots_from(&self.workspace);
        }
        let panes = pane_changes(&self.workspace, &workspace);
        let previous_projects = self
            .workspace
            .opened_projects()
            .iter()
            .map(|project| project.id.clone())
            .collect::<HashSet<_>>();
        let previous_item = self.active_work_item();

        self.workspace = workspace;
        *self.project.project_editor_runtime.workspace_mut() = host.editor().clone();
        self.project.project_editor_runtime.sync_file_trees();
        self.workspace_persistence.revision = Some(state.revision);

        self.forget_removed_panes(&panes.removed);
        for project_id in &previous_projects {
            if self.workspace.project(project_id).is_none() {
                self.cleanup_closed_project(project_id);
            }
        }
        self.register_mirrored_projects(runtime, &previous_projects);
        self.load_mirrored_files(window, cx);

        let next_item = self.selected_active_work_item();
        if next_item != previous_item {
            match next_item {
                Some(item) => {
                    let _ = self.apply_active_work_item(&item);
                }
                None => self.sync_input_owner_state(),
            }
        }
        cx.notify();
    }

    /// Registers the projects another Client opened with the Host.
    fn register_mirrored_projects(
        &mut self,
        runtime: &Arc<crate::host_runtime::DesktopHostRuntime>,
        previous_projects: &HashSet<ProjectId>,
    ) {
        let opened = self
            .workspace
            .opened_projects()
            .iter()
            .filter(|project| !previous_projects.contains(&project.id))
            .filter_map(|project| {
                project
                    .location
                    .local_path()
                    .map(|path| (project.id.clone(), path.clone()))
            })
            .collect::<Vec<_>>();
        for (project_id, path) in opened {
            match ProjectServices::host(runtime.clone(), project_id.clone(), path) {
                Ok(services) => {
                    self.project.services.insert(project_id, services);
                }
                Err(error) => {
                    self.load_error = combine_load_messages(
                        self.load_error.take(),
                        Some(
                            self.ui_text
                                .get(UiTextKey::HostProjectRegistrationFailed)
                                .replace("{error}", &error.to_string()),
                        ),
                    );
                }
            }
        }
    }

    /// Drops the files the layout closed and loads the ones another Client
    /// opened, without changing the active item.
    fn load_mirrored_files(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let closed = self.project.project_editor_runtime.forget_closed_files();
        if self
            .documents
            .pending_file_conflict
            .as_ref()
            .is_some_and(|conflict| closed.contains(&conflict.document_id))
        {
            self.documents.pending_file_conflict = None;
        }
        if self
            .project
            .pending_editor_focus_document_id
            .as_ref()
            .is_some_and(|document_id| closed.contains(document_id))
        {
            self.project.pending_editor_focus_document_id = None;
        }
        for document_id in self.project.project_editor_runtime.unloaded_files() {
            let Some(relative_path) =
                self.project
                    .services
                    .get(&document_id.project_id)
                    .and_then(|services| {
                        services
                            .relative_path_for_document(&document_id.canonical_path)
                            .ok()
                    })
            else {
                continue;
            };
            if crate::ui::editor::preview::is_image_path(&relative_path)
                && !crate::ui::editor::preview::is_svg_path(&relative_path)
            {
                self.load_file_preview(document_id, relative_path, false, window, cx);
                continue;
            }
            let Some(generation) = self
                .project
                .project_editor_runtime
                .begin_file_load(document_id.clone())
            else {
                continue;
            };
            self.spawn_project_file_load(
                ProjectFileLoadRequest {
                    document_id,
                    relative_path,
                    generation,
                    activate: false,
                },
                window,
                cx,
            );
        }
    }
}
