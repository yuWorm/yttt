use super::*;

const ACTIVE_PROJECT_FILE_WATCH_DEBOUNCE: Duration = Duration::from_millis(150);
const MAX_INCREMENTAL_PROJECT_TREE_DIRECTORIES: usize = 256;

#[derive(Default)]
struct ProjectFileRefreshBatch {
    tree_directories: BTreeSet<PathBuf>,
    refresh_all_expanded: bool,
    refresh_status: bool,
    preview_paths: BTreeSet<PathBuf>,
    refresh_all_previews: bool,
}

impl ProjectFileRefreshBatch {
    fn initial() -> Self {
        Self {
            refresh_all_expanded: true,
            refresh_status: true,
            refresh_all_previews: true,
            ..Self::default()
        }
    }

    fn record_change(&mut self, change: ProjectChange) -> bool {
        if change.refresh_all || change.relative_paths.is_empty() {
            self.refresh_all_previews = true;
            self.preview_paths.clear();
        } else if !self.refresh_all_previews {
            for path in &change.relative_paths {
                self.preview_paths
                    .insert(crate::runtime::project::relative_os_path(path.clone()));
                if self.preview_paths.len() > MAX_INCREMENTAL_PROJECT_TREE_DIRECTORIES {
                    self.refresh_all_previews = true;
                    self.preview_paths.clear();
                    break;
                }
            }
        }
        if !change.refresh_status {
            return false;
        }
        self.refresh_status = true;
        if !change.refresh_tree || self.refresh_all_expanded {
            return true;
        }
        if change.refresh_all {
            self.refresh_all_expanded = true;
            self.tree_directories.clear();
            return true;
        }

        let mut found_project_path = false;
        for relative_path in change.relative_paths {
            let relative_path = crate::runtime::project::relative_os_path(relative_path);
            let relative_directory = relative_path.parent().unwrap_or_else(|| Path::new(""));
            self.tree_directories
                .insert(relative_directory.to_path_buf());
            found_project_path = true;
            if self.tree_directories.len() > MAX_INCREMENTAL_PROJECT_TREE_DIRECTORIES {
                self.refresh_all_expanded = true;
                self.tree_directories.clear();
                break;
            }
        }
        if !found_project_path {
            self.refresh_all_expanded = true;
            self.tree_directories.clear();
        }
        true
    }

    fn has_tree_refresh(&self) -> bool {
        self.refresh_all_expanded || !self.tree_directories.is_empty()
    }
}

pub(super) enum ProjectDocumentLoadError {
    File(ProjectFileIoError),
    ProjectSettings(crate::config::project_settings::ProjectSettingsError),
}

pub(super) fn load_project_document_with_overrides(
    services: &ProjectServices,
    config_paths: &AppConfigPaths,
    project_path: Option<&Path>,
    relative_path: &Path,
) -> Result<
    (
        LoadedProjectFile,
        crate::config::project_settings::ProjectEditorOverrides,
    ),
    ProjectDocumentLoadError,
> {
    let loaded = services
        .read_file(relative_path)
        .map_err(ProjectDocumentLoadError::File)?;
    let overrides = match project_path {
        Some(path) => crate::config::project_settings::load_project_overrides(config_paths, path)
            .map_err(ProjectDocumentLoadError::ProjectSettings)?,
        // SSH roots are not paths in the Host configuration filesystem.
        None => crate::config::project_settings::ProjectEditorOverrides::default(),
    };
    Ok((loaded, overrides))
}

pub(super) struct ProjectEditorSettingsIo {
    pub(super) project_id: ProjectId,
    pub(super) config_paths: AppConfigPaths,
    pub(super) project_path: PathBuf,
    pub(super) host_editor_settings: crate::config::settings::EditorSettings,
    pub(super) can_write_host: bool,
}

impl WorkbenchView {
    pub(super) fn selected_project_editor_settings_io(
        &self,
    ) -> Result<ProjectEditorSettingsIo, String> {
        let project_id = self
            .workspace
            .selected_project_id()
            .cloned()
            .ok_or_else(|| "No project is selected.".to_string())?;
        let project_path = self
            .workspace
            .project(&project_id)
            .and_then(|project| project.location.local_path())
            .cloned()
            .ok_or_else(|| "The selected project has no Host configuration path.".to_string())?;
        Ok(ProjectEditorSettingsIo {
            project_id,
            config_paths: self.config_paths.clone(),
            project_path,
            host_editor_settings: self.app_settings.editor.clone(),
            can_write_host: self.shared_mutation_allowed(),
        })
    }
}

impl WorkbenchView {
    pub fn refresh_project_tree_state(
        &mut self,
        project_id: &ProjectId,
    ) -> Option<DirectoryLoadRequest> {
        let request = self
            .project
            .project_editor_runtime
            .workspace_mut()
            .session_mut(project_id)?
            .file_tree_mut()
            .refresh();
        self.project
            .project_editor_runtime
            .track_tree_load(project_id.clone(), request.generation);
        Some(request)
    }

    pub(super) fn refresh_expanded_project_tree_states(
        &mut self,
        project_id: &ProjectId,
    ) -> Vec<DirectoryLoadRequest> {
        let Some(session) = self
            .project
            .project_editor_runtime
            .workspace_mut()
            .session_mut(project_id)
        else {
            return Vec::new();
        };
        let requests = session.file_tree_mut().refresh_expanded();
        if let Some(request) = requests.first() {
            self.project
                .project_editor_runtime
                .track_tree_load(project_id.clone(), request.generation);
        }
        requests
    }

    pub(super) fn refresh_project_tree_directory_states(
        &mut self,
        project_id: &ProjectId,
        directories: &BTreeSet<PathBuf>,
    ) -> Vec<DirectoryLoadRequest> {
        let Some(session) = self
            .project
            .project_editor_runtime
            .workspace_mut()
            .session_mut(project_id)
        else {
            return Vec::new();
        };
        let requests = session
            .file_tree_mut()
            .refresh_directories(directories.iter().cloned());
        if let Some(request) = requests.first() {
            self.project
                .project_editor_runtime
                .track_tree_load(project_id.clone(), request.generation);
        }
        requests
    }

    pub(super) fn ensure_active_project_file_watcher(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.project_file_watching_enabled {
            self.active_project_file_watcher = None;
            return;
        }
        let Some((project_id, project_path)) =
            self.workspace.selected_project_id().and_then(|project_id| {
                self.workspace.project(project_id).and_then(|project| {
                    project
                        .location
                        .local_path()
                        .map(|path| (project_id.clone(), path.clone()))
                })
            })
        else {
            self.active_project_file_watcher = None;
            return;
        };
        if self.active_project_file_watcher_matches(&project_id, &project_path) {
            return;
        }

        self.active_project_file_watcher = None;
        let Some(services) = self.project.services.get(&project_id).cloned() else {
            return;
        };
        if services.host_registration_epoch().is_none() {
            return;
        }
        let Some(runtime) = self.terminal.host_runtime.as_ref() else {
            return;
        };
        let events = runtime.events();
        let pending_refresh = Arc::new(std::sync::Mutex::new(ProjectFileRefreshBatch::initial()));
        let watched_project_id = project_id.clone();
        let watched_project_path = project_path.clone();
        let task = cx.spawn_in(window, async move |this, cx| {
            let mut initial = true;
            loop {
                if !initial {
                    let change = loop {
                        let Ok(event) = events.recv_async().await else {
                            return;
                        };
                        let yttt_client_core::ClientEvent::Server(event) = event else {
                            continue;
                        };
                        let ServerEvent::ProjectChanged(change) = event.body else {
                            continue;
                        };
                        if change.project_id == watched_project_id
                            && services.host_registration_epoch() == Some(change.registration_epoch)
                        {
                            break change;
                        }
                    };
                    pending_refresh
                        .lock()
                        .expect("project file refresh batch mutex poisoned")
                        .record_change(change);
                }
                initial = false;

                cx.background_executor()
                    .timer(ACTIVE_PROJECT_FILE_WATCH_DEBOUNCE)
                    .await;
                while let Ok(event) = events.try_recv() {
                    let yttt_client_core::ClientEvent::Server(event) = event else {
                        continue;
                    };
                    let ServerEvent::ProjectChanged(change) = event.body else {
                        continue;
                    };
                    if change.project_id == watched_project_id
                        && services.host_registration_epoch() == Some(change.registration_epoch)
                    {
                        pending_refresh
                            .lock()
                            .expect("project file refresh batch mutex poisoned")
                            .record_change(change);
                    }
                }
                let refresh = {
                    let mut pending = pending_refresh
                        .lock()
                        .expect("project file refresh batch mutex poisoned");
                    std::mem::take(&mut *pending)
                };
                if !refresh.refresh_status && !refresh.has_tree_refresh() {
                    continue;
                }

                let is_active = this
                    .update_in(cx, |root, window, cx| {
                        if !root.active_project_file_watcher_matches(
                            &watched_project_id,
                            &watched_project_path,
                        ) {
                            return false;
                        }
                        let previews = root
                            .project
                            .project_editor_runtime
                            .previews_for_project(&watched_project_id)
                            .filter_map(|(_, preview)| {
                                let path = &preview.read(cx).relative_path;
                                (refresh.refresh_all_previews
                                    || refresh
                                        .preview_paths
                                        .iter()
                                        .any(|changed| path.starts_with(changed)))
                                .then(|| path.clone())
                            })
                            .collect::<Vec<_>>();
                        for path in previews {
                            root.open_file_preview(watched_project_id.clone(), path, window, cx);
                        }
                        let tree_load_queued = if refresh.refresh_all_expanded {
                            root.queue_project_tree_refresh(watched_project_id.clone())
                        } else if !refresh.tree_directories.is_empty() {
                            root.queue_project_tree_directories_refresh(
                                watched_project_id.clone(),
                                &refresh.tree_directories,
                            )
                        } else {
                            false
                        };
                        if tree_load_queued {
                            cx.notify();
                        } else {
                            root.check_project_documents_for_external_changes(
                                &watched_project_id,
                                window,
                                cx,
                            );
                        }
                        true
                    })
                    .unwrap_or(false);
                if !is_active || !refresh.refresh_status {
                    continue;
                }

                let status_services = services.clone();
                let status_task = cx
                    .background_executor()
                    .spawn(async move { read_project_git_status_with(&status_services) });
                let status = status_task.await;
                let _ = this.update_in(cx, |root, _window, cx| {
                    if !root.active_project_file_watcher_matches(
                        &watched_project_id,
                        &watched_project_path,
                    ) {
                        return;
                    }
                    root.apply_project_git_status(&watched_project_id, status);
                    cx.notify();
                });
            }
        });
        self.active_project_file_watcher = Some(ActiveProjectFileWatcher {
            project_id,
            project_path,
            _task: task,
        });
    }

    fn active_project_file_watcher_matches(
        &self,
        project_id: &ProjectId,
        project_path: &Path,
    ) -> bool {
        self.workspace.selected_project_id() == Some(project_id)
            && self
                .active_project_file_watcher
                .as_ref()
                .is_some_and(|watcher| {
                    &watcher.project_id == project_id && watcher.project_path == project_path
                })
    }

    pub fn apply_project_tree_snapshot(
        &mut self,
        project_id: &ProjectId,
        generation: u64,
        snapshot: DirectorySnapshot,
    ) -> bool {
        if !self
            .project
            .project_editor_runtime
            .tree_load_is_current(project_id, generation)
        {
            return false;
        }
        self.project
            .project_editor_runtime
            .workspace_mut()
            .session_mut(project_id)
            .is_some_and(|session| session.file_tree_mut().apply_snapshot(generation, snapshot))
    }

    pub fn apply_project_tree_error(
        &mut self,
        project_id: &ProjectId,
        generation: u64,
        relative_directory: &Path,
        error: impl Into<String>,
    ) -> bool {
        if !self
            .project
            .project_editor_runtime
            .tree_load_is_current(project_id, generation)
        {
            return false;
        }
        self.project
            .project_editor_runtime
            .workspace_mut()
            .session_mut(project_id)
            .is_some_and(|session| {
                session
                    .file_tree_mut()
                    .apply_error(generation, relative_directory, error)
            })
    }

    pub(super) fn project_tree_render_snapshot(
        &self,
        project_id: &ProjectId,
    ) -> Option<ProjectTreeRenderSnapshot> {
        let session = self
            .project
            .project_editor_runtime
            .workspace()
            .session(project_id)?;
        Some(ProjectTreeRenderSnapshot::from_tree_with_text(
            session.file_tree(),
            self.project.project_git_statuses.get(project_id),
            &ProjectTreeRenderText {
                loading: self.ui_text.get(UiTextKey::ProjectFilesLoading).to_string(),
                empty_directory: self
                    .ui_text
                    .get(UiTextKey::ProjectFilesEmptyDirectory)
                    .to_string(),
                retry: self.ui_text.get(UiTextKey::ProjectFilesRetry).to_string(),
            },
        ))
    }

    fn project_tree_interaction_text(&self) -> ProjectTreeInteractionText {
        ProjectTreeInteractionText {
            open: self.ui_text.get(UiTextKey::FileOpen).into(),
            open_external: self
                .ui_text
                .get(
                    if self
                        .workspace
                        .selected_project_id()
                        .and_then(|id| self.project.services.get(id))
                        .is_some_and(ProjectServices::requires_download)
                    {
                        UiTextKey::FileDownloadOpen
                    } else {
                        UiTextKey::FileOpenExternal
                    },
                )
                .into(),
            new_file: self.ui_text.get(UiTextKey::ProjectFilesNewFile).to_string(),
            new_directory: self
                .ui_text
                .get(UiTextKey::ProjectFilesNewDirectory)
                .to_string(),
            create_project_layout: self
                .ui_text
                .get(UiTextKey::ProjectFilesCreateProjectLayout)
                .to_string(),
            refresh: self.ui_text.get(UiTextKey::ProjectFilesRefresh).to_string(),
            rename: self.ui_text.get(UiTextKey::ProjectFilesRename).to_string(),
            delete: self.ui_text.get(UiTextKey::ProjectFilesDelete).to_string(),
            copy: self.ui_text.get(UiTextKey::ProjectFilesCopy).to_string(),
            cut: self.ui_text.get(UiTextKey::ProjectFilesCut).to_string(),
            paste: self.ui_text.get(UiTextKey::ProjectFilesPaste).to_string(),
            show_hidden: self
                .ui_text
                .get(UiTextKey::ProjectFilesShowHidden)
                .to_string(),
            hide_hidden: self
                .ui_text
                .get(UiTextKey::ProjectFilesHideHidden)
                .to_string(),
            entry_placeholder: self
                .ui_text
                .get(UiTextKey::ProjectFilesEntryPlaceholder)
                .to_string(),
        }
    }

    pub(super) fn ensure_project_tree_view(
        &mut self,
        project_id: &ProjectId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Entity<ProjectTreeView>> {
        if let Some(tree) = self
            .project
            .project_editor_runtime
            .tree(project_id)
            .cloned()
        {
            let shared_mutation_allowed = self.shared_mutation_allowed();
            let interaction_text = self.project_tree_interaction_text();
            let show_hidden = self.app_settings.project_panel.show_hidden;
            if let Some(snapshot) = self.project_tree_render_snapshot(project_id) {
                tree.update(cx, |tree, tree_cx| {
                    tree.sync_with_icon_theme(snapshot, self.icon_theme.clone(), tree_cx);
                    tree.set_interaction_text(interaction_text, tree_cx);
                    tree.set_show_hidden(show_hidden, tree_cx);
                    tree.set_shared_mutation_allowed(shared_mutation_allowed, tree_cx);
                });
            }
            return Some(tree);
        }

        let request = self
            .project
            .project_editor_runtime
            .workspace_mut()
            .session_mut(project_id)?
            .file_tree_mut()
            .request_expand(Path::new(""));
        if let Some(request) = &request {
            self.project
                .project_editor_runtime
                .track_tree_load(project_id.clone(), request.generation);
        }
        let snapshot = self.project_tree_render_snapshot(project_id)?;
        let icon_theme = self.icon_theme.clone();
        let interaction_text = self.project_tree_interaction_text();
        let show_hidden = self.app_settings.project_panel.show_hidden;
        let shared_mutation_allowed = self.shared_mutation_allowed();
        let tree = cx.new(|tree_cx| {
            let mut tree = ProjectTreeView::new_with_icon_theme(snapshot, icon_theme, tree_cx);
            tree.set_interaction_text(interaction_text, tree_cx);
            tree.set_show_hidden(show_hidden, tree_cx);
            tree.set_shared_mutation_allowed(shared_mutation_allowed, tree_cx);
            tree.set_show_focus_indicator(false, tree_cx);
            tree
        });
        let event_project_id = project_id.clone();
        let subscription = cx.subscribe_in(&tree, window, move |this, tree, event, window, cx| {
            this.on_project_tree_view_event(&event_project_id, tree, event, window, cx);
        });
        self.project.project_editor_runtime.insert_tree(
            project_id.clone(),
            tree.clone(),
            subscription,
        );
        if let Some(request) = request {
            self.spawn_project_directory_scan(project_id.clone(), request, window, cx);
        }
        Some(tree)
    }

    pub(super) fn on_project_tree_view_event(
        &mut self,
        project_id: &ProjectId,
        _tree: &Entity<ProjectTreeView>,
        event: &ProjectTreeViewEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            ProjectTreeViewEvent::SelectPath(path) => {
                if let Some(session) = self
                    .project
                    .project_editor_runtime
                    .workspace_mut()
                    .session_mut(project_id)
                {
                    session.file_tree_mut().select(Some(path.clone()));
                }
            }
            ProjectTreeViewEvent::ToggleDirectory { path, expanded } => {
                let request = self
                    .project
                    .project_editor_runtime
                    .workspace_mut()
                    .session_mut(project_id)
                    .and_then(|session| {
                        let tree = session.file_tree_mut();
                        tree.select(Some(path.clone()));
                        if *expanded {
                            tree.request_expand(path)
                        } else {
                            tree.collapse(path);
                            None
                        }
                    });
                if let Some(request) = request {
                    self.project
                        .project_editor_runtime
                        .track_tree_load(project_id.clone(), request.generation);
                    self.spawn_project_directory_scan(project_id.clone(), request, window, cx);
                }
            }
            ProjectTreeViewEvent::OpenFile(path) => {
                if let Some(session) = self
                    .project
                    .project_editor_runtime
                    .workspace_mut()
                    .session_mut(project_id)
                {
                    session.file_tree_mut().select(Some(path.clone()));
                }
                self.spawn_project_file_open(project_id.clone(), path.clone(), window, cx);
            }
            ProjectTreeViewEvent::OpenExternal(path) => {
                self.open_project_file_external(project_id.clone(), path.clone(), window, cx);
            }
            ProjectTreeViewEvent::CreateEntry { parent, input } => {
                if self.require_shared_mutation_control() {
                    self.spawn_project_entry_create(
                        project_id.clone(),
                        parent.clone(),
                        input.clone(),
                        window,
                        cx,
                    );
                }
            }
            ProjectTreeViewEvent::CreateProjectLayout => {
                if self.require_shared_mutation_control() {
                    self.spawn_project_layout_scaffold(project_id.clone(), window, cx);
                }
            }
            ProjectTreeViewEvent::RenameEntry { path, new_name } => {
                if self.require_shared_mutation_control() {
                    self.spawn_project_entry_rename(
                        project_id.clone(),
                        path.clone(),
                        new_name.clone(),
                        window,
                        cx,
                    );
                }
            }
            ProjectTreeViewEvent::RequestDelete(path) => {
                if self.require_shared_mutation_control() {
                    self.confirm_project_entry_delete(project_id.clone(), path.clone(), window, cx);
                }
            }
            ProjectTreeViewEvent::CopyEntry(path) => {
                self.project.project_tree_clipboard = Some(ProjectTreeClipboard {
                    source_project_id: project_id.clone(),
                    relative_path: path.clone(),
                    mode: ProjectEntryPasteMode::Copy,
                });
            }
            ProjectTreeViewEvent::CutEntry(path) => {
                if self.require_shared_mutation_control() {
                    self.project.project_tree_clipboard = Some(ProjectTreeClipboard {
                        source_project_id: project_id.clone(),
                        relative_path: path.clone(),
                        mode: ProjectEntryPasteMode::Cut,
                    });
                }
            }
            ProjectTreeViewEvent::PasteEntry {
                destination_directory,
            } => {
                if self.require_shared_mutation_control() {
                    self.spawn_project_entry_paste(
                        project_id.clone(),
                        destination_directory.clone(),
                        window,
                        cx,
                    );
                }
            }
            ProjectTreeViewEvent::SetShowHidden(show_hidden) => {
                if let Err(error) = self.set_project_panel_show_hidden(*show_hidden) {
                    self.load_error = Some(error.to_string());
                }
            }
            ProjectTreeViewEvent::Refresh => {
                self.refresh_project_tree(project_id.clone(), window, cx);
            }
        }
        cx.notify();
    }

    fn relocate_open_project_documents(
        &mut self,
        source_project_id: &ProjectId,
        source_relative_path: &Path,
        destination_project_id: &ProjectId,
        destination_relative_path: &Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(source_services) = self.project.services.get(source_project_id).cloned() else {
            return;
        };
        let Some(destination_services) = self.project.services.get(destination_project_id).cloned()
        else {
            return;
        };
        let Some(source_base) = source_services.document_path(source_relative_path) else {
            return;
        };
        let Some(destination_base) = destination_services.document_path(destination_relative_path)
        else {
            return;
        };
        let preview_migrations = self
            .project
            .project_editor_runtime
            .previews_for_project(source_project_id)
            .filter_map(|(id, preview)| {
                let suffix = id.canonical_path.strip_prefix(&source_base).ok()?;
                Some((
                    id.clone(),
                    crate::ui::editor::DocumentId {
                        project_id: destination_project_id.clone(),
                        canonical_path: destination_base.join(suffix),
                    },
                    destination_relative_path.join(suffix),
                    preview.clone(),
                ))
            })
            .collect::<Vec<_>>();
        let migrations = self
            .project
            .project_editor_runtime
            .documents_for_project(source_project_id)
            .filter_map(|(document_id, _)| {
                let suffix = document_id.canonical_path.strip_prefix(&source_base).ok()?;
                let destination_relative = destination_relative_path.join(suffix);
                let new_document_id = crate::ui::editor::DocumentId {
                    project_id: destination_project_id.clone(),
                    canonical_path: destination_base.join(suffix),
                };
                Some((
                    document_id.clone(),
                    new_document_id,
                    destination_relative.to_string_lossy().into_owned(),
                ))
            })
            .collect::<Vec<_>>();

        for (old_document_id, new_document_id, breadcrumb_header) in migrations {
            let Some(document) = self
                .project
                .project_editor_runtime
                .relocate_document(&old_document_id, new_document_id.clone())
            else {
                continue;
            };
            document.update(cx, |document, document_cx| {
                document.relocate(
                    new_document_id.clone(),
                    breadcrumb_header.clone(),
                    window,
                    document_cx,
                );
            });
            self.relocate_pending_document_id(&old_document_id, &new_document_id);
        }
        for (old, new, relative_path, preview) in preview_migrations {
            if self
                .project
                .project_editor_runtime
                .relocate_preview(&old, new.clone())
            {
                preview.update(cx, |preview, cx| {
                    preview.relative_path = relative_path.clone();
                    cx.notify();
                });
                self.relocate_pending_document_id(&old, &new);
                self.open_file_preview(new.project_id, relative_path, window, cx);
            }
        }
    }

    fn relocate_pending_document_id(
        &mut self,
        old: &crate::ui::editor::DocumentId,
        new: &crate::ui::editor::DocumentId,
    ) {
        for pending in self
            .documents
            .pending_document_saves
            .iter_mut()
            .chain(self.documents.pending_focus_change_autosaves.iter_mut())
            .chain(self.documents.pending_file_close_requests.iter_mut())
        {
            if pending == old {
                *pending = new.clone();
            }
        }
        if self.project.pending_editor_focus_document_id.as_ref() == Some(old) {
            self.project.pending_editor_focus_document_id = Some(new.clone());
        }
        if let Some(conflict) = self.documents.pending_file_conflict.as_mut()
            && &conflict.document_id == old
        {
            conflict.document_id = new.clone();
            conflict.request.document_id = new.clone();
        }
        if let Some(pending) = self.documents.pending_dirty_close.as_mut() {
            if let DirtyCloseIntent::File(document_id) = &mut pending.intent
                && document_id == old
            {
                *document_id = new.clone();
            }
            if let DirtyCloseIntent::WorkItems { file_ids, .. } = &mut pending.intent {
                for document_id in file_ids {
                    if document_id == old {
                        *document_id = new.clone();
                    }
                }
            }
            for document_id in &mut pending.dirty_documents {
                if document_id == old {
                    *document_id = new.clone();
                }
            }
            if pending.saving_documents.remove(old) {
                pending.saving_documents.insert(new.clone());
            }
        }
    }

    fn spawn_project_entry_create(
        &mut self,
        project_id: ProjectId,
        parent: PathBuf,
        input: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.require_shared_mutation_control() {
            return;
        }
        let Some(services) = self.project.services.get(&project_id).cloned() else {
            return;
        };
        let refresh_parent = parent.clone();
        let io_task = cx.background_spawn(async move { services.create_entry(&parent, &input) });
        cx.spawn_in(window, async move |this, cx| {
            let result = io_task.await;
            let _ = this.update_in(cx, |root, window, cx| {
                match result {
                    Ok(created) => {
                        if let Some(session) = root
                            .project
                            .project_editor_runtime
                            .workspace_mut()
                            .session_mut(&project_id)
                        {
                            session
                                .file_tree_mut()
                                .select(Some(created.relative_path.clone()));
                        }
                        root.load_error = None;
                        root.refresh_project_tree_after_mutation(
                            project_id.clone(),
                            BTreeSet::from([refresh_parent.clone()]),
                            window,
                            cx,
                        );
                        if !created.kind.is_directory() {
                            root.spawn_project_file_open(
                                project_id.clone(),
                                created.relative_path,
                                window,
                                cx,
                            );
                        }
                    }
                    Err(error) => {
                        root.load_error = Some(root.localized_project_entry_error(&error));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn spawn_project_layout_scaffold(
        &mut self,
        project_id: ProjectId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.require_shared_mutation_control() {
            return;
        }
        let Some((project_path, layout)) =
            self.workspace.project(&project_id).and_then(|project| {
                project
                    .location
                    .local_path()
                    .map(|path| (path.clone(), project.layout.clone()))
            })
        else {
            return;
        };
        let config_paths = self.config_paths.clone();
        let io_task = cx.background_spawn(async move {
            create_project_layout_scaffold(&config_paths, &project_path, &layout)
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = io_task.await;
            let _ = this.update_in(cx, |root, window, cx| {
                match result {
                    Ok(scaffold) => {
                        root.load_error = None;
                        root.refresh_project_tree(project_id.clone(), window, cx);
                        root.queue_status_notification(
                            root.ui_text.get(UiTextKey::ProjectFilesProjectLayoutReady),
                            format!(
                                "{} · {}",
                                scaffold.layout_file.display(),
                                scaffold.guide_file.display()
                            ),
                        );
                    }
                    Err(error) => {
                        root.load_error = Some(error.to_string());
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn spawn_project_entry_rename(
        &mut self,
        project_id: ProjectId,
        relative_path: PathBuf,
        new_name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.require_shared_mutation_control() {
            return;
        }
        let Some(services) = self.project.services.get(&project_id).cloned() else {
            return;
        };
        let refresh_parent = relative_path
            .parent()
            .unwrap_or_else(|| Path::new(""))
            .to_path_buf();
        let moved_relative_path = relative_path.clone();
        let io_task =
            cx.background_spawn(async move { services.rename_entry(&relative_path, &new_name) });
        cx.spawn_in(window, async move |this, cx| {
            let result = io_task.await;
            let _ = this.update_in(cx, |root, window, cx| {
                match result {
                    Ok(renamed) => {
                        root.relocate_open_project_documents(
                            &project_id,
                            &moved_relative_path,
                            &project_id,
                            &renamed.relative_path,
                            window,
                            cx,
                        );
                        if let Some(session) = root
                            .project
                            .project_editor_runtime
                            .workspace_mut()
                            .session_mut(&project_id)
                        {
                            session
                                .file_tree_mut()
                                .select(Some(renamed.relative_path.clone()));
                        }
                        root.load_error = None;
                        root.refresh_project_tree_after_mutation(
                            project_id.clone(),
                            BTreeSet::from([refresh_parent.clone()]),
                            window,
                            cx,
                        );
                    }
                    Err(error) => {
                        root.load_error = Some(root.localized_project_entry_error(&error));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn confirm_project_entry_delete(
        &mut self,
        project_id: ProjectId,
        relative_path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.require_shared_mutation_control() {
            return;
        }
        let title = self
            .ui_text
            .get(UiTextKey::ProjectFilesDeleteConfirmTitle)
            .to_string();
        let message = self
            .ui_text
            .get(UiTextKey::ProjectFilesDeleteConfirmMessage)
            .to_string();
        let delete_label = self.ui_text.get(UiTextKey::ProjectFilesDelete).to_string();
        let cancel_label = self.ui_text.get(UiTextKey::Cancel).to_string();
        let appearance = self.theme_runtime();
        let theme = appearance.ui;
        let ui_style = appearance.style;
        let display_path = relative_path.display().to_string();
        let workbench = cx.weak_entity();
        window.open_alert_dialog(cx, move |alert, _, cx| {
            let workbench = workbench.clone();
            let project_id = project_id.clone();
            let relative_path = relative_path.clone();
            let delete_label = delete_label.clone();
            let cancel_label = cancel_label.clone();
            alert
                .title(title.clone())
                .description(
                    div()
                        .flex()
                        .flex_col()
                        .gap(ui_style.spacing.md)
                        .child(display_path.clone())
                        .child(message.clone()),
                )
                .footer(
                    DialogFooter::new()
                        .child(
                            yttt_button(
                                "project-entry-delete-cancel",
                                cancel_label.clone(),
                                YtttButtonVariant::Secondary,
                                theme,
                                ui_style,
                                cx,
                            )
                            .debug_selector(|| "project-entry-delete-cancel".to_string())
                            .on_click(|_, window, cx| {
                                window.close_dialog(cx);
                            }),
                        )
                        .child(
                            yttt_button(
                                "project-entry-delete-confirm",
                                delete_label.clone(),
                                YtttButtonVariant::Danger,
                                theme,
                                ui_style,
                                cx,
                            )
                            .debug_selector(|| "project-entry-delete-confirm".to_string())
                            .on_click(move |_, window, cx| {
                                let _ = workbench.update(cx, |root, root_cx| {
                                    root.spawn_project_entry_delete(
                                        project_id.clone(),
                                        relative_path.clone(),
                                        window,
                                        root_cx,
                                    );
                                });
                                window.close_dialog(cx);
                            }),
                        ),
                )
        });
    }

    pub(super) fn spawn_project_entry_delete(
        &mut self,
        project_id: ProjectId,
        relative_path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.require_shared_mutation_control() {
            return;
        }
        let Some(services) = self.project.services.get(&project_id).cloned() else {
            return;
        };
        let refresh_parent = relative_path
            .parent()
            .unwrap_or_else(|| Path::new(""))
            .to_path_buf();
        let io_task = cx.background_spawn(async move { services.delete_entry(&relative_path) });
        cx.spawn_in(window, async move |this, cx| {
            let result = io_task.await;
            let _ = this.update_in(cx, |root, window, cx| {
                match result {
                    Ok(()) => {
                        if let Some(session) = root
                            .project
                            .project_editor_runtime
                            .workspace_mut()
                            .session_mut(&project_id)
                        {
                            session.file_tree_mut().select(None);
                        }
                        root.load_error = None;
                        root.refresh_project_tree_after_mutation(
                            project_id.clone(),
                            BTreeSet::from([refresh_parent.clone()]),
                            window,
                            cx,
                        );
                    }
                    Err(error) => {
                        root.load_error = Some(root.localized_project_entry_error(&error));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn spawn_project_entry_paste(
        &mut self,
        destination_project_id: ProjectId,
        destination_directory: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.require_shared_mutation_control() {
            return;
        }
        let Some(clipboard) = self.project.project_tree_clipboard.clone() else {
            return;
        };
        let Some(source_services) = self
            .project
            .services
            .get(&clipboard.source_project_id)
            .cloned()
        else {
            self.project.project_tree_clipboard = None;
            return;
        };
        let Some(destination_services) =
            self.project.services.get(&destination_project_id).cloned()
        else {
            return;
        };
        let source_relative_path = clipboard.relative_path.clone();
        let mode = clipboard.mode;
        let destination_refresh_directory = destination_directory.clone();
        let source_refresh_directory = clipboard
            .relative_path
            .parent()
            .unwrap_or_else(|| Path::new(""))
            .to_path_buf();
        let io_task = cx.background_spawn(async move {
            source_services.paste_entry(
                &source_relative_path,
                &destination_services,
                &destination_directory,
                mode,
            )
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = io_task.await;
            let _ = this.update_in(cx, |root, window, cx| {
                match result {
                    Ok(pasted) => {
                        if mode == ProjectEntryPasteMode::Cut {
                            root.relocate_open_project_documents(
                                &clipboard.source_project_id,
                                &clipboard.relative_path,
                                &destination_project_id,
                                &pasted.relative_path,
                                window,
                                cx,
                            );
                        }
                        if mode == ProjectEntryPasteMode::Cut
                            && root.project.project_tree_clipboard.as_ref() == Some(&clipboard)
                        {
                            root.project.project_tree_clipboard = None;
                        }
                        if let Some(session) = root
                            .project
                            .project_editor_runtime
                            .workspace_mut()
                            .session_mut(&destination_project_id)
                        {
                            session
                                .file_tree_mut()
                                .select(Some(pasted.relative_path.clone()));
                        }
                        root.load_error = None;
                        if clipboard.source_project_id != destination_project_id
                            && mode == ProjectEntryPasteMode::Cut
                        {
                            root.refresh_project_tree_after_mutation(
                                clipboard.source_project_id.clone(),
                                BTreeSet::from([source_refresh_directory.clone()]),
                                window,
                                cx,
                            );
                        }
                        let mut destination_refresh_directories =
                            BTreeSet::from([destination_refresh_directory.clone()]);
                        if clipboard.source_project_id == destination_project_id
                            && mode == ProjectEntryPasteMode::Cut
                        {
                            destination_refresh_directories
                                .insert(source_refresh_directory.clone());
                        }
                        root.refresh_project_tree_after_mutation(
                            destination_project_id.clone(),
                            destination_refresh_directories,
                            window,
                            cx,
                        );
                    }
                    Err(error) => {
                        root.load_error = Some(root.localized_project_entry_error(&error));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn refresh_project_tree(
        &mut self,
        project_id: ProjectId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        for request in self.refresh_expanded_project_tree_states(&project_id) {
            self.spawn_project_directory_scan(project_id.clone(), request, window, cx);
        }
        self.check_project_documents_for_external_changes(&project_id, window, cx);
        self.refresh_project_git_status(project_id, cx);
    }

    pub(super) fn refresh_project_git_status(
        &mut self,
        project_id: ProjectId,
        cx: &mut Context<Self>,
    ) {
        let Some((project_location, services)) = self
            .workspace
            .project(&project_id)
            .map(|project| project.location.clone())
            .zip(self.project.services.get(&project_id).cloned())
        else {
            return;
        };
        let task = cx.background_spawn(async move { read_project_git_status_with(&services) });
        cx.spawn(async move |this, cx| {
            let status = task.await;
            let _ = this.update(cx, |root, cx| {
                if root
                    .workspace
                    .project(&project_id)
                    .map(|project| &project.location)
                    != Some(&project_location)
                {
                    return;
                }
                root.apply_project_git_status(&project_id, status);
                cx.notify();
            });
        })
        .detach();
    }

    fn refresh_project_tree_after_mutation(
        &mut self,
        project_id: ProjectId,
        directories: BTreeSet<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        for request in self.refresh_project_tree_directory_states(&project_id, &directories) {
            self.spawn_project_directory_scan(project_id.clone(), request, window, cx);
        }
        self.check_project_documents_for_external_changes(&project_id, window, cx);
        self.refresh_project_git_status(project_id, cx);
    }

    pub(super) fn queue_project_tree_refresh(&mut self, project_id: ProjectId) -> bool {
        let requests = self.refresh_expanded_project_tree_states(&project_id);
        let queued = !requests.is_empty();
        for request in requests {
            self.project
                .pending_project_tree_loads
                .push((project_id.clone(), request));
        }
        queued
    }

    fn queue_project_tree_directories_refresh(
        &mut self,
        project_id: ProjectId,
        directories: &BTreeSet<PathBuf>,
    ) -> bool {
        let requests = self.refresh_project_tree_directory_states(&project_id, directories);
        let queued = !requests.is_empty();
        for request in requests {
            self.project
                .pending_project_tree_loads
                .push((project_id.clone(), request));
        }
        queued
    }

    pub(super) fn flush_pending_project_tree_loads(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let pending = std::mem::take(&mut self.project.pending_project_tree_loads);
        let mut checked_projects = HashSet::new();
        for (project_id, request) in pending {
            if checked_projects.insert(project_id.clone()) {
                self.check_project_documents_for_external_changes(&project_id, window, cx);
            }
            self.spawn_project_directory_scan(project_id, request, window, cx);
        }
    }

    pub(super) fn spawn_project_directory_scan(
        &mut self,
        project_id: ProjectId,
        request: DirectoryLoadRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self
            .project
            .project_editor_runtime
            .tree_load_is_current(&project_id, request.generation)
        {
            return;
        }
        let Some(services) = self.project.services.get(&project_id).cloned() else {
            return;
        };
        let relative_directory = request.relative_directory.clone();
        let generation = request.generation;
        let show_hidden = self.app_settings.project_panel.show_hidden;
        let scan_relative_directory = relative_directory.clone();
        let io_task = cx.background_spawn(async move {
            services.scan_directory(&scan_relative_directory, show_hidden)
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = io_task.await;
            let _ = this.update_in(cx, |root, _window, cx| {
                match result {
                    Ok(snapshot) => {
                        root.apply_project_tree_snapshot(&project_id, generation, snapshot);
                    }
                    Err(error) => {
                        let message = root.localized_project_tree_error(&error);
                        root.apply_project_tree_error(
                            &project_id,
                            generation,
                            &relative_directory,
                            message,
                        );
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn localized_project_tree_error(&self, error: &ProjectTreeFsError) -> String {
        format!(
            "{}: {error}",
            self.ui_text.get(UiTextKey::ProjectFilesDirectoryError)
        )
    }

    fn localized_project_entry_error(&self, error: &ProjectEntryFsError) -> String {
        format!(
            "{}: {error}",
            self.ui_text.get(UiTextKey::StatusErrorContext)
        )
    }

    pub(super) fn localized_project_file_error(&self, error: &ProjectFileIoError) -> String {
        let summary = match error {
            ProjectFileIoError::PathOutsideProject { .. } => {
                self.ui_text.get(UiTextKey::ProjectFileOutsideProject)
            }
            ProjectFileIoError::FileTooLarge { .. } => {
                self.ui_text.get(UiTextKey::ProjectFileTooLarge)
            }
            ProjectFileIoError::BinaryContent { .. } => {
                self.ui_text.get(UiTextKey::ProjectFileUnsupportedBinary)
            }
            ProjectFileIoError::InvalidUtf8 { .. } => {
                self.ui_text.get(UiTextKey::ProjectFileInvalidEncoding)
            }
            ProjectFileIoError::NotAFile { .. }
            | ProjectFileIoError::Io { .. }
            | ProjectFileIoError::Remote { .. } => self.ui_text.get(UiTextKey::StatusErrorContext),
        };
        format!("{summary}: {error}")
    }

    pub fn begin_project_file_open(
        &mut self,
        project_id: &ProjectId,
        relative_path: &Path,
    ) -> Option<ProjectFileLoadRequest> {
        let document_path = self
            .project
            .services
            .get(project_id)?
            .document_path(relative_path)?;
        self.project
            .project_editor_runtime
            .workspace()
            .session(project_id)?;
        let document_id = crate::ui::editor::DocumentId {
            project_id: project_id.clone(),
            canonical_path: document_path,
        };
        let generation = self
            .project
            .project_editor_runtime
            .begin_file_load(document_id.clone())?;
        Some(ProjectFileLoadRequest {
            document_id,
            relative_path: relative_path.to_path_buf(),
            generation,
        })
    }

    pub fn cancel_project_file_open(&mut self, request: &ProjectFileLoadRequest) -> bool {
        self.project
            .project_editor_runtime
            .finish_file_load(&request.document_id, request.generation)
    }

    pub fn apply_project_file_open_error(
        &mut self,
        request: &ProjectFileLoadRequest,
        error: impl Into<String>,
    ) -> bool {
        if !self
            .project
            .project_editor_runtime
            .finish_file_load(&request.document_id, request.generation)
        {
            return false;
        }
        self.load_error = Some(error.into());
        true
    }

    pub(super) fn spawn_project_file_open(
        &mut self,
        project_id: ProjectId,
        relative_path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let requested_document_id = self
            .project
            .services
            .get(&project_id)
            .and_then(|services| services.document_path(&relative_path))
            .map(|canonical_path| crate::ui::editor::DocumentId {
                project_id: project_id.clone(),
                canonical_path,
            });
        if let Some(document_id) = requested_document_id
            && (self
                .project
                .project_editor_runtime
                .document(&document_id)
                .is_some()
                || self
                    .project
                    .project_editor_runtime
                    .preview(&document_id)
                    .is_some())
        {
            let _ = self.select_work_item(WorkItemId::File(document_id));
            cx.notify();
            return;
        }
        if crate::ui::editor::preview::is_image_path(&relative_path)
            && !crate::ui::editor::preview::is_svg_path(&relative_path)
        {
            self.open_file_preview(project_id, relative_path, window, cx);
            return;
        }
        let Some(request) = self.begin_project_file_open(&project_id, &relative_path) else {
            return;
        };
        let Some(services) = self.project.services.get(&project_id).cloned() else {
            self.cancel_project_file_open(&request);
            return;
        };
        let project_path = self
            .workspace
            .project(&project_id)
            .and_then(|project| project.location.local_path())
            .cloned();
        let config_paths = self.config_paths.clone();
        let read_relative_path = request.relative_path.clone();
        let io_task = cx.background_spawn(async move {
            load_project_document_with_overrides(
                &services,
                &config_paths,
                project_path.as_deref(),
                &read_relative_path,
            )
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = io_task.await;
            let _ = this.update_in(cx, |root, window, cx| {
                match result {
                    Ok((loaded, overrides)) => {
                        root.apply_project_file_open_success(
                            &request, loaded, overrides, window, cx,
                        );
                    }
                    Err(ProjectDocumentLoadError::File(error)) => {
                        let message = root.localized_project_file_error(&error);
                        if root.cancel_project_file_open(&request) {
                            root.open_unavailable_file(&request, message, window, cx);
                        }
                    }
                    Err(ProjectDocumentLoadError::ProjectSettings(error)) => {
                        root.apply_project_file_open_error(&request, error.to_string());
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn apply_project_file_open_success(
        &mut self,
        request: &ProjectFileLoadRequest,
        loaded: LoadedProjectFile,
        overrides: crate::config::project_settings::ProjectEditorOverrides,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self
            .project
            .project_editor_runtime
            .finish_file_load(&request.document_id, request.generation)
            || self
                .workspace
                .project(&request.document_id.project_id)
                .is_none()
        {
            return false;
        }
        let document_id = crate::ui::editor::DocumentId {
            project_id: request.document_id.project_id.clone(),
            canonical_path: loaded.canonical_path.clone(),
        };
        if self
            .project
            .project_editor_runtime
            .document(&document_id)
            .is_none()
        {
            let editor_settings = &self.app_settings.editor;
            let language_mode = if overrides.auto_detect_language(editor_settings) {
                CodeEditorLanguageMode::Auto
            } else {
                CodeEditorLanguageMode::from(overrides.default_language(editor_settings))
            };
            let breadcrumb_header = loaded.relative_path.to_string_lossy().into_owned();
            let title = loaded
                .relative_path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| loaded.relative_path.to_string_lossy().into_owned());
            let config = CodeEditorConfig::new(title, language_mode)
                .with_editor_settings(editor_settings)
                .with_tab_size(overrides.tab_size(editor_settings));
            let model = ProjectEditorModel::new(
                document_id.clone(),
                CodeEditorState::new(&loaded.canonical_path, config, loaded.text),
                loaded.fingerprint,
            );
            let appearance = EditorAppearance::from(&self.app_settings.editor);
            let markdown_config = self.markdown_document_config();
            let vim_enabled = self.app_settings.vim.mode != VimModeSetting::Disabled;
            let document = cx.new(|document_cx| {
                ProjectEditorDocument::new_with_markdown_config(
                    model,
                    appearance,
                    markdown_config,
                    window,
                    document_cx,
                )
                .with_breadcrumb_header(breadcrumb_header)
                .with_vim_mode(vim_enabled, window, document_cx)
            });
            let subscription =
                cx.subscribe_in(&document, window, Self::on_project_editor_document_event);
            self.project.project_editor_runtime.insert_document(
                document_id.clone(),
                document,
                subscription,
            );
        }
        let opened_id = self
            .project
            .project_editor_runtime
            .workspace_mut()
            .session_mut(&document_id.project_id)
            .map(|session| session.open_file(document_id.canonical_path.clone()));
        let Some(opened_id) = opened_id else {
            self.project
                .project_editor_runtime
                .remove_document(&document_id);
            return false;
        };
        let _ = self.select_work_item(WorkItemId::File(opened_id));
        self.load_error = None;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn change(paths: &[&str], refresh_tree: bool, refresh_all: bool) -> ProjectChange {
        ProjectChange {
            project_id: ProjectId::new("project"),
            registration_epoch: 1,
            relative_paths: paths
                .iter()
                .map(|path| crate::runtime::project::path_to_relative(Path::new(path)).unwrap())
                .collect(),
            refresh_status: true,
            refresh_tree,
            refresh_all,
        }
    }

    #[test]
    fn content_changes_refresh_status_without_rescanning_tree() {
        let mut batch = ProjectFileRefreshBatch::default();

        assert!(batch.record_change(change(&["src/lib.rs"], false, false)));
        assert!(batch.refresh_status);
        assert!(!batch.has_tree_refresh());
    }

    #[test]
    fn structural_changes_coalesce_affected_parent_directories() {
        let mut batch = ProjectFileRefreshBatch::default();

        assert!(batch.record_change(change(&["src/new.rs", "src/old.rs"], true, false)));
        assert!(batch.record_change(change(&["tests/new.rs"], true, false)));

        assert_eq!(
            batch.tree_directories,
            BTreeSet::from([PathBuf::from("src"), PathBuf::from("tests")])
        );
        assert!(!batch.refresh_all_expanded);
        assert!(batch.refresh_status);
    }

    #[test]
    fn watcher_rescan_signal_falls_back_to_all_expanded_directories() {
        let mut batch = ProjectFileRefreshBatch::default();

        assert!(batch.record_change(change(&[], true, true)));
        assert!(batch.refresh_all_expanded);
        assert!(batch.tree_directories.is_empty());
        assert!(batch.refresh_status);
    }

    #[test]
    fn excessive_affected_directories_fall_back_to_bounded_full_refresh() {
        let mut batch = ProjectFileRefreshBatch::default();

        for index in 0..=MAX_INCREMENTAL_PROJECT_TREE_DIRECTORIES {
            let path = format!("directory-{index}/file.rs");
            assert!(batch.record_change(change(&[&path], true, false)));
        }

        assert!(batch.refresh_all_expanded);
        assert!(batch.tree_directories.is_empty());
    }
}
