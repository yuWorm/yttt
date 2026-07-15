use super::*;

impl WorkbenchView {
    pub fn active_palette(&self) -> Option<&ActivePalette> {
        self.palette.active_palette.as_ref()
    }

    pub fn open_palette(&mut self, kind: PaletteKind) {
        self.palette.active_palette = Some(ActivePalette::new(kind));
        self.reset_palette_input();
        self.palette.scroll_handle = ScrollHandle::new();
        self.palette.input_needs_focus = true;
        self.sync_input_owner_state();
    }

    pub fn new_tab_from_toolbar(&mut self) -> Result<(), WorkbenchError> {
        if self.app_settings.general.new_tab_command_picker_enabled {
            self.open_palette(PaletteKind::NewTabCommand);
            Ok(())
        } else {
            self.run_command(CommandId::TabNew)
        }
    }

    pub fn close_palette(&mut self) {
        self.palette.active_palette = None;
        self.reset_palette_input();
        self.sync_input_owner_state();
    }

    pub fn set_palette_query(&mut self, query: impl Into<String>) {
        if let Some(active_palette) = &mut self.palette.active_palette {
            active_palette.query = query.into();
            active_palette.selected_index = 0;
            self.reset_palette_input();
        }
    }

    pub fn sync_palette_query_from_input_value(&mut self, query: impl Into<String>) -> bool {
        let Some(active_palette) = &mut self.palette.active_palette else {
            return false;
        };

        let query = query.into();
        if active_palette.query != query {
            active_palette.query = query;
            active_palette.selected_index = 0;
        }
        true
    }

    pub fn confirm_palette_selection(&mut self) -> Result<(), WorkbenchError> {
        let Some(active_palette) = self.palette.active_palette.clone() else {
            return Ok(());
        };
        let items = self.palette_items(active_palette.kind);
        let Some(item) = active_palette.selected_item(&items).cloned() else {
            return Ok(());
        };

        if !item.enabled {
            let reason = item
                .disabled_reason
                .as_deref()
                .unwrap_or(self.ui_text.get(UiTextKey::CommandUnavailable));
            self.load_error = Some(format!("Command unavailable: {reason}"));
            return Ok(());
        }

        match active_palette.kind {
            PaletteKind::Command => {
                let opens_palette = opens_palette_command(item.command);
                self.run_command(item.command)?;
                if opens_palette {
                    return Ok(());
                }
            }
            PaletteKind::NewTabCommand => {
                let tab_id = self.workspace.create_shell_tab_with_command(item.id)?;
                self.select_work_item(WorkItemId::Terminal(tab_id))?;
            }
            PaletteKind::Project | PaletteKind::OpenedProject | PaletteKind::RecentProject => {
                let project_id = self
                    .workspace
                    .opened_projects()
                    .iter()
                    .find(|project| project.id.as_str() == item.id)
                    .map(|project| project.id.clone());
                if let Some(project_id) = project_id {
                    self.select_project(&project_id)?;
                } else if item.command == CommandId::ProjectOpenRecent {
                    self.open_project_path(PathBuf::from(&item.id))?;
                }
            }
            PaletteKind::Tab => {
                let project_id = self
                    .workspace
                    .selected_project_id()
                    .cloned()
                    .ok_or(WorkspaceError::NoSelectedProject)?;
                if let Some(work_item) = decode_tab_palette_item_id(&item.id, &project_id) {
                    self.select_work_item(work_item)?;
                }
            }
            PaletteKind::Pane => {
                self.focus_visible_terminal_pane(&item.id)?;
            }
            PaletteKind::GitBranch => {
                self.queue_git_branch_switch(&item.id);
                return Ok(());
            }
        }

        self.close_palette();
        Ok(())
    }

    pub fn active_palette_items(&self) -> Vec<PaletteItem> {
        let Some(active_palette) = &self.palette.active_palette else {
            return Vec::new();
        };

        self.palette_items(active_palette.kind)
    }

    pub fn visible_palette_titles(&self) -> Vec<String> {
        let Some(active_palette) = &self.palette.active_palette else {
            return Vec::new();
        };
        let items = self.palette_items(active_palette.kind);

        active_palette
            .filtered_items(&items)
            .into_iter()
            .map(|item| item.title.clone())
            .collect()
    }

    pub(super) fn palette_items(&self, kind: PaletteKind) -> Vec<PaletteItem> {
        match kind {
            PaletteKind::Command => self.command_palette_items(),
            PaletteKind::NewTabCommand => {
                new_tab_command_palette_items(&self.app_settings.general.new_tab_commands)
            }
            PaletteKind::Project => project_palette_items_with_text(
                &self.workspace,
                &self.palette.recent_projects,
                &self.ui_text,
            ),
            PaletteKind::OpenedProject => {
                opened_project_palette_items_with_text(&self.workspace, &self.ui_text)
            }
            PaletteKind::RecentProject => recent_project_palette_items_with_text(
                &self.workspace,
                &self.palette.recent_projects,
                &self.ui_text,
            ),
            PaletteKind::Tab => self.selected_work_item_palette_items(),
            PaletteKind::Pane => {
                if matches!(self.active_work_item(), Some(WorkItemId::File(_))) {
                    Vec::new()
                } else {
                    pane_palette_items_with_text(&self.workspace, &self.ui_text).unwrap_or_default()
                }
            }
            PaletteKind::GitBranch => self.git_branch_palette_items(),
        }
    }

    pub(super) fn command_palette_items(&self) -> Vec<PaletteItem> {
        let mut items = command_palette_items_with_text(
            &self.command_registry,
            CommandPaletteContext::from_command_context(self.command_context()),
            &self.ui_text,
        );

        for item in &mut items {
            item.keybinding = self.display_keybinding_for_command(item.command);
        }

        items
    }

    pub(super) fn selected_work_item_palette_items(&self) -> Vec<PaletteItem> {
        let Some(project_id) = self.workspace.selected_project_id() else {
            return Vec::new();
        };
        let Some(project) = self.workspace.project(project_id) else {
            return Vec::new();
        };
        let mut snapshots = tab_palette_items_with_text(&self.workspace, &self.ui_text)
            .unwrap_or_default()
            .into_iter()
            .map(|item| {
                TabPaletteSnapshot::terminal(item.id, item.title, item.subtitle, item.status)
            })
            .collect::<Vec<_>>();
        let active = self.active_work_item();
        if let Some(session) = self
            .project
            .project_editor_runtime
            .workspace()
            .session(project_id)
        {
            snapshots.extend(session.file_ids().iter().cloned().map(|document_id| {
                let relative_path = document_id
                    .canonical_path
                    .strip_prefix(&project.path)
                    .unwrap_or(&document_id.canonical_path)
                    .to_path_buf();
                let status = (active.as_ref() == Some(&WorkItemId::File(document_id.clone())))
                    .then(|| self.ui_text.get(UiTextKey::PaletteStatusActive).to_string());
                TabPaletteSnapshot::file(document_id, relative_path, status)
            }));
        }
        unified_tab_palette_items(&snapshots)
    }

    pub(super) fn display_keybinding_for_command(&self, command: CommandId) -> Option<String> {
        primary_display_keybinding_for_current_platform(
            &self.settings.keybindings_editor.command_keys(command),
        )
    }

    pub(super) fn select_next_palette_item(&mut self) -> bool {
        let Some(kind) = self
            .palette
            .active_palette
            .as_ref()
            .map(|palette| palette.kind)
        else {
            return false;
        };
        let items = self.palette_items(kind);
        let Some(active_palette) = &mut self.palette.active_palette else {
            return false;
        };

        active_palette.select_next(&items);
        true
    }

    pub(super) fn select_prev_palette_item(&mut self) -> bool {
        let Some(kind) = self
            .palette
            .active_palette
            .as_ref()
            .map(|palette| palette.kind)
        else {
            return false;
        };
        let items = self.palette_items(kind);
        let Some(active_palette) = &mut self.palette.active_palette else {
            return false;
        };

        active_palette.select_prev(&items);
        true
    }

    pub(super) fn append_palette_query(&mut self, value: char) -> bool {
        let Some(active_palette) = &mut self.palette.active_palette else {
            return false;
        };

        active_palette.query.push(value);
        active_palette.selected_index = 0;
        true
    }

    pub(super) fn pop_palette_query(&mut self) -> bool {
        let Some(active_palette) = &mut self.palette.active_palette else {
            return false;
        };

        active_palette.query.pop();
        active_palette.selected_index = 0;
        true
    }

    pub(super) fn reset_palette_input(&mut self) {
        self.palette.input = None;
        self.palette.input_subscription = None;
        self.palette.input_needs_focus = false;
    }

    pub(super) fn palette_input_contains_focus(&self, window: &Window, cx: &Context<Self>) -> bool {
        self.palette
            .input
            .as_ref()
            .is_some_and(|input| input.read(cx).focus_handle(cx).contains_focused(window, cx))
    }

    pub(super) fn palette_query_input(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Entity<InputState>> {
        let active_palette = self.palette.active_palette.as_ref()?;
        let input = if let Some(input) = &self.palette.input {
            input.clone()
        } else {
            let placeholder = palette_input_placeholder(active_palette.kind, &self.ui_text);
            let query = active_palette.query.clone();
            let input = cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(placeholder)
                    .default_value(query)
            });
            let subscription = cx.subscribe_in(&input, window, Self::on_palette_input_event);
            self.palette.input = Some(input.clone());
            self.palette.input_subscription = Some(subscription);
            input
        };

        if self.palette.input_needs_focus {
            input.update(cx, |input, cx| input.focus(window, cx));
            self.palette.input_needs_focus = false;
        }

        Some(input)
    }

    pub(super) fn on_palette_input_event(
        &mut self,
        input: &Entity<InputState>,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            InputEvent::Change => {
                let query = input.read(cx).value().to_string();
                if self.sync_palette_query_from_input_value(query) {
                    cx.notify();
                }
            }
            InputEvent::PressEnter { .. } => {
                let _ = self.confirm_palette_selection();
                self.handle_pending_open_project_request(cx);
                self.flush_pending_status_notifications(window, cx);
                cx.notify();
            }
            InputEvent::Focus | InputEvent::Blur => {}
        }
    }
}
