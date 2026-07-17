use super::*;

impl WorkbenchView {
    pub(super) fn flush_pending_document_saves(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let pending = std::mem::take(&mut self.documents.pending_document_saves);
        for document_id in pending {
            self.save_document(document_id, false, SaveContinuation::None, window, cx);
        }
    }

    pub(super) fn flush_pending_focus_change_autosaves(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let pending = std::mem::take(&mut self.documents.pending_focus_change_autosaves);
        for document_id in pending {
            self.autosave_document(document_id, None, window, cx);
        }
    }

    pub(super) fn flush_pending_file_close_requests(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let pending = std::mem::take(&mut self.documents.pending_file_close_requests);
        for document_id in pending {
            let is_dirty = self
                .project
                .project_editor_runtime
                .document(&document_id)
                .is_some_and(|document| document.read(cx).model().is_dirty());
            if is_dirty {
                if self.documents.pending_dirty_close.is_none() {
                    self.documents.pending_dirty_close = Some(PendingDirtyClose {
                        intent: DirtyCloseIntent::File(document_id.clone()),
                        dirty_documents: vec![document_id],
                        running_pane_count: 0,
                        saving_documents: HashSet::new(),
                    });
                    self.sync_input_owner_state();
                }
            } else if let Err(error) = self.close_file_work_item_immediately(&document_id) {
                self.load_error = Some(error.to_string());
            }
        }
    }

    pub(super) fn flush_pending_project_close_requests(&mut self, cx: &mut Context<Self>) {
        let pending = std::mem::take(&mut self.documents.pending_project_close_requests);
        for project_id in pending {
            if self.workspace.project(&project_id).is_none() {
                continue;
            }
            let dirty_documents = self
                .project
                .project_editor_runtime
                .documents_for_project(&project_id)
                .filter(|(_, document)| document.read(cx).model().is_dirty())
                .map(|(document_id, _)| document_id.clone())
                .collect::<Vec<_>>();
            let running_pane_count = self.project_running_pane_count(&project_id);
            if !dirty_documents.is_empty() || running_pane_count > 0 {
                if self.documents.pending_dirty_close.is_none() {
                    self.overlays.pending_close_project_id = None;
                    self.documents.pending_dirty_close = Some(PendingDirtyClose {
                        intent: DirtyCloseIntent::Project(project_id),
                        dirty_documents,
                        running_pane_count,
                        saving_documents: HashSet::new(),
                    });
                }
            } else {
                match self.workspace.request_close_project(&project_id) {
                    Ok(CloseProjectDecision::Closed(closed)) => {
                        self.cleanup_closed_project(&closed.project_id)
                    }
                    Ok(CloseProjectDecision::NeedsConfirmation { project_id, .. }) => {
                        self.overlays.pending_close_project_id = Some(project_id);
                    }
                    Err(error) => self.load_error = Some(error.to_string()),
                }
            }
        }
        self.sync_input_owner_state();
    }

    pub fn has_pending_dirty_close(&self) -> bool {
        self.documents.pending_dirty_close.is_some()
    }

    pub fn request_window_close(&mut self, cx: &mut Context<Self>) -> bool {
        if std::mem::take(&mut self.documents.allow_window_close_once) {
            return true;
        }
        if self.documents.pending_dirty_close.is_some() {
            return false;
        }
        let project_ids = self
            .workspace
            .opened_projects()
            .iter()
            .map(|project| project.id.clone())
            .collect::<Vec<_>>();
        let dirty_documents = project_ids
            .iter()
            .flat_map(|project_id| {
                self.project
                    .project_editor_runtime
                    .documents_for_project(project_id)
                    .filter(|(_, document)| document.read(cx).model().is_dirty())
                    .map(|(document_id, _)| document_id.clone())
            })
            .collect::<Vec<_>>();
        let running_pane_count = project_ids
            .iter()
            .map(|project_id| self.project_running_pane_count(project_id))
            .sum();
        if dirty_documents.is_empty() && running_pane_count == 0 {
            return true;
        }
        self.overlays.pending_close_project_id = None;
        self.documents.pending_dirty_close = Some(PendingDirtyClose {
            intent: DirtyCloseIntent::Window,
            dirty_documents,
            running_pane_count,
            saving_documents: HashSet::new(),
        });
        self.sync_input_owner_state();
        cx.notify();
        false
    }

    pub fn visible_dirty_close_actions(&self) -> Vec<String> {
        let Some(pending) = self.documents.pending_dirty_close.as_ref() else {
            return Vec::new();
        };
        let save = if matches!(pending.intent, DirtyCloseIntent::File(_)) {
            UiTextKey::FileSaveAction
        } else {
            UiTextKey::SaveAllAndContinue
        };
        let discard = if matches!(pending.intent, DirtyCloseIntent::File(_)) {
            UiTextKey::Discard
        } else {
            UiTextKey::DiscardAndContinue
        };
        vec![
            self.ui_text.get(UiTextKey::Cancel).to_string(),
            self.ui_text.get(discard).to_string(),
            self.ui_text.get(save).to_string(),
        ]
    }

    pub fn visible_dirty_close_dialog_text(&self) -> Option<String> {
        let pending = self.documents.pending_dirty_close.as_ref()?;
        let title = match &pending.intent {
            DirtyCloseIntent::File(_) => self.ui_text.get(UiTextKey::UnsavedChangesTitle),
            DirtyCloseIntent::WorkItems { .. } => self.ui_text.get(UiTextKey::UnsavedChangesTitle),
            DirtyCloseIntent::Project(_) => self.ui_text.get(UiTextKey::CloseProjectTitle),
            DirtyCloseIntent::Window => self.ui_text.get(UiTextKey::CloseWindowTitle),
        };
        let mut lines = vec![title.to_string()];
        if !pending.dirty_documents.is_empty() {
            let count = self.localized_close_count(
                pending.dirty_documents.len(),
                UiTextKey::UnsavedFileSingular,
                UiTextKey::UnsavedFilePlural,
            );
            let file_names = pending
                .dirty_documents
                .iter()
                .map(|document_id| {
                    document_id
                        .canonical_path
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_else(|| document_id.canonical_path.display().to_string())
                })
                .collect::<Vec<_>>()
                .join(", ");
            lines.push(format!("{count}: {file_names}"));
        }
        if pending.running_pane_count > 0 {
            lines.push(self.localized_close_count(
                pending.running_pane_count,
                UiTextKey::RunningProcessSingular,
                UiTextKey::RunningProcessPlural,
            ));
        }
        Some(lines.join("\n"))
    }

    pub(super) fn dirty_close_has_save_error(&self, cx: &Context<Self>) -> bool {
        self.documents
            .pending_dirty_close
            .as_ref()
            .is_some_and(|pending| {
                pending.dirty_documents.iter().any(|document_id| {
                    self.project
                        .project_editor_runtime
                        .document(document_id)
                        .is_some_and(|document| {
                            document.read(cx).model().editor().error().is_some()
                        })
                })
            })
    }

    pub(super) fn localized_close_count(
        &self,
        count: usize,
        singular: UiTextKey,
        plural: UiTextKey,
    ) -> String {
        let unit = self.ui_text.get(if count == 1 { singular } else { plural });
        if unit.starts_with('个') {
            format!("{count}{unit}")
        } else {
            format!("{count} {unit}")
        }
    }

    pub fn cancel_pending_dirty_close(&mut self) {
        self.documents.pending_dirty_close = None;
        self.sync_input_owner_state();
    }

    pub fn save_pending_dirty_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(pending) = &mut self.documents.pending_dirty_close else {
            return;
        };
        let document_ids = pending.dirty_documents.clone();
        pending.saving_documents = document_ids.iter().cloned().collect();
        if document_ids.is_empty() {
            self.finish_pending_dirty_close(window, cx);
            return;
        }
        for document_id in document_ids {
            self.save_document(
                document_id,
                false,
                SaveContinuation::CompletePendingClose,
                window,
                cx,
            );
        }
        cx.notify();
    }

    pub(super) fn finish_pending_dirty_close(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(pending) = self.documents.pending_dirty_close.take() else {
            return;
        };
        match pending.intent {
            DirtyCloseIntent::File(document_id) => {
                if let Err(error) = self.close_file_work_item_immediately(&document_id) {
                    self.load_error = Some(error.to_string());
                }
            }
            DirtyCloseIntent::WorkItems {
                terminal_ids,
                file_ids,
            } => {
                if let Err(error) = self.close_work_items_immediately(&terminal_ids, &file_ids) {
                    self.load_error = Some(error.to_string());
                }
            }
            DirtyCloseIntent::Project(project_id) => {
                match self.workspace.confirm_close_project(&project_id) {
                    Ok(closed) => self.cleanup_closed_project(&closed.project_id),
                    Err(error) => self.load_error = Some(error.to_string()),
                }
            }
            DirtyCloseIntent::Window => {
                self.documents.allow_window_close_once = true;
                window.remove_window();
            }
        }
        self.sync_input_owner_state();
        cx.notify();
    }

    pub fn discard_pending_dirty_close(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(pending) = self.documents.pending_dirty_close.take() else {
            return;
        };
        match pending.intent {
            DirtyCloseIntent::File(document_id) => {
                if let Err(error) = self.close_file_work_item_immediately(&document_id) {
                    self.load_error = Some(error.to_string());
                }
            }
            DirtyCloseIntent::WorkItems {
                terminal_ids,
                file_ids,
            } => {
                if let Err(error) = self.close_work_items_immediately(&terminal_ids, &file_ids) {
                    self.load_error = Some(error.to_string());
                }
            }
            DirtyCloseIntent::Project(project_id) => {
                match self.workspace.confirm_close_project(&project_id) {
                    Ok(closed) => self.cleanup_closed_project(&closed.project_id),
                    Err(error) => self.load_error = Some(error.to_string()),
                }
            }
            DirtyCloseIntent::Window => {
                self.documents.allow_window_close_once = true;
                _window.remove_window();
            }
        }
        self.sync_input_owner_state();
        cx.notify();
    }

    pub(super) fn queue_focus_change_autosave(
        &mut self,
        document_id: crate::ui::editor::DocumentId,
    ) {
        if self.app_settings.editor.autosave == EditorAutosave::OnFocusChange
            && !self
                .documents
                .pending_focus_change_autosaves
                .contains(&document_id)
        {
            self.documents
                .pending_focus_change_autosaves
                .push(document_id);
        }
    }

    pub(super) fn schedule_delayed_autosave(
        &mut self,
        document_id: crate::ui::editor::DocumentId,
        generation: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.app_settings.editor.autosave != EditorAutosave::AfterDelay {
            self.project
                .project_editor_runtime
                .cancel_autosave_task(&document_id);
            return;
        }
        let delay = Duration::from_millis(self.app_settings.editor.autosave_delay_ms);
        let task_document_id = document_id.clone();
        let task = cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(delay).await;
            let _ = this.update_in(cx, |root, window, cx| {
                if root.app_settings.editor.autosave == EditorAutosave::AfterDelay {
                    root.autosave_document(task_document_id, Some(generation), window, cx);
                }
            });
        });
        self.project
            .project_editor_runtime
            .replace_autosave_task(document_id, task);
    }

    pub(super) fn autosave_document(
        &mut self,
        document_id: crate::ui::editor::DocumentId,
        expected_generation: Option<u64>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .documents
            .pending_file_conflict
            .as_ref()
            .is_some_and(|conflict| conflict.document_id == document_id)
        {
            return;
        }
        let Some(document) = self
            .project
            .project_editor_runtime
            .document(&document_id)
            .cloned()
        else {
            return;
        };
        let (generation, dirty, saving) = {
            let document = document.read(cx);
            (
                document.model().generation(),
                document.model().is_dirty(),
                !matches!(document.model().save_state(), ProjectEditorSaveState::Idle),
            )
        };
        if expected_generation.is_some_and(|expected| expected != generation) || !dirty {
            return;
        }
        if saving {
            self.project
                .project_editor_runtime
                .request_follow_up_autosave(document_id, generation);
            return;
        }
        self.save_document(document_id, false, SaveContinuation::None, window, cx);
    }

    pub(super) fn run_follow_up_autosave(
        &mut self,
        document_id: &crate::ui::editor::DocumentId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(generation) = self
            .project
            .project_editor_runtime
            .take_follow_up_autosave(document_id)
        {
            self.autosave_document(document_id.clone(), Some(generation), window, cx);
        }
    }

    pub fn save_active_document(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(WorkItemId::File(document_id)) = self.active_work_item() else {
            return;
        };
        self.save_document(document_id, false, SaveContinuation::None, window, cx);
    }

    pub(super) fn save_document(
        &mut self,
        document_id: crate::ui::editor::DocumentId,
        force: bool,
        continuation: SaveContinuation,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(document) = self
            .project
            .project_editor_runtime
            .document(&document_id)
            .cloned()
        else {
            return;
        };
        let request = document.update(cx, |document, document_cx| {
            if !force && !matches!(document.model().save_state(), ProjectEditorSaveState::Idle) {
                return None;
            }
            Some(document.begin_save(document_cx))
        });
        let Some(request) = request else {
            return;
        };
        self.spawn_project_file_save_request(request, force, continuation, window, cx);
    }

    pub(super) fn project_file_services(
        &self,
        document_id: &crate::ui::editor::DocumentId,
    ) -> Option<(ProjectServices, PathBuf)> {
        let services = self.project.services.get(&document_id.project_id)?.clone();
        let relative_path = services
            .relative_path_for_document(&document_id.canonical_path)
            .ok()?;
        Some((services, relative_path))
    }

    pub(super) fn spawn_project_file_save_request(
        &mut self,
        request: SaveRequest,
        force: bool,
        continuation: SaveContinuation,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((services, relative_path)) = self.project_file_services(&request.document_id)
        else {
            let message = self
                .ui_text
                .get(UiTextKey::ProjectFileOutsideProject)
                .to_string();
            if let Some(document) = self
                .project
                .project_editor_runtime
                .document(&request.document_id)
                .cloned()
            {
                document.update(cx, |document, _| {
                    document.model_mut().fail_save(&request, message.clone());
                });
            }
            self.load_error = Some(message);
            return;
        };
        let text = request.text.clone();
        let expected_fingerprint = request.expected_fingerprint.clone();
        let io_task = cx.background_spawn(async move {
            services.save_file(&relative_path, &text, Some(&expected_fingerprint), force)
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = io_task.await;
            let _ = this.update_in(cx, |root, window, cx| {
                root.apply_project_file_save_result(request, result, continuation, window, cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn apply_project_file_save_result(
        &mut self,
        request: SaveRequest,
        result: Result<SaveProjectFileOutcome, ProjectFileIoError>,
        continuation: SaveContinuation,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(document) = self
            .project
            .project_editor_runtime
            .document(&request.document_id)
            .cloned()
        else {
            return;
        };
        let continuation = if self
            .documents
            .pending_dirty_close
            .as_ref()
            .is_some_and(|pending| pending.saving_documents.contains(&request.document_id))
        {
            SaveContinuation::CompletePendingClose
        } else {
            continuation
        };
        match result {
            Ok(SaveProjectFileOutcome::Saved(fingerprint)) => {
                let completed = document.update(cx, |document, _| {
                    document.model_mut().finish_save(&request, fingerprint)
                });
                if !completed {
                    return;
                }
                if self
                    .documents
                    .pending_file_conflict
                    .as_ref()
                    .is_some_and(|conflict| conflict.document_id == request.document_id)
                {
                    self.documents.pending_file_conflict = None;
                }
                let file_name = request
                    .document_id
                    .canonical_path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| request.document_id.canonical_path.display().to_string());
                self.queue_status_notification(
                    format!("{}: {file_name}", self.ui_text.get(UiTextKey::FileSaved)),
                    self.ui_text.get(UiTextKey::ProjectFiles),
                );
                self.load_error = None;
                if self
                    .workspace
                    .project(&request.document_id.project_id)
                    .is_some_and(|project| matches!(&project.location, ProjectLocation::Ssh { .. }))
                {
                    self.refresh_project_git_status(request.document_id.project_id.clone(), cx);
                }
                self.complete_save_continuation(continuation, &request.document_id, window, cx);
                self.flush_pending_status_notifications(window, cx);
                self.run_follow_up_autosave(&request.document_id, window, cx);
            }
            Ok(SaveProjectFileOutcome::Conflict(current_disk)) => {
                self.project
                    .project_editor_runtime
                    .take_follow_up_autosave(&request.document_id);
                let is_dirty = document.read(cx).model().is_dirty();
                if !is_dirty && matches!(current_disk, CurrentDiskState::Present(_)) {
                    document.update(cx, |document, _| {
                        document.model_mut().cancel_save(&request);
                    });
                    self.spawn_project_file_reload(request.document_id, None, window, cx);
                    return;
                }
                self.documents.pending_file_conflict = Some(PendingFileConflict {
                    document_id: request.document_id.clone(),
                    request,
                    current_disk,
                    continuation,
                });
                self.load_error = None;
                self.sync_input_owner_state();
            }
            Err(error) => {
                self.project
                    .project_editor_runtime
                    .take_follow_up_autosave(&request.document_id);
                let message = format!(
                    "{}: {}",
                    self.ui_text.get(UiTextKey::FileSaveFailed),
                    self.localized_project_file_error(&error)
                );
                document.update(cx, |document, _| {
                    document.model_mut().fail_save(&request, message.clone());
                });
                if continuation == SaveContinuation::CompletePendingClose
                    && let Some(pending) = &mut self.documents.pending_dirty_close
                {
                    pending.saving_documents.remove(&request.document_id);
                }
                self.load_error = Some(message);
            }
        }
    }

    pub(super) fn complete_save_continuation(
        &mut self,
        continuation: SaveContinuation,
        document_id: &crate::ui::editor::DocumentId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match continuation {
            SaveContinuation::None => {}
            SaveContinuation::CompletePendingClose => {
                let still_dirty = self
                    .project
                    .project_editor_runtime
                    .document(document_id)
                    .is_some_and(|document| document.read(cx).model().is_dirty());
                let should_finish = if let Some(pending) = &mut self.documents.pending_dirty_close {
                    pending.saving_documents.remove(document_id);
                    if !still_dirty {
                        pending
                            .dirty_documents
                            .retain(|pending_id| pending_id != document_id);
                    }
                    pending.dirty_documents.is_empty() && pending.saving_documents.is_empty()
                } else {
                    false
                };
                if should_finish {
                    self.finish_pending_dirty_close(window, cx);
                }
            }
        }
    }

    pub(super) fn spawn_project_file_reload(
        &mut self,
        document_id: crate::ui::editor::DocumentId,
        restore_conflict_on_error: Option<PendingFileConflict>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((services, relative_path)) = self.project_file_services(&document_id) else {
            if let Some(conflict) = restore_conflict_on_error {
                self.documents.pending_file_conflict = Some(conflict);
            }
            return;
        };
        let io_task = cx.background_spawn(async move { services.read_file(&relative_path) });
        cx.spawn_in(window, async move |this, cx| {
            let result = io_task.await;
            let _ = this.update_in(cx, |root, window, cx| {
                let document = root
                    .project
                    .project_editor_runtime
                    .document(&document_id)
                    .cloned();
                match (document, result) {
                    (Some(document), Ok(loaded)) => {
                        document.update(cx, |document, document_cx| {
                            document.replace_from_disk(
                                loaded.text,
                                loaded.fingerprint,
                                window,
                                document_cx,
                            );
                        });
                        root.documents.pending_file_conflict = None;
                        root.load_error = None;
                        root.sync_input_owner_state();
                    }
                    (Some(_document), Err(error)) => {
                        let message = root.localized_project_file_error(&error);
                        if let Some(conflict) = restore_conflict_on_error {
                            root.documents.pending_file_conflict = Some(conflict);
                        }
                        root.load_error = Some(message);
                    }
                    (None, _) => {}
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn check_project_documents_for_external_changes(
        &mut self,
        project_id: &ProjectId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let document_ids = self
            .project
            .project_editor_runtime
            .documents_for_project(project_id)
            .map(|(document_id, _)| document_id.clone())
            .collect::<Vec<_>>();
        for document_id in document_ids {
            self.check_document_for_external_changes(document_id, window, cx);
        }
    }

    pub(super) fn check_document_for_external_changes(
        &mut self,
        document_id: crate::ui::editor::DocumentId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .documents
            .pending_file_conflict
            .as_ref()
            .is_some_and(|conflict| conflict.document_id == document_id)
        {
            return;
        }
        let Some(document) = self
            .project
            .project_editor_runtime
            .document(&document_id)
            .cloned()
        else {
            return;
        };
        let (expected_fingerprint, save_is_idle) = {
            let document = document.read(cx);
            (
                document.model().disk_fingerprint().clone(),
                matches!(document.model().save_state(), ProjectEditorSaveState::Idle),
            )
        };
        if !save_is_idle {
            return;
        }
        let Some((services, relative_path)) = self.project_file_services(&document_id) else {
            return;
        };
        let io_task = cx.background_spawn(async move { services.read_file(&relative_path) });
        cx.spawn_in(window, async move |this, cx| {
            let result = io_task.await;
            let _ = this.update_in(cx, |root, window, cx| {
                if root
                    .documents
                    .pending_file_conflict
                    .as_ref()
                    .is_some_and(|conflict| conflict.document_id == document_id)
                {
                    return;
                }
                let Some(document) = root
                    .project
                    .project_editor_runtime
                    .document(&document_id)
                    .cloned()
                else {
                    return;
                };
                if document.read(cx).model().disk_fingerprint() != &expected_fingerprint
                    || !matches!(
                        document.read(cx).model().save_state(),
                        ProjectEditorSaveState::Idle
                    )
                {
                    return;
                }
                match result {
                    Ok(loaded) if loaded.fingerprint == expected_fingerprint => {}
                    Ok(loaded) if document.read(cx).model().is_dirty() => {
                        let request = document.update(cx, |document, cx| document.begin_save(cx));
                        root.documents.pending_file_conflict = Some(PendingFileConflict {
                            document_id: document_id.clone(),
                            request,
                            current_disk: CurrentDiskState::Present(loaded.fingerprint),
                            continuation: SaveContinuation::None,
                        });
                        root.load_error = None;
                        root.sync_input_owner_state();
                    }
                    Ok(loaded) => {
                        document.update(cx, |document, document_cx| {
                            document.replace_from_disk(
                                loaded.text,
                                loaded.fingerprint,
                                window,
                                document_cx,
                            );
                        });
                        root.load_error = None;
                    }
                    Err(ProjectFileIoError::Io { source, .. })
                        if source.kind() == std::io::ErrorKind::NotFound =>
                    {
                        let request = document.update(cx, |document, cx| document.begin_save(cx));
                        root.documents.pending_file_conflict = Some(PendingFileConflict {
                            document_id: document_id.clone(),
                            request,
                            current_disk: CurrentDiskState::Missing,
                            continuation: SaveContinuation::None,
                        });
                        root.load_error = None;
                        root.sync_input_owner_state();
                    }
                    Err(error) => {
                        root.load_error = Some(root.localized_project_file_error(&error));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub fn has_pending_file_conflict(&self) -> bool {
        self.documents.pending_file_conflict.is_some()
    }

    pub fn visible_file_conflict_dialog_text(&self) -> Option<String> {
        let conflict = self.documents.pending_file_conflict.as_ref()?;
        let title = if matches!(conflict.current_disk, CurrentDiskState::Missing) {
            self.ui_text.get(UiTextKey::FileDeletedOnDisk)
        } else {
            self.ui_text.get(UiTextKey::FileChangedOnDisk)
        };
        Some(format!(
            "{title}\n{}",
            conflict.document_id.canonical_path.display()
        ))
    }

    pub fn visible_file_conflict_dialog_actions(&self) -> Vec<String> {
        let Some(conflict) = self.documents.pending_file_conflict.as_ref() else {
            return Vec::new();
        };
        let mut actions = vec![self.ui_text.get(UiTextKey::Cancel).to_string()];
        if !matches!(conflict.current_disk, CurrentDiskState::Missing) {
            actions.push(self.ui_text.get(UiTextKey::FileReload).to_string());
        }
        actions.push(
            self.ui_text
                .get(
                    if matches!(conflict.current_disk, CurrentDiskState::Missing) {
                        UiTextKey::FileRecreate
                    } else {
                        UiTextKey::FileOverwrite
                    },
                )
                .to_string(),
        );
        actions
    }

    pub fn pending_document_save_count(&self) -> usize {
        self.documents.pending_document_saves.len()
    }

    pub fn pending_file_conflict_is_missing(&self) -> bool {
        self.documents
            .pending_file_conflict
            .as_ref()
            .is_some_and(|conflict| matches!(conflict.current_disk, CurrentDiskState::Missing))
    }

    pub fn overwrite_pending_file_conflict(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(conflict) = self.documents.pending_file_conflict.take() else {
            return;
        };
        self.spawn_project_file_save_request(
            conflict.request,
            true,
            conflict.continuation,
            window,
            cx,
        );
        self.sync_input_owner_state();
        cx.notify();
    }

    pub fn reload_pending_file_conflict(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(mut conflict) = self.documents.pending_file_conflict.take() else {
            return;
        };
        if conflict.continuation == SaveContinuation::CompletePendingClose {
            self.documents.pending_dirty_close = None;
            conflict.continuation = SaveContinuation::None;
        }
        let document_id = conflict.document_id.clone();
        self.spawn_project_file_reload(document_id, Some(conflict), window, cx);
        self.sync_input_owner_state();
        cx.notify();
    }

    pub fn cancel_pending_file_conflict(&mut self, cx: &mut Context<Self>) {
        let Some(conflict) = self.documents.pending_file_conflict.take() else {
            return;
        };
        if let Some(document) = self
            .project
            .project_editor_runtime
            .document(&conflict.document_id)
            .cloned()
        {
            document.update(cx, |document, _| {
                document.model_mut().cancel_save(&conflict.request);
            });
        }
        if conflict.continuation == SaveContinuation::CompletePendingClose
            && let Some(pending) = &mut self.documents.pending_dirty_close
        {
            pending.saving_documents.remove(&conflict.document_id);
        }
        self.sync_input_owner_state();
    }
}
