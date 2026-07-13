use super::*;

const ACTIVE_PROJECT_FILE_WATCH_DEBOUNCE: Duration = Duration::from_millis(150);

fn project_file_event_requires_refresh(kind: &notify::EventKind) -> bool {
    matches!(
        kind,
        notify::EventKind::Any
            | notify::EventKind::Create(_)
            | notify::EventKind::Modify(_)
            | notify::EventKind::Remove(_)
    )
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

    pub(super) fn ensure_active_project_file_watcher(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((project_id, project_path)) =
            self.workspace.selected_project_id().and_then(|project_id| {
                self.workspace
                    .project(project_id)
                    .map(|project| (project_id.clone(), project.path.clone()))
            })
        else {
            self.active_project_file_watcher = None;
            return;
        };
        if self.active_project_file_watcher_matches(&project_id, &project_path) {
            return;
        }

        self.active_project_file_watcher = None;
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        let _ = tx.try_send(());
        let mut watcher =
            match notify::recommended_watcher(move |result: notify::Result<notify::Event>| {
                if result
                    .as_ref()
                    .is_ok_and(|event| project_file_event_requires_refresh(&event.kind))
                {
                    let _ = tx.try_send(());
                }
            }) {
                Ok(watcher) => watcher,
                Err(error) => {
                    self.load_error = Some(format!(
                        "Failed to watch project files at {}: {error}",
                        project_path.display()
                    ));
                    return;
                }
            };
        use notify::Watcher as _;
        if let Err(error) = watcher.watch(&project_path, notify::RecursiveMode::Recursive) {
            self.load_error = Some(format!(
                "Failed to watch project files at {}: {error}",
                project_path.display()
            ));
            return;
        }

        let watched_project_id = project_id.clone();
        let watched_project_path = project_path.clone();
        let task = cx.spawn_in(window, async move |this, cx| {
            let _watcher = watcher;
            loop {
                cx.background_executor()
                    .timer(ACTIVE_PROJECT_FILE_WATCH_DEBOUNCE)
                    .await;
                match rx.try_recv() {
                    Ok(()) => while rx.try_recv().is_ok() {},
                    Err(std::sync::mpsc::TryRecvError::Empty) => continue,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => break,
                }

                let is_active = this
                    .update_in(cx, |root, _window, cx| {
                        if !root.active_project_file_watcher_matches(
                            &watched_project_id,
                            &watched_project_path,
                        ) {
                            return false;
                        }
                        root.queue_project_tree_refresh(watched_project_id.clone());
                        cx.notify();
                        true
                    })
                    .unwrap_or(false);
                if !is_active {
                    continue;
                }

                let status_project_path = watched_project_path.clone();
                let status_task = cx
                    .background_executor()
                    .spawn(async move { read_project_git_status(&status_project_path) });
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
            new_file: self.ui_text.get(UiTextKey::ProjectFilesNewFile).to_string(),
            new_directory: self
                .ui_text
                .get(UiTextKey::ProjectFilesNewDirectory)
                .to_string(),
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
            let interaction_text = self.project_tree_interaction_text();
            let show_hidden = self.app_settings.project_panel.show_hidden;
            if let Some(snapshot) = self.project_tree_render_snapshot(project_id) {
                tree.update(cx, |tree, tree_cx| {
                    tree.sync_with_icon_theme(snapshot, self.icon_theme.clone(), tree_cx);
                    tree.set_interaction_text(interaction_text, tree_cx);
                    tree.set_show_hidden(show_hidden, tree_cx);
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
        let tree = cx.new(|tree_cx| {
            let mut tree = ProjectTreeView::new_with_icon_theme(snapshot, icon_theme, tree_cx);
            tree.set_interaction_text(interaction_text, tree_cx);
            tree.set_show_hidden(show_hidden, tree_cx);
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
            ProjectTreeViewEvent::CreateEntry { parent, input } => {
                self.spawn_project_entry_create(
                    project_id.clone(),
                    parent.clone(),
                    input.clone(),
                    window,
                    cx,
                );
            }
            ProjectTreeViewEvent::RenameEntry { path, new_name } => {
                self.spawn_project_entry_rename(
                    project_id.clone(),
                    path.clone(),
                    new_name.clone(),
                    window,
                    cx,
                );
            }
            ProjectTreeViewEvent::RequestDelete(path) => {
                self.confirm_project_entry_delete(project_id.clone(), path.clone(), window, cx);
            }
            ProjectTreeViewEvent::CopyEntry(path) => {
                self.project.project_tree_clipboard = Some(ProjectTreeClipboard {
                    source_project_id: project_id.clone(),
                    relative_path: path.clone(),
                    mode: ProjectEntryPasteMode::Copy,
                });
            }
            ProjectTreeViewEvent::CutEntry(path) => {
                self.project.project_tree_clipboard = Some(ProjectTreeClipboard {
                    source_project_id: project_id.clone(),
                    relative_path: path.clone(),
                    mode: ProjectEntryPasteMode::Cut,
                });
            }
            ProjectTreeViewEvent::PasteEntry {
                destination_directory,
            } => {
                self.spawn_project_entry_paste(
                    project_id.clone(),
                    destination_directory.clone(),
                    window,
                    cx,
                );
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
        cx: &mut Context<Self>,
    ) {
        let Some(source_root) = self
            .workspace
            .project(source_project_id)
            .map(|project| project.path.clone())
        else {
            return;
        };
        let Some(destination_root) = self
            .workspace
            .project(destination_project_id)
            .map(|project| project.path.clone())
        else {
            return;
        };
        let Ok(canonical_source_root) = fs::canonicalize(source_root) else {
            return;
        };
        let Ok(canonical_destination_base) =
            fs::canonicalize(destination_root.join(destination_relative_path))
        else {
            return;
        };
        let source_base = canonical_source_root.join(source_relative_path);
        let migrations = self
            .project
            .project_editor_runtime
            .documents_for_project(source_project_id)
            .filter_map(|(document_id, _)| {
                let suffix = document_id.canonical_path.strip_prefix(&source_base).ok()?;
                let destination_relative = destination_relative_path.join(suffix);
                let new_document_id = crate::ui::editor::DocumentId {
                    project_id: destination_project_id.clone(),
                    canonical_path: canonical_destination_base.join(suffix),
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
                    document_cx,
                );
            });
            self.relocate_pending_document_id(&old_document_id, &new_document_id);
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
        let Some(project_root) = self
            .workspace
            .project(&project_id)
            .map(|project| project.path.clone())
        else {
            return;
        };
        let io_task = cx
            .background_spawn(async move { create_project_entry(&project_root, &parent, &input) });
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
                        root.refresh_project_tree(project_id.clone(), window, cx);
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

    fn spawn_project_entry_rename(
        &mut self,
        project_id: ProjectId,
        relative_path: PathBuf,
        new_name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(project_root) = self
            .workspace
            .project(&project_id)
            .map(|project| project.path.clone())
        else {
            return;
        };
        let moved_relative_path = relative_path.clone();
        let io_task = cx.background_spawn(async move {
            rename_project_entry(&project_root, &relative_path, &new_name)
        });
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
                        root.refresh_project_tree(project_id.clone(), window, cx);
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
        let display_path = relative_path.display().to_string();
        let workbench = cx.weak_entity();
        window.open_alert_dialog(cx, move |alert, _, _| {
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
                        .gap_2()
                        .child(display_path.clone())
                        .child(message.clone()),
                )
                .footer(
                    DialogFooter::new()
                        .child(
                            Button::new("project-entry-delete-cancel")
                                .debug_selector(|| "project-entry-delete-cancel".to_string())
                                .label(cancel_label.clone())
                                .on_click(|_, window, cx| {
                                    window.close_dialog(cx);
                                }),
                        )
                        .child(
                            Button::new("project-entry-delete-confirm")
                                .debug_selector(|| "project-entry-delete-confirm".to_string())
                                .danger()
                                .label(delete_label.clone())
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

    fn spawn_project_entry_delete(
        &mut self,
        project_id: ProjectId,
        relative_path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(project_root) = self
            .workspace
            .project(&project_id)
            .map(|project| project.path.clone())
        else {
            return;
        };
        let io_task =
            cx.background_spawn(async move { delete_project_entry(&project_root, &relative_path) });
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
                        root.refresh_project_tree(project_id.clone(), window, cx);
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
        let Some(clipboard) = self.project.project_tree_clipboard.clone() else {
            return;
        };
        let Some(source_root) = self
            .workspace
            .project(&clipboard.source_project_id)
            .map(|project| project.path.clone())
        else {
            self.project.project_tree_clipboard = None;
            return;
        };
        let Some(destination_root) = self
            .workspace
            .project(&destination_project_id)
            .map(|project| project.path.clone())
        else {
            return;
        };
        let source_relative_path = clipboard.relative_path.clone();
        let mode = clipboard.mode;
        let io_task = cx.background_spawn(async move {
            paste_project_entry(
                &source_root,
                &source_relative_path,
                &destination_root,
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
                            root.refresh_project_tree(
                                clipboard.source_project_id.clone(),
                                window,
                                cx,
                            );
                        }
                        root.refresh_project_tree(destination_project_id.clone(), window, cx);
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
