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
        if self.pending_tab_rename.is_some() {
            let _ = self.confirm_tab_rename_dialog_from_input(cx);
            cx.notify();
            return;
        }

        if self.pending_close_project_id.is_some() {
            let _ = self.confirm_pending_project_close();
            cx.notify();
            return;
        }

        if self.active_palette.is_some() {
            let _ = self.confirm_palette_selection();
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
        if self.layout_toml_editor.is_some() {
            self.cancel_layout_toml_editor();
            cx.notify();
            return;
        }

        if self.pending_tab_rename.is_some() {
            self.cancel_tab_rename_dialog();
            cx.notify();
            return;
        }

        if self.pending_close_project_id.is_some() {
            self.cancel_pending_project_close();
            cx.notify();
            return;
        }

        if self.active_palette.is_some() {
            self.close_palette();
            cx.notify();
        } else if self.settings_page.is_open {
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
        }
        cx.notify();
    }

    pub(super) fn dispatch_command_action(
        &mut self,
        command_id: CommandId,
        cx: &mut Context<Self>,
    ) {
        if self.active_palette.is_some()
            || self.pending_tab_rename.is_some()
            || self.pending_keybinding_edit.is_some()
            || self.layout_toml_editor.is_some()
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
        if self.layout_toml_editor.is_some() {
            cx.propagate();
            return;
        }

        if self.active_palette.is_none() {
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
