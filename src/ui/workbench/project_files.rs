use super::*;

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
            if let Some(snapshot) = self.project_tree_render_snapshot(project_id) {
                tree.update(cx, |tree, tree_cx| {
                    tree.sync_with_icon_theme(snapshot, self.icon_theme.clone(), tree_cx)
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
        let tree =
            cx.new(|tree_cx| ProjectTreeView::new_with_icon_theme(snapshot, icon_theme, tree_cx));
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
            ProjectTreeViewEvent::Refresh => {
                self.refresh_project_tree(project_id.clone(), window, cx);
            }
        }
        cx.notify();
    }

    pub(super) fn refresh_project_tree(
        &mut self,
        project_id: ProjectId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let project_path = self
            .workspace
            .project(&project_id)
            .map(|project| project.path.clone());
        for request in self.refresh_expanded_project_tree_states(&project_id) {
            self.spawn_project_directory_scan(project_id.clone(), request, window, cx);
        }
        self.check_project_documents_for_external_changes(&project_id, window, cx);
        if let Some(project_path) = project_path {
            self.refresh_project_git_status(&project_id, &project_path);
        }
    }

    pub(super) fn queue_project_tree_refresh(&mut self, project_id: ProjectId) {
        for request in self.refresh_expanded_project_tree_states(&project_id) {
            self.project
                .pending_project_tree_loads
                .push((project_id.clone(), request));
        }
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
        let Some(project_root) = self
            .workspace
            .project(&project_id)
            .map(|project| project.path.clone())
        else {
            return;
        };
        let relative_directory = request.relative_directory.clone();
        let generation = request.generation;
        let show_hidden = self.app_settings.project_panel.show_hidden;
        let scan_relative_directory = relative_directory.clone();
        let io_task = cx.background_spawn(async move {
            scan_project_directory(&project_root, &scan_relative_directory, show_hidden)
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
            ProjectFileIoError::NotAFile { .. } | ProjectFileIoError::Io { .. } => {
                self.ui_text.get(UiTextKey::StatusErrorContext)
            }
        };
        format!("{summary}: {error}")
    }

    pub fn begin_project_file_open(
        &mut self,
        project_id: &ProjectId,
        relative_path: &Path,
    ) -> Option<ProjectFileLoadRequest> {
        let project_root = self.workspace.project(project_id)?.path.clone();
        self.project
            .project_editor_runtime
            .workspace()
            .session(project_id)?;
        let document_id = crate::ui::editor::DocumentId {
            project_id: project_id.clone(),
            canonical_path: project_root.join(relative_path),
        };
        let generation = self
            .project
            .project_editor_runtime
            .begin_file_load(document_id.clone())?;
        Some(ProjectFileLoadRequest {
            document_id,
            project_root,
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
        let requested_document_id =
            self.workspace
                .project(&project_id)
                .map(|project| crate::ui::editor::DocumentId {
                    project_id: project_id.clone(),
                    canonical_path: project.path.join(&relative_path),
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
                    .workspace()
                    .session(&project_id)
                    .is_some_and(|session| session.file_ids().contains(&document_id)))
        {
            let _ = self.select_work_item(WorkItemId::File(document_id));
            cx.notify();
            return;
        }
        let Some(request) = self.begin_project_file_open(&project_id, &relative_path) else {
            return;
        };
        let project_root = request.project_root.clone();
        let read_relative_path = request.relative_path.clone();
        let io_task = cx
            .background_spawn(async move { read_project_file(&project_root, &read_relative_path) });
        cx.spawn_in(window, async move |this, cx| {
            let result = io_task.await;
            let _ = this.update_in(cx, |root, window, cx| {
                match result {
                    Ok(loaded) => {
                        root.apply_project_file_open_success(&request, loaded, window, cx);
                    }
                    Err(error) => {
                        let message = root.localized_project_file_error(&error);
                        root.apply_project_file_open_error(&request, message);
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
            let language_mode = if self.app_settings.editor.auto_detect_language {
                CodeEditorLanguageMode::Auto
            } else {
                CodeEditorLanguageMode::from(self.app_settings.editor.default_language.clone())
            };
            let breadcrumb_header = loaded.relative_path.to_string_lossy().into_owned();
            let title = loaded
                .relative_path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| loaded.relative_path.to_string_lossy().into_owned());
            let config = CodeEditorConfig::new(title, language_mode)
                .with_tab_size(self.app_settings.editor.tab_size)
                .with_soft_wrap(self.app_settings.editor.soft_wrap)
                .with_line_number(self.app_settings.editor.line_numbers);
            let model = ProjectEditorModel::new(
                document_id.clone(),
                CodeEditorState::new(&loaded.canonical_path, config, loaded.text),
                loaded.fingerprint,
            );
            let appearance = EditorAppearance::from(&self.app_settings.editor);
            let document = cx.new(|document_cx| {
                ProjectEditorDocument::new(model, appearance, window, document_cx)
                    .with_breadcrumb_header(breadcrumb_header)
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
