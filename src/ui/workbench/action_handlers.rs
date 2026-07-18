use super::*;

impl WorkbenchView {
    pub(super) fn on_open_command_palette(
        &mut self,
        _: &OpenCommandPalette,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_palette(PaletteKind::Command);
        cx.notify();
    }

    pub(super) fn on_open_project_palette(
        &mut self,
        _: &OpenProjectPalette,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_palette(PaletteKind::Project);
        cx.notify();
    }

    pub(super) fn on_opened_project_palette(
        &mut self,
        _: &OpenOpenedProjectPalette,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dispatch_command_action(CommandId::ProjectOpenedPalette, cx);
    }

    pub(super) fn on_project_panel_toggle(
        &mut self,
        _: &ProjectPanelToggle,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dispatch_command_action(CommandId::ProjectPanelToggle, cx);
    }

    pub(super) fn on_project_panel_refresh(
        &mut self,
        _: &ProjectPanelRefresh,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dispatch_command_action(CommandId::ProjectPanelRefresh, cx);
    }

    pub(super) fn on_create_project(
        &mut self,
        _: &CreateProject,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.request_create_project();
        self.handle_pending_create_project_request(cx);
        cx.notify();
    }

    pub(super) fn on_open_project(
        &mut self,
        _: &OpenProject,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.request_open_project();
        self.handle_pending_open_project_request(cx);
        cx.notify();
    }

    pub(super) fn on_open_ssh_project(
        &mut self,
        _: &OpenSshProject,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_ssh_project_picker();
        cx.notify();
    }

    pub(super) fn prompt_for_new_project_directory(&mut self, cx: &mut Context<Self>) {
        let parent_directory = self
            .workspace
            .selected_project_id()
            .and_then(|project_id| self.workspace.project(project_id))
            .and_then(|project| project.location.local_path())
            .and_then(|path| path.parent())
            .map(Path::to_path_buf)
            .unwrap_or_else(default_new_project_parent_directory);
        let picked_path = cx.prompt_for_new_path(&parent_directory, None);

        cx.spawn(async move |this, cx| match picked_path.await {
            Ok(Ok(Some(project_path))) => {
                let path_to_create = project_path.clone();
                let create_task = cx
                    .background_executor()
                    .spawn(async move { std::fs::create_dir(path_to_create) });
                let result = create_task.await;
                let _ = this.update(cx, |this, cx| {
                    match result {
                        Ok(()) => {
                            let _ = this.open_project_path(project_path);
                        }
                        Err(error) => {
                            this.load_error = Some(format!(
                                "{} '{}': {error}",
                                this.ui_text.get(UiTextKey::CreateProjectDirectoryError),
                                project_path.display(),
                            ));
                        }
                    }
                    cx.notify();
                });
            }
            Ok(Ok(None)) => {}
            Ok(Err(error)) => {
                let _ = this.update(cx, |this, cx| {
                    this.load_error = Some(format!(
                        "{}: {error}",
                        this.ui_text.get(UiTextKey::CreateProjectPickerError),
                    ));
                    cx.notify();
                });
            }
            Err(error) => {
                let _ = this.update(cx, |this, cx| {
                    this.load_error = Some(format!(
                        "{}: {error}",
                        this.ui_text.get(UiTextKey::CreateProjectPickerError),
                    ));
                    cx.notify();
                });
            }
        })
        .detach();
    }

    pub(super) fn prompt_for_project_directory(&mut self, cx: &mut Context<Self>) {
        let picked_paths = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Open Directory".into()),
        });

        cx.spawn(async move |this, cx| match picked_paths.await {
            Ok(Ok(Some(paths))) => {
                if let Some(project_path) = paths.into_iter().next() {
                    let _ = this.update(cx, |this, cx| {
                        let _ = this.open_project_path(project_path);
                        cx.notify();
                    });
                }
            }
            Ok(Ok(None)) => {}
            Ok(Err(error)) => {
                let _ = this.update(cx, |this, cx| {
                    this.load_error = Some(format!("Failed to open directory picker: {error}"));
                    cx.notify();
                });
            }
            Err(error) => {
                let _ = this.update(cx, |this, cx| {
                    this.load_error = Some(format!("Directory picker was interrupted: {error}"));
                    cx.notify();
                });
            }
        })
        .detach();
    }

    pub(super) fn on_open_tab_palette(
        &mut self,
        _: &OpenTabPalette,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_palette(PaletteKind::Tab);
        cx.notify();
    }

    pub(super) fn on_open_pane_palette(
        &mut self,
        _: &OpenPanePalette,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_palette(PaletteKind::Pane);
        cx.notify();
    }

    pub(super) fn on_palette_select_next(
        &mut self,
        _: &PaletteSelectNext,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.select_next_palette_item() {
            cx.notify();
        } else {
            cx.propagate();
        }
    }

    pub(super) fn on_palette_select_prev(
        &mut self,
        _: &PaletteSelectPrev,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.select_prev_palette_item() {
            cx.notify();
        } else {
            cx.propagate();
        }
    }

    pub(super) fn on_palette_confirm(
        &mut self,
        _: &PaletteConfirm,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.zed_theme_import_dialog_is_open() {
            return;
        }

        if self.overlays.pending_tab_rename.is_some() {
            let _ = self.confirm_tab_rename_dialog_from_input(cx);
            cx.notify();
            return;
        }

        if self.overlays.pending_close_project_id.is_some() {
            let _ = self.confirm_pending_project_close();
            cx.notify();
            return;
        }

        if self.palette.active_palette.is_some() {
            let _ = self.confirm_palette_selection_with_context(cx);
            self.handle_pending_create_project_request(cx);
            self.handle_pending_open_project_request(cx);
            self.flush_pending_status_notifications(window, cx);
            cx.notify();
        } else {
            cx.propagate();
        }
    }

    pub(super) fn on_palette_cancel(
        &mut self,
        _: &PaletteCancel,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.ssh.pending_host_keys.is_empty() {
            self.answer_ssh_host_key(false, false);
            cx.notify();
            return;
        }

        if self.ssh.project_picker.open {
            self.close_ssh_project_picker(cx);
            cx.notify();
            return;
        }

        if self.ssh.manager_open {
            self.close_ssh_connection_manager();
            cx.notify();
            return;
        }

        if self.zed_theme_import_dialog_is_open() {
            self.cancel_zed_theme_import_dialog();
            cx.notify();
            return;
        }

        if self.overlays.git_diff_panel.is_some() {
            self.close_git_diff_panel();
            cx.notify();
            return;
        }
        if self.overlays.layout_toml_editor.is_some() {
            self.cancel_layout_toml_editor();
            cx.notify();
            return;
        }

        if self.overlays.pending_tab_rename.is_some() {
            self.cancel_tab_rename_dialog();
            cx.notify();
            return;
        }

        if self.overlays.pending_close_project_id.is_some() {
            self.cancel_pending_project_close();
            cx.notify();
            return;
        }

        if self.palette.active_palette.is_some() {
            self.close_palette();
            cx.notify();
        } else if self.settings.settings_page.is_open {
            self.close_settings();
            cx.notify();
        } else {
            cx.propagate();
        }
    }

    pub(super) fn on_tab_new(&mut self, _: &TabNew, _window: &mut Window, cx: &mut Context<Self>) {
        self.dispatch_command_action(CommandId::TabNew, cx);
    }

    pub(super) fn on_project_close(
        &mut self,
        _: &ProjectClose,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dispatch_command_action(CommandId::ProjectClose, cx);
    }

    pub(super) fn on_tab_close(
        &mut self,
        _: &TabClose,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dispatch_command_action(CommandId::TabClose, cx);
    }

    pub(super) fn on_tab_close_all(
        &mut self,
        _: &TabCloseAll,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dispatch_tab_close_scope(WorkbenchTabCloseScope::All, cx);
    }

    pub(super) fn on_tab_close_before(
        &mut self,
        _: &TabCloseBefore,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dispatch_tab_close_scope(WorkbenchTabCloseScope::Before, cx);
    }

    pub(super) fn on_tab_close_after(
        &mut self,
        _: &TabCloseAfter,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dispatch_tab_close_scope(WorkbenchTabCloseScope::After, cx);
    }

    pub(super) fn on_tab_close_all_files(
        &mut self,
        _: &TabCloseAllFiles,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dispatch_tab_close_scope(WorkbenchTabCloseScope::Files, cx);
    }

    pub(super) fn on_tab_close_all_terminals(
        &mut self,
        _: &TabCloseAllTerminals,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dispatch_tab_close_scope(WorkbenchTabCloseScope::Terminals, cx);
    }

    fn dispatch_tab_close_scope(&mut self, scope: WorkbenchTabCloseScope, cx: &mut Context<Self>) {
        if let Some(anchor) = self.active_work_item()
            && let Err(error) = self.close_work_item_tabs(&anchor, scope, cx)
        {
            self.load_error = Some(error.to_string());
        }
        cx.notify();
    }

    pub(super) fn on_tab_rename(
        &mut self,
        _: &TabRename,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dispatch_command_action(CommandId::TabRename, cx);
    }

    pub(super) fn on_tab_next(
        &mut self,
        _: &TabNext,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dispatch_command_action(CommandId::TabNext, cx);
    }

    pub(super) fn on_tab_prev(
        &mut self,
        _: &TabPrev,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dispatch_command_action(CommandId::TabPrev, cx);
    }

    pub(super) fn on_pane_split_vertical(
        &mut self,
        _: &PaneSplitVertical,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dispatch_command_action(CommandId::PaneSplitVertical, cx);
    }

    pub(super) fn on_pane_split_horizontal(
        &mut self,
        _: &PaneSplitHorizontal,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dispatch_command_action(CommandId::PaneSplitHorizontal, cx);
    }

    pub(super) fn on_pane_close(
        &mut self,
        _: &PaneClose,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dispatch_command_action(CommandId::PaneClose, cx);
    }

    pub(super) fn on_pane_rename(
        &mut self,
        _: &PaneRename,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dispatch_command_action(CommandId::PaneRename, cx);
    }

    pub(super) fn on_pane_focus_left(
        &mut self,
        _: &PaneFocusLeft,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dispatch_command_action(CommandId::PaneFocusLeft, cx);
    }

    pub(super) fn on_pane_focus_right(
        &mut self,
        _: &PaneFocusRight,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dispatch_command_action(CommandId::PaneFocusRight, cx);
    }

    pub(super) fn on_pane_focus_up(
        &mut self,
        _: &PaneFocusUp,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dispatch_command_action(CommandId::PaneFocusUp, cx);
    }

    pub(super) fn on_pane_focus_down(
        &mut self,
        _: &PaneFocusDown,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dispatch_command_action(CommandId::PaneFocusDown, cx);
    }

    pub(super) fn on_pane_resize_left(
        &mut self,
        _: &PaneResizeLeft,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dispatch_command_action(CommandId::PaneResizeLeft, cx);
    }

    pub(super) fn on_pane_resize_right(
        &mut self,
        _: &PaneResizeRight,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dispatch_command_action(CommandId::PaneResizeRight, cx);
    }

    pub(super) fn on_pane_resize_up(
        &mut self,
        _: &PaneResizeUp,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dispatch_command_action(CommandId::PaneResizeUp, cx);
    }

    pub(super) fn on_pane_resize_down(
        &mut self,
        _: &PaneResizeDown,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dispatch_command_action(CommandId::PaneResizeDown, cx);
    }

    pub(super) fn on_layout_save_current(
        &mut self,
        _: &LayoutSaveCurrent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dispatch_command_action(CommandId::LayoutSaveCurrent, cx);
        self.flush_pending_status_notifications(window, cx);
    }

    pub(super) fn on_layout_default_edit(
        &mut self,
        _: &LayoutDefaultEdit,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dispatch_command_action(CommandId::LayoutDefaultEdit, cx);
    }

    pub(super) fn on_layout_default_reset(
        &mut self,
        _: &LayoutDefaultReset,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dispatch_command_action(CommandId::LayoutDefaultReset, cx);
        self.flush_pending_status_notifications(window, cx);
    }

    pub(super) fn on_layout_default_reload(
        &mut self,
        _: &LayoutDefaultReload,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dispatch_command_action(CommandId::LayoutDefaultReload, cx);
        self.flush_pending_status_notifications(window, cx);
    }

    pub(super) fn on_layout_project_edit(
        &mut self,
        _: &LayoutProjectEdit,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dispatch_command_action(CommandId::LayoutProjectEdit, cx);
    }

    pub(super) fn on_layout_reset_local_override(
        &mut self,
        _: &LayoutResetLocalOverride,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dispatch_command_action(CommandId::LayoutResetLocalOverride, cx);
        self.flush_pending_status_notifications(window, cx);
    }

    pub(super) fn on_layout_export_project_config(
        &mut self,
        _: &LayoutExportProjectConfig,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dispatch_command_action(CommandId::LayoutExportProjectConfig, cx);
        self.flush_pending_status_notifications(window, cx);
    }

    pub(super) fn on_layout_open_file(
        &mut self,
        _: &LayoutOpenFile,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dispatch_command_action(CommandId::LayoutOpenFile, cx);
        self.flush_pending_status_notifications(window, cx);
    }

    pub(super) fn on_settings_keybindings(
        &mut self,
        _: &SettingsKeybindings,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dispatch_command_action(CommandId::SettingsKeybindings, cx);
        self.flush_pending_status_notifications(window, cx);
    }

    pub(super) fn on_file_save(
        &mut self,
        _: &FileSave,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.save_active_document(window, cx);
        cx.notify();
    }

    pub(super) fn on_git_branch_switch(
        &mut self,
        _: &GitBranchSwitch,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dispatch_command_action(CommandId::GitBranchSwitch, cx);
    }

    pub(super) fn on_git_diff_open(
        &mut self,
        _: &GitDiffOpen,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dispatch_command_action(CommandId::GitDiffOpen, cx);
    }

    pub(super) fn on_settings_open(
        &mut self,
        _: &SettingsOpen,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dispatch_command_action(CommandId::SettingsOpen, cx);
    }

    pub(super) fn on_settings_notifications(
        &mut self,
        _: &SettingsNotifications,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dispatch_command_action(CommandId::SettingsNotifications, cx);
        self.flush_pending_status_notifications(window, cx);
    }

    pub(super) fn on_project_editor_document_event(
        &mut self,
        document: &Entity<ProjectEditorDocument>,
        event: &ProjectEditorDocumentEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let document_id = document.read(cx).model().document_id().clone();
        match event {
            ProjectEditorDocumentEvent::Changed { generation } => {
                self.schedule_delayed_autosave(document_id, *generation, window, cx);
            }
            ProjectEditorDocumentEvent::Focused => {
                let _ = self.select_work_item(WorkItemId::File(document_id.clone()));
                self.check_document_for_external_changes(document_id, window, cx);
            }
            ProjectEditorDocumentEvent::Blurred => {
                self.queue_focus_change_autosave(document_id);
            }
            ProjectEditorDocumentEvent::Error { message } => {
                self.load_error = Some(message.clone());
            }
        }
        cx.notify();
    }

    pub(super) fn dispatch_command_action(
        &mut self,
        command_id: CommandId,
        cx: &mut Context<Self>,
    ) {
        if self.palette.active_palette.is_some()
            || self.overlays.pending_tab_rename.is_some()
            || self.overlays.pending_keybinding_edit.is_some()
            || self.overlays.layout_toml_editor.is_some()
            || self.overlays.git_diff_panel.is_some()
            || self.zed_theme_import_dialog_is_open()
        {
            cx.propagate();
            return;
        }

        let _ = self.run_command(command_id);
        cx.notify();
    }

    pub(super) fn on_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.keystroke.key == "escape" && self.work_area_drop_target.take().is_some() {
            cx.notify();
        }

        if self.overlays.pending_keybinding_edit.is_some() {
            let recorded = self.record_keybinding_edit_keystroke(&event.keystroke);
            cx.stop_propagation();
            if recorded {
                cx.notify();
            }
            return;
        }

        if self.overlays.git_diff_panel.is_some() {
            if self.handle_git_diff_key_down(event, cx) {
                cx.stop_propagation();
                cx.notify();
            } else {
                cx.propagate();
            }
            return;
        }

        if self.overlays.layout_toml_editor.is_some() {
            cx.propagate();
            return;
        }

        if self.palette.active_palette.is_none() {
            if let Some(command_id) = Self::workspace_arrow_keydown_command_for_owner(
                self.foreground_input_owner_kind(),
                &event.keystroke.key,
                event.keystroke.modifiers.platform,
                event.keystroke.modifiers.control,
                event.keystroke.modifiers.alt,
                event.keystroke.modifiers.shift,
            ) {
                let _ = self.run_command(command_id);
                cx.stop_propagation();
                cx.notify();
                return;
            }

            cx.propagate();
            return;
        }

        if !self.should_use_palette_text_fallback(self.palette_input_contains_focus(window, cx)) {
            cx.propagate();
            return;
        }

        if event.keystroke.key == "backspace" {
            self.pop_palette_query();
            cx.stop_propagation();
            cx.notify();
            return;
        }

        let has_command_modifier = event.keystroke.modifiers.control
            || event.keystroke.modifiers.alt
            || event.keystroke.modifiers.platform
            || event.keystroke.modifiers.function;
        if has_command_modifier {
            cx.propagate();
            return;
        }

        let Some(key_char) = event.keystroke.key_char.as_deref() else {
            cx.propagate();
            return;
        };
        let mut chars = key_char.chars();
        let Some(value) = chars.next() else {
            cx.propagate();
            return;
        };
        if chars.next().is_none() && !value.is_control() && self.append_palette_query(value) {
            cx.stop_propagation();
            cx.notify();
        } else {
            cx.propagate();
        }
    }
}

fn default_new_project_parent_directory() -> PathBuf {
    ["HOME", "USERPROFILE"]
        .into_iter()
        .find_map(std::env::var_os)
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."))
}
