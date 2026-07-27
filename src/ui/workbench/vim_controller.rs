use std::time::Duration;

use gpui::{App, Context, Entity, Focusable as _, Keystroke, KeystrokeEvent, Window};
use gpui_component::{input::InputState, kbd::Kbd};

use crate::{
    config::settings::VimModeSetting,
    ui::{
        editor::{ProjectEditorDocument, WorkItemId},
        interaction::{
            actions::{VimEnterInsert, VimEnterNormal, VimEnterTerminal},
            input_owner::InputOwnerKind,
        },
        settings::keybinding_display::recorded_keybinding,
        terminal::pane::TerminalPaneView,
        vim::{VimCapture, VimSurface, WorkbenchVimMode},
    },
};

use super::*;

const VIM_KEY_FEEDBACK_TIMEOUT: Duration = Duration::from_millis(1_200);

impl WorkbenchView {
    pub(super) fn sync_vim_controller(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let surface = self.detect_vim_surface(window, cx);
        let capture = self.detect_vim_capture(surface, window, cx);
        self.vim.sync_surface(surface);
        self.vim.set_capture(capture);

        if surface == VimSurface::Editor
            && let Some(document) = self.active_editor_document()
        {
            let document = document.read(cx);
            if let Some(mode) = document.vim_mode() {
                self.vim.sync_editor(mode, document.vim_status());
            }
        }

        if self.vim.support() == VimModeSetting::Global && surface == VimSurface::Terminal {
            let enabled = self.vim.mode() == WorkbenchVimMode::Normal;
            if let Some(pane) = self.active_terminal_pane() {
                pane.update(cx, |pane, pane_cx| {
                    pane.set_terminal_vi_mode(enabled, pane_cx);
                });
            }
        }
    }

    pub(super) fn ensure_vim_key_feedback_observers(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.vim_keystroke_subscription.is_none() {
            self.vim_keystroke_subscription =
                Some(cx.observe_keystrokes(Self::observe_vim_keystroke));
        }
        if self.vim_pending_input_subscription.is_none() {
            self.vim_pending_input_subscription =
                Some(cx.observe_pending_input(window, Self::observe_pending_vim_input));
        }
    }

    fn observe_vim_keystroke(
        &mut self,
        event: &KeystrokeEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.record_vim_key_feedback(&event.keystroke, window, cx);
    }

    fn observe_pending_vim_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(keystrokes) = window.pending_input_keystrokes() else {
            return;
        };
        let feedback = keystrokes
            .iter()
            .filter_map(Self::format_vim_key_feedback)
            .collect::<Vec<_>>();
        let Some(generation) = self.vim.replace_key_feedback(feedback) else {
            return;
        };
        cx.notify();
        self.schedule_vim_key_feedback_expiration(generation, window, cx);
    }

    fn record_vim_key_feedback(
        &mut self,
        keystroke: &Keystroke,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.vim.accepts_key_feedback() {
            self.vim_key_feedback_task = None;
            if self.vim.clear_key_feedback() {
                cx.notify();
            }
            return;
        }
        let Some(feedback) = Self::format_vim_key_feedback(keystroke) else {
            return;
        };

        let generation = self
            .vim
            .record_key_feedback(feedback)
            .expect("command-mode Vim keystrokes accept feedback");
        cx.notify();
        self.schedule_vim_key_feedback_expiration(generation, window, cx);
    }

    fn format_vim_key_feedback(keystroke: &Keystroke) -> Option<String> {
        recorded_keybinding(keystroke)?;
        let has_command_modifier = keystroke.modifiers.control
            || keystroke.modifiers.alt
            || keystroke.modifiers.platform
            || keystroke.modifiers.function;
        if !has_command_modifier {
            let literal = keystroke
                .key_char
                .as_deref()
                .unwrap_or(keystroke.key.as_str());
            let mut characters = literal.chars();
            if let Some(character) = characters.next()
                && characters.next().is_none()
                && !character.is_control()
                && !character.is_whitespace()
            {
                return Some(
                    if keystroke.key_char.is_none() && keystroke.modifiers.shift {
                        character.to_uppercase().collect()
                    } else {
                        literal.to_string()
                    },
                );
            }
        }
        Some(Kbd::format(keystroke))
    }

    fn schedule_vim_key_feedback_expiration(
        &mut self,
        generation: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.vim_key_feedback_task = Some(cx.spawn_in(window, async move |this, cx| {
            cx.background_executor()
                .timer(VIM_KEY_FEEDBACK_TIMEOUT)
                .await;
            let _ = this.update_in(cx, |root, _window, cx| {
                if root.vim.expire_key_feedback(generation) {
                    cx.notify();
                }
            });
        }));
    }

    fn detect_vim_surface(&self, window: &Window, cx: &App) -> VimSurface {
        match self.current_input_owner_registration().kind() {
            InputOwnerKind::Palette => return VimSurface::Palette,
            InputOwnerKind::Settings => return VimSurface::Settings,
            InputOwnerKind::Dialog | InputOwnerKind::KeybindingRecorder => {
                return if self.overlays.git_diff_panel.is_some() {
                    VimSurface::GitDiff
                } else {
                    VimSurface::Dialog
                };
            }
            InputOwnerKind::ContextMenu | InputOwnerKind::Popover => return VimSurface::Dialog,
            InputOwnerKind::Workspace | InputOwnerKind::Editor => {}
        }

        if self.pending_projects_focus {
            return VimSurface::Projects;
        }
        if self.project.pending_project_tree_focus {
            return VimSurface::ProjectTree;
        }
        if self.terminal.pending_terminal_focus.is_some() {
            return VimSurface::Terminal;
        }
        if self.project.pending_editor_focus_document_id.is_some() {
            return VimSurface::Editor;
        }

        if self.projects_focus_active
            && !self.workspace.opened_projects().is_empty()
            && self
                .focus_handle
                .as_ref()
                .is_some_and(|focus_handle| focus_handle.is_focused(window))
        {
            return VimSurface::Projects;
        }
        if let Some(project_id) = self.workspace.selected_project_id()
            && let Some(tree) = self.project.project_editor_runtime.tree(project_id)
            && tree.read(cx).is_focused(window, cx)
        {
            return VimSurface::ProjectTree;
        }
        if let Some(pane) = self.active_terminal_pane()
            && pane.read(cx).terminal_is_focused(window, cx)
        {
            return VimSurface::Terminal;
        }
        if let Some(document) = self.active_editor_document()
            && document.read(cx).is_focused(window, cx)
        {
            return VimSurface::Editor;
        }

        match self.active_work_item() {
            Some(WorkItemId::Terminal(_)) => VimSurface::Terminal,
            Some(WorkItemId::File(_)) => VimSurface::Editor,
            None => VimSurface::Workspace,
        }
    }

    fn detect_vim_capture(&self, surface: VimSurface, window: &Window, cx: &App) -> VimCapture {
        match self.current_input_owner_registration().kind() {
            InputOwnerKind::KeybindingRecorder
            | InputOwnerKind::ContextMenu
            | InputOwnerKind::Popover => return VimCapture::Suspended,
            InputOwnerKind::Dialog => return VimCapture::ForceInsert,
            InputOwnerKind::Settings if self.settings_text_input_is_focused(window, cx) => {
                return VimCapture::ForceInsert;
            }
            _ => {}
        }
        if surface == VimSurface::Editor
            && let Some(document) = self.active_editor_document()
        {
            let document = document.read(cx);
            if document.vim_mode().is_none()
                || document
                    .code_input()
                    .is_some_and(|input| input.read(cx).search_panel_is_open(cx))
            {
                return VimCapture::ForceInsert;
            }
        }

        if surface == VimSurface::Terminal
            && self
                .active_terminal_pane()
                .is_some_and(|pane| pane.read(cx).terminal_search_is_active(cx))
        {
            return VimCapture::ForceInsert;
        }

        if surface == VimSurface::ProjectTree
            && self
                .workspace
                .selected_project_id()
                .and_then(|project_id| self.project.project_editor_runtime.tree(project_id))
                .is_some_and(|tree| tree.read(cx).edit_input_is_focused(window, cx))
        {
            return VimCapture::ForceInsert;
        }

        VimCapture::Inherit
    }

    fn settings_text_input_is_focused(&self, window: &Window, cx: &App) -> bool {
        fn focused(input: &Entity<InputState>, window: &Window, cx: &App) -> bool {
            input.read(cx).focus_handle(cx).is_focused(window)
        }

        self.settings
            .settings_search_input
            .as_ref()
            .into_iter()
            .chain(self.settings.settings_custom_shell_input.as_ref())
            .chain(self.settings.settings_new_tab_command_input.as_ref())
            .chain(self.settings.settings_number_inputs.values())
            .any(|input| focused(input, window, cx))
    }

    fn active_editor_document(&self) -> Option<Entity<ProjectEditorDocument>> {
        let WorkItemId::File(document_id) = self.active_work_item()? else {
            return None;
        };
        self.project
            .project_editor_runtime
            .document(&document_id)
            .cloned()
    }

    fn active_terminal_pane(&self) -> Option<Entity<TerminalPaneView>> {
        let project_id = self.workspace.selected_project_id()?;
        let project = self.workspace.project(project_id)?;
        let pane_id = project
            .tab_state(&project.selected_tab_id)?
            .focused_pane_id
            .as_deref()?;
        let key = terminal_pane_key(project_id.as_str(), &project.selected_tab_id, pane_id);
        self.terminal.terminal_panes.get(&key).cloned()
    }

    pub(super) fn on_vim_enter_normal(
        &mut self,
        _: &VimEnterNormal,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.vim.enter_normal() {
            cx.propagate();
            return;
        }
        if self.vim.surface() == VimSurface::Terminal
            && let Some(pane) = self.active_terminal_pane()
        {
            pane.update(cx, |pane, pane_cx| {
                pane.set_terminal_vi_mode(true, pane_cx);
            });
        }
        cx.stop_propagation();
        cx.notify();
    }

    pub(super) fn on_vim_enter_insert(
        &mut self,
        _: &VimEnterInsert,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.vim.support() != VimModeSetting::Global || !self.vim.enter_insert() {
            cx.propagate();
            return;
        }
        cx.stop_propagation();
        cx.notify();
    }

    pub(super) fn on_vim_enter_terminal(
        &mut self,
        _: &VimEnterTerminal,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.vim.enter_terminal() {
            cx.propagate();
            return;
        }
        if let Some(pane) = self.active_terminal_pane() {
            pane.update(cx, |pane, pane_cx| {
                pane.set_terminal_vi_mode(false, pane_cx);
                pane.focus_terminal(window, pane_cx);
            });
        }
        cx.stop_propagation();
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use gpui::Modifiers;

    use super::*;

    #[test]
    fn vim_key_feedback_preserves_printable_character_case() {
        let lowercase = Keystroke {
            modifiers: Modifiers::default(),
            key: "g".to_string(),
            key_char: Some("g".to_string()),
        };
        let uppercase = Keystroke {
            modifiers: Modifiers {
                shift: true,
                ..Modifiers::default()
            },
            key: "g".to_string(),
            key_char: Some("G".to_string()),
        };

        assert_eq!(
            WorkbenchView::format_vim_key_feedback(&lowercase).as_deref(),
            Some("g")
        );
        assert_eq!(
            WorkbenchView::format_vim_key_feedback(&uppercase).as_deref(),
            Some("G")
        );
    }
}
