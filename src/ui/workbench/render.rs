use super::{
    bars::BarUnavailableReason,
    layout_editor::{BarEditorPreset, BarEditorRegion, LayoutEditorTarget},
    shell::bar::{BarHost, bar_sections_content},
};

use super::*;

impl Render for WorkbenchView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.performance.attach(window, cx);
        self.flush_pending_settings_save(window, cx);
        self.flush_pending_project_settings_save(window, cx);
        self.flush_pending_onboarding_completion(window, cx);
        self.ensure_settings_sync(window, cx);
        self.ensure_agent_initialization(window, cx);
        self.ensure_settings_project_target_loaded(window, cx);
        let appearance = self.appearance.runtime();
        self.app_settings.theme.ui_style = appearance.style_id;
        window.set_rem_size(px(appearance.typography.font_size));
        self.sync_auxiliary_windows(cx);
        self.sync_error_notification(window, cx);
        self.ensure_active_project_file_watcher(window, cx);
        self.ensure_keybindings_watcher(window, cx);
        self.flush_pending_keybindings_reload(cx);
        self.flush_pending_git_operations(window, cx);
        self.flush_pending_file_finder_operations(window, cx);
        self.ensure_agent_session_scan_requested();
        self.flush_pending_agent_session_scan(window, cx);
        self.flush_pending_project_tree_loads(window, cx);
        self.flush_pending_document_saves(window, cx);
        self.flush_pending_focus_change_autosaves(window, cx);
        self.flush_pending_file_close_requests(window, cx);
        self.flush_pending_project_close_requests(cx);
        self.sync_input_owner_state();
        self.prune_terminal_panes();
        self.ensure_eager_terminal_panes(window, cx);
        #[cfg(feature = "perf-metrics")]
        if std::env::var_os("YTTT_TERMINAL_PERF_OUTPUT").is_some()
            && let Some(terminal) = self.terminal.terminal_panes.iter().find_map(|(key, pane)| {
                key.ends_with(":perf:perf")
                    .then(|| pane.read(cx).performance_terminal())
                    .flatten()
            })
        {
            return div().flex().size_full().overflow_hidden().child(terminal);
        }
        self.ensure_vim_key_feedback_observers(window, cx);
        self.sync_vim_controller(window, cx);
        let focus_handle = self.workbench_focus_handle(cx);
        let default_active_content_focus_requested = self.onboarding.is_none()
            && !focus_handle.contains_focused(window, cx)
            && self.queue_default_active_work_item_focus(cx);

        let onboarding_needs_font_detection = self.onboarding.as_ref().is_some_and(|state| {
            state.step == OnboardingStep::Font
                && state.font_detection == OnboardingFontDetection::Pending
        });
        if onboarding_needs_font_detection {
            let system_fonts = cx.text_system().all_font_names();
            let recommendation =
                recommend_installed_monospace_nerd_font(&system_fonts, |font_family| {
                    font_family_has_fixed_ascii_width(window, font_family)
                });
            self.onboarding
                .as_mut()
                .expect("onboarding must exist while detecting terminal fonts")
                .font_detection = recommendation
                .map(OnboardingFontDetection::Recommended)
                .unwrap_or(OnboardingFontDetection::Missing);
        }

        let onboarding_terminal_font_select = self
            .onboarding
            .as_ref()
            .is_some_and(|state| state.step == OnboardingStep::Font)
            .then(|| self.settings_font_family_select(window, cx));
        let body = if self.workspace_is_loading() {
            div()
                .flex()
                .flex_1()
                .items_center()
                .justify_center()
                .child("正在恢复工作区…")
        } else if let Some(onboarding) = self.onboarding.as_ref() {
            let command_palette_keybinding =
                self.display_keybinding_for_command(CommandId::CommandPaletteOpen);
            onboarding_view(
                cx,
                onboarding,
                &self.ui_text,
                appearance.ui,
                appearance.style,
                onboarding_terminal_font_select.as_ref(),
                command_palette_keybinding,
            )
        } else if self.workspace.opened_projects().is_empty() {
            empty_workspace(
                window,
                cx,
                &self.ui_text,
                &appearance.ui,
                self.has_restorable_workspace(),
            )
        } else {
            self.reconcile_selected_work_area();
            let projects_focus_requested =
                self.pending_projects_focus && self.should_auto_focus_workspace();
            if projects_focus_requested {
                focus_handle.focus(window, cx);
                self.pending_projects_focus = false;
            }
            let projects_has_keyboard_focus = self.projects_focus_active
                && (projects_focus_requested || focus_handle.is_focused(window));
            let tab_items = self.workbench_tab_items(cx);
            let project_panel_visible = self.selected_project_panel_visible();
            let work_area_snapshot = self.selected_work_area_snapshot();
            let work_area = if let Some((project_id, node, active_group_id)) = work_area_snapshot {
                self.work_area_view(
                    &project_id,
                    &node,
                    active_group_id,
                    &tab_items,
                    project_panel_visible,
                    window,
                    cx,
                )
            } else {
                div().flex().flex_1()
            };
            let project_file_panel = project_panel_visible
                .then(|| self.project_file_panel(window, cx))
                .flatten();

            let workbench = div()
                .flex()
                .flex_1()
                .min_h_0()
                .relative()
                .bg(gpui::transparent_black())
                .text_color(appearance.ui.text)
                .child({
                    let sidebar = project_sidebar(
                        &self.workspace,
                        appearance.ui,
                        appearance.style,
                        self.ui_text,
                        focus_handle.clone(),
                        projects_has_keyboard_focus,
                        self.app_settings.project_panel.project_sidebar_width,
                        self.sidebar_collapsed,
                        &self.app_settings.project_panel.collapsed_agent_projects,
                        cx.listener(|this, _: &MouseDownEvent, _window, cx| {
                            if this.queue_projects_focus() {
                                cx.notify();
                            }
                        }),
                        cx.listener(|this, _, _window, cx| {
                            this.toggle_sidebar();
                            cx.notify();
                        }),
                        |project_id| {
                            let project_id = ProjectId::new(project_id);
                            cx.listener(move |this, event: &ClickEvent, _window, cx| {
                                let _ = this.select_project(&project_id);
                                if event.click_count() >= 2 {
                                    let _ = this.toggle_project_agent_expansion(&project_id);
                                }
                                cx.notify();
                            })
                        },
                        |project_id| {
                            let project_id = ProjectId::new(project_id);
                            cx.listener(move |this, _, _window, cx| {
                                cx.stop_propagation();
                                let _ = this.toggle_project_agent_expansion(&project_id);
                                cx.notify();
                            })
                        },
                        |project_id, tab_id, pane_id| {
                            let project_id = ProjectId::new(project_id);
                            cx.listener(move |this, _, _window, cx| {
                                let _ = this.activate_agent_pane(&project_id, &tab_id, &pane_id);
                                cx.notify();
                            })
                        },
                        |project_id| {
                            let project_id = ProjectId::new(project_id);
                            cx.listener(move |this, _: &MouseDownEvent, _window, cx| {
                                let _ = this.select_project(&project_id);
                                cx.notify();
                            })
                        },
                    );
                    let container = div().relative().flex_none().h_full().child(sidebar);
                    if self.sidebar_collapsed {
                        container
                    } else {
                        container.child(self.sidebar_resize_handle(SidebarSide::Left, cx))
                    }
                })
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .min_h_0()
                        .child(work_area),
                );
            if let Some(project_file_panel) = project_file_panel {
                workbench.child(project_file_panel)
            } else {
                workbench
            }
        };
        let default_active_content_focus_scheduled = default_active_content_focus_requested
            && match self.active_work_item() {
                Some(WorkItemId::Terminal(_)) => self.terminal.pending_terminal_focus.is_none(),
                Some(WorkItemId::File(document_id)) => self
                    .project
                    .pending_editor_focus_document_id
                    .as_ref()
                    .is_none_or(|pending| pending != &document_id),
                None => false,
            };

        let (window_bar_sections, status_bar_sections) = self.shell_bar_sections(window, cx);
        let mut root = div()
            .debug_selector(|| "workbench-surface".to_string())
            .flex()
            .flex_col()
            .size_full()
            .relative()
            .bg(appearance.ui.app_background)
            .text_color(appearance.ui.text)
            .line_height(relative(appearance.typography.line_height))
            .child(workbench_titlebar(
                window_bar_sections,
                appearance.ui,
                appearance.style,
                window,
            ))
            .child(body)
            .when_some(status_bar_sections, |root, sections| {
                root.child(workbench_status_bar(
                    sections,
                    appearance.ui,
                    appearance.style,
                ))
            });
        root = root.font_family(appearance.typography.font_family.clone());
        if let Some(active_palette) = self.palette.active_palette.clone() {
            let items = self.palette_items(active_palette.kind);
            if let Some(query_input) = self.palette_query_input(window, cx) {
                if active_palette.kind == PaletteKind::File {
                    let preview = self.file_finder_preview_element();
                    root = root.child(file_finder_palette_overlay(
                        &active_palette,
                        &items,
                        &self.ui_text,
                        &query_input,
                        &self.palette.scroll_handle,
                        preview,
                        appearance.ui,
                        appearance.style,
                        |selected_index| {
                            cx.listener(move |this, _, window, cx| {
                                if let Some(active_palette) = &mut this.palette.active_palette {
                                    active_palette.selected_index = selected_index;
                                }
                                let _ = this.confirm_palette_selection_with_context(window, cx);
                                cx.notify();
                            })
                        },
                    ));
                } else {
                    root = root.child(palette_overlay(
                        &active_palette,
                        &items,
                        &self.ui_text,
                        &query_input,
                        &self.palette.scroll_handle,
                        appearance.ui,
                        appearance.style,
                        |selected_index| {
                            cx.listener(move |this, _, window, cx| {
                                if let Some(active_palette) = &mut this.palette.active_palette {
                                    active_palette.selected_index = selected_index;
                                }
                                let _ = this.confirm_palette_selection_with_context(window, cx);
                                this.handle_pending_create_project_request(cx);
                                this.handle_pending_open_project_request(cx);
                                this.flush_pending_status_notifications(window, cx);
                                cx.notify();
                            })
                        },
                    ));
                }
            }
        }
        if self.ssh.project_picker.open {
            root = root.child(ssh_project_picker_overlay(self, window, cx));
        }
        if !self.settings.settings_page.is_open
            && let Some(dialog) = self.settings.zed_theme_import_dialog.clone()
        {
            root = root.child(zed_theme_import_dialog(
                cx,
                &self.ui_text,
                &dialog.detection,
                &dialog.existing_paths,
                dialog.conflict_policy,
                &self.config_paths,
                appearance.ui,
            ));
        }
        if let Some(panel) = self.render_git_diff_panel(window, cx) {
            root = root.child(panel);
        }
        if self.overlays.pending_tab_rename.is_some()
            && let Some(input) = self.tab_rename_input(window, cx)
        {
            root = root.child(tab_rename_dialog(cx, &self.ui_text, &input, appearance.ui));
        }
        if !self.settings.settings_page.is_open {
            root = self.render_keybinding_dialog(root, &focus_handle, window, cx);
        }
        if let Some(text) = self.visible_dirty_close_dialog_text() {
            let mut lines = text.lines();
            let title = lines.next().unwrap_or_default().to_string();
            let details = lines.map(str::to_string).collect::<Vec<_>>();
            let file_intent = self
                .documents
                .pending_dirty_close
                .as_ref()
                .is_some_and(|pending| matches!(pending.intent, DirtyCloseIntent::File(_)));
            root = root.child(dirty_close_dialog(
                cx,
                &self.ui_text,
                appearance.ui,
                title,
                details,
                file_intent,
                self.dirty_close_has_save_error(cx),
            ));
        }
        if self.overlays.pending_close_project_id.is_some() {
            root = root.child(close_project_dialog(cx, &self.ui_text, appearance.ui));
        }
        if let Some(conflict) = self.documents.pending_file_conflict.as_ref() {
            root = root.child(file_conflict_dialog(
                cx,
                &self.ui_text,
                appearance.ui,
                conflict.document_id.canonical_path.display().to_string(),
            ));
        }
        if !self.ssh.pending_host_keys.is_empty() && !self.ssh.manager_open {
            root = root.child(ssh_host_key_overlay(self, cx));
        }
        if let Some(notification_layer) = ComponentRoot::render_notification_layer(window, cx) {
            root = root.child(notification_layer);
        }
        if let Some(sheet_layer) = ComponentRoot::render_sheet_layer(window, cx) {
            root = root.child(sheet_layer);
        }
        if let Some(dialog_layer) = ComponentRoot::render_dialog_layer(window, cx) {
            root = root.child(dialog_layer);
        }

        if self.onboarding.is_none()
            && self.should_auto_focus_workspace()
            && !focus_handle.contains_focused(window, cx)
            && !default_active_content_focus_scheduled
        {
            focus_handle.focus(window, cx);
        }

        let input_owner = self.foreground_input_owner_kind();
        let mut key_context = if input_owner == InputOwnerKind::KeybindingRecorder {
            gpui::KeyContext::new_with_defaults()
        } else {
            self.vim.current_key_context()
        };
        if input_owner == InputOwnerKind::KeybindingRecorder {
            key_context.add("YtttKeybindingRecorder");
        } else {
            key_context.add(WORKSPACE_CONTEXT);
            if input_owner == InputOwnerKind::Palette {
                key_context.add(PALETTE_CONTEXT);
            }
        }

        root.track_focus(&focus_handle)
            .key_context(key_context)
            .on_key_down(cx.listener(Self::on_key_down))
            .on_mouse_move(cx.listener(Self::on_resize_mouse_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_resize_mouse_up))
            .on_action(cx.listener(Self::on_create_project))
            .on_action(cx.listener(Self::on_application_quit))
            .on_action(cx.listener(Self::on_open_project))
            .on_action(cx.listener(Self::on_open_ssh_project))
            .on_action(cx.listener(Self::on_connect_existing_host))
            .on_action(cx.listener(Self::on_open_command_palette))
            .on_action(cx.listener(Self::on_open_file_finder))
            .on_action(cx.listener(Self::on_open_project_palette))
            .on_action(cx.listener(Self::on_opened_project_palette))
            .on_action(cx.listener(Self::on_recent_project_palette))
            .on_action(cx.listener(Self::on_project_panel_toggle))
            .on_action(cx.listener(Self::on_project_panel_refresh))
            .on_action(cx.listener(Self::on_project_panel_select_previous_page))
            .on_action(cx.listener(Self::on_project_panel_select_next_page))
            .on_action(cx.listener(Self::on_project_panel_select_files_page))
            .on_action(cx.listener(Self::on_focus_projects))
            .on_action(cx.listener(Self::on_projects_select_previous))
            .on_action(cx.listener(Self::on_projects_select_next))
            .on_action(cx.listener(Self::on_projects_select_first))
            .on_action(cx.listener(Self::on_projects_select_last))
            .on_action(cx.listener(Self::on_open_tab_palette))
            .on_action(cx.listener(Self::on_open_pane_palette))
            .on_action(cx.listener(Self::on_palette_select_next))
            .on_action(cx.listener(Self::on_palette_select_prev))
            .on_action(cx.listener(Self::on_palette_confirm))
            .on_action(cx.listener(Self::on_palette_cancel))
            .on_action(cx.listener(Self::on_project_close))
            .on_action(cx.listener(Self::on_tab_new))
            .on_action(cx.listener(Self::on_tab_close))
            .on_action(cx.listener(Self::on_tab_close_all))
            .on_action(cx.listener(Self::on_tab_close_before))
            .on_action(cx.listener(Self::on_tab_close_after))
            .on_action(cx.listener(Self::on_tab_close_all_files))
            .on_action(cx.listener(Self::on_tab_close_all_terminals))
            .on_action(cx.listener(Self::on_tab_rename))
            .on_action(cx.listener(Self::on_tab_next))
            .on_action(cx.listener(Self::on_tab_prev))
            .on_action(cx.listener(Self::on_file_save))
            .on_action(cx.listener(Self::on_git_branch_switch))
            .on_action(cx.listener(Self::on_git_diff_close))
            .on_action(cx.listener(Self::on_git_diff_toggle_stage_mode))
            .on_action(cx.listener(Self::on_git_diff_toggle_view_mode))
            .on_action(cx.listener(Self::on_git_diff_toggle_whitespace))
            .on_action(cx.listener(Self::on_git_diff_select_previous_file))
            .on_action(cx.listener(Self::on_git_diff_select_next_file))
            .on_action(cx.listener(Self::on_git_diff_copy_selected))
            .on_action(cx.listener(Self::on_git_diff_open))
            .on_action(cx.listener(Self::on_pane_split_vertical))
            .on_action(cx.listener(Self::on_pane_split_horizontal))
            .on_action(cx.listener(Self::on_pane_close))
            .on_action(cx.listener(Self::on_pane_rename))
            .on_action(cx.listener(Self::on_pane_focus_left))
            .on_action(cx.listener(Self::on_pane_focus_right))
            .on_action(cx.listener(Self::on_pane_focus_up))
            .on_action(cx.listener(Self::on_pane_focus_down))
            .on_action(cx.listener(Self::on_pane_resize_left))
            .on_action(cx.listener(Self::on_pane_resize_right))
            .on_action(cx.listener(Self::on_pane_resize_up))
            .on_action(cx.listener(Self::on_pane_resize_down))
            .on_action(cx.listener(Self::on_layout_default_edit))
            .on_action(cx.listener(Self::on_layout_default_reset))
            .on_action(cx.listener(Self::on_layout_default_reload))
            .on_action(cx.listener(Self::on_layout_project_edit))
            .on_action(cx.listener(Self::on_layout_save_current))
            .on_action(cx.listener(Self::on_layout_export_project_config))
            .on_action(cx.listener(Self::on_layout_reset_local_override))
            .on_action(cx.listener(Self::on_layout_open_file))
            .on_action(cx.listener(Self::on_settings_open))
            .on_action(cx.listener(Self::on_settings_keybindings))
            .on_action(cx.listener(Self::on_settings_notifications))
            .on_action(cx.listener(Self::on_settings_vim_previous_group))
            .on_action(cx.listener(Self::on_settings_vim_next_group))
            .on_action(cx.listener(Self::on_settings_vim_first_group))
            .on_action(cx.listener(Self::on_settings_vim_last_group))
            .on_action(cx.listener(Self::on_vim_enter_normal))
            .on_action(cx.listener(Self::on_vim_enter_insert))
            .on_action(cx.listener(Self::on_vim_enter_terminal))
    }
}

impl WorkbenchView {
    pub(super) fn render_keybinding_dialog(
        &mut self,
        mut root: Div,
        focus_handle: &FocusHandle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        if self.overlays.pending_keybinding_edit.is_some() {
            if self.overlays.keybinding_recorder_needs_focus {
                focus_handle.focus(window, cx);
                self.overlays.keybinding_recorder_needs_focus = false;
            }
            if let Some(edit) = self.overlays.pending_keybinding_edit.as_ref() {
                root = root.child(keybinding_edit_dialog(
                    cx,
                    &self.ui_text,
                    edit.action,
                    edit.profile,
                    &edit.keys,
                    &edit.original_keys,
                    edit.is_recording,
                    edit.recording_index,
                    edit.error.as_deref(),
                    self.theme_runtime().ui,
                ));
            }
        }
        root
    }
}

pub(super) fn split_child(child: Div, basis: f32) -> Div {
    div()
        .flex()
        .flex_col()
        .flex_basis(relative(basis))
        .flex_shrink(1.0)
        .overflow_hidden()
        .child(child)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BarComponentCategory {
    Project,
    EditorVim,
    Terminal,
    Agent,
    Performance,
    LayoutActions,
}

#[derive(Clone, Copy)]
struct BarComponentCatalog {
    id: &'static str,
    name: UiTextKey,
    description: UiTextKey,
    category: BarComponentCategory,
}
const BAR_COMPONENT_CATEGORIES: [BarComponentCategory; 6] = [
    BarComponentCategory::Project,
    BarComponentCategory::EditorVim,
    BarComponentCategory::Terminal,
    BarComponentCategory::Agent,
    BarComponentCategory::Performance,
    BarComponentCategory::LayoutActions,
];

const BAR_COMPONENT_CATALOG: [BarComponentCatalog; 41] = [
    BarComponentCatalog {
        id: "project-name",
        name: UiTextKey::BarsComponentProjectNameName,
        description: UiTextKey::BarsComponentProjectNameDescription,
        category: BarComponentCategory::Project,
    },
    BarComponentCatalog {
        id: "project-path",
        name: UiTextKey::BarsComponentProjectPathName,
        description: UiTextKey::BarsComponentProjectPathDescription,
        category: BarComponentCategory::Project,
    },
    BarComponentCatalog {
        id: "git-branch",
        name: UiTextKey::BarsComponentGitBranchName,
        description: UiTextKey::BarsComponentGitBranchDescription,
        category: BarComponentCategory::Project,
    },
    BarComponentCatalog {
        id: "git-changes",
        name: UiTextKey::BarsComponentGitChangesName,
        description: UiTextKey::BarsComponentGitChangesDescription,
        category: BarComponentCategory::Project,
    },
    BarComponentCatalog {
        id: "ssh",
        name: UiTextKey::BarsComponentSshName,
        description: UiTextKey::BarsComponentSshDescription,
        category: BarComponentCategory::Project,
    },
    BarComponentCatalog {
        id: "update",
        name: UiTextKey::BarsComponentUpdateName,
        description: UiTextKey::BarsComponentUpdateDescription,
        category: BarComponentCategory::Project,
    },
    BarComponentCatalog {
        id: "active-item",
        name: UiTextKey::BarsComponentActiveItemName,
        description: UiTextKey::BarsComponentActiveItemDescription,
        category: BarComponentCategory::EditorVim,
    },
    BarComponentCatalog {
        id: "surface",
        name: UiTextKey::BarsComponentSurfaceName,
        description: UiTextKey::BarsComponentSurfaceDescription,
        category: BarComponentCategory::EditorVim,
    },
    BarComponentCatalog {
        id: "vim-mode",
        name: UiTextKey::BarsComponentVimModeName,
        description: UiTextKey::BarsComponentVimModeDescription,
        category: BarComponentCategory::EditorVim,
    },
    BarComponentCatalog {
        id: "vim-detail",
        name: UiTextKey::BarsComponentVimDetailName,
        description: UiTextKey::BarsComponentVimDetailDescription,
        category: BarComponentCategory::EditorVim,
    },
    BarComponentCatalog {
        id: "vim-keys",
        name: UiTextKey::BarsComponentVimKeysName,
        description: UiTextKey::BarsComponentVimKeysDescription,
        category: BarComponentCategory::EditorVim,
    },
    BarComponentCatalog {
        id: "editor-language",
        name: UiTextKey::BarsComponentEditorLanguageName,
        description: UiTextKey::BarsComponentEditorLanguageDescription,
        category: BarComponentCategory::EditorVim,
    },
    BarComponentCatalog {
        id: "editor-position",
        name: UiTextKey::BarsComponentEditorPositionName,
        description: UiTextKey::BarsComponentEditorPositionDescription,
        category: BarComponentCategory::EditorVim,
    },
    BarComponentCatalog {
        id: "editor-dirty",
        name: UiTextKey::BarsComponentEditorDirtyName,
        description: UiTextKey::BarsComponentEditorDirtyDescription,
        category: BarComponentCategory::EditorVim,
    },
    BarComponentCatalog {
        id: "editor-diagnostics",
        name: UiTextKey::BarsComponentEditorDiagnosticsName,
        description: UiTextKey::BarsComponentEditorDiagnosticsDescription,
        category: BarComponentCategory::EditorVim,
    },
    BarComponentCatalog {
        id: "editor-selection",
        name: UiTextKey::BarsComponentEditorSelectionName,
        description: UiTextKey::BarsComponentEditorSelectionDescription,
        category: BarComponentCategory::EditorVim,
    },
    BarComponentCatalog {
        id: "editor-tab-size",
        name: UiTextKey::BarsComponentEditorTabSizeName,
        description: UiTextKey::BarsComponentEditorTabSizeDescription,
        category: BarComponentCategory::EditorVim,
    },
    BarComponentCatalog {
        id: "editor-wrap",
        name: UiTextKey::BarsComponentEditorWrapName,
        description: UiTextKey::BarsComponentEditorWrapDescription,
        category: BarComponentCategory::EditorVim,
    },
    BarComponentCatalog {
        id: "terminal-title",
        name: UiTextKey::BarsComponentTerminalTitleName,
        description: UiTextKey::BarsComponentTerminalTitleDescription,
        category: BarComponentCategory::Terminal,
    },
    BarComponentCatalog {
        id: "terminal-state",
        name: UiTextKey::BarsComponentTerminalStateName,
        description: UiTextKey::BarsComponentTerminalStateDescription,
        category: BarComponentCategory::Terminal,
    },
    BarComponentCatalog {
        id: "terminal-exit",
        name: UiTextKey::BarsComponentTerminalExitName,
        description: UiTextKey::BarsComponentTerminalExitDescription,
        category: BarComponentCategory::Terminal,
    },
    BarComponentCatalog {
        id: "terminal-size",
        name: UiTextKey::BarsComponentTerminalSizeName,
        description: UiTextKey::BarsComponentTerminalSizeDescription,
        category: BarComponentCategory::Terminal,
    },
    BarComponentCatalog {
        id: "agent-state",
        name: UiTextKey::BarsComponentAgentStateName,
        description: UiTextKey::BarsComponentAgentStateDescription,
        category: BarComponentCategory::Agent,
    },
    BarComponentCatalog {
        id: "agent-waiting",
        name: UiTextKey::BarsComponentAgentWaitingName,
        description: UiTextKey::BarsComponentAgentWaitingDescription,
        category: BarComponentCategory::Agent,
    },
    BarComponentCatalog {
        id: "agent-model",
        name: UiTextKey::BarsComponentAgentModelName,
        description: UiTextKey::BarsComponentAgentModelDescription,
        category: BarComponentCategory::Agent,
    },
    BarComponentCatalog {
        id: "agent-children",
        name: UiTextKey::BarsComponentAgentChildrenName,
        description: UiTextKey::BarsComponentAgentChildrenDescription,
        category: BarComponentCategory::Agent,
    },
    BarComponentCatalog {
        id: "agent-state-duration",
        name: UiTextKey::BarsComponentAgentStateDurationName,
        description: UiTextKey::BarsComponentAgentStateDurationDescription,
        category: BarComponentCategory::Agent,
    },
    BarComponentCatalog {
        id: "projects-count",
        name: UiTextKey::BarsComponentProjectsCountName,
        description: UiTextKey::BarsComponentProjectsCountDescription,
        category: BarComponentCategory::Performance,
    },
    BarComponentCatalog {
        id: "terminals-count",
        name: UiTextKey::BarsComponentTerminalsCountName,
        description: UiTextKey::BarsComponentTerminalsCountDescription,
        category: BarComponentCategory::Performance,
    },
    BarComponentCatalog {
        id: "tabs-count",
        name: UiTextKey::BarsComponentTabsCountName,
        description: UiTextKey::BarsComponentTabsCountDescription,
        category: BarComponentCategory::Performance,
    },
    BarComponentCatalog {
        id: "editors-count",
        name: UiTextKey::BarsComponentEditorsCountName,
        description: UiTextKey::BarsComponentEditorsCountDescription,
        category: BarComponentCategory::Performance,
    },
    BarComponentCatalog {
        id: "app-cpu",
        name: UiTextKey::BarsComponentAppCpuName,
        description: UiTextKey::BarsComponentAppCpuDescription,
        category: BarComponentCategory::Performance,
    },
    BarComponentCatalog {
        id: "app-memory",
        name: UiTextKey::BarsComponentAppMemoryName,
        description: UiTextKey::BarsComponentAppMemoryDescription,
        category: BarComponentCategory::Performance,
    },
    BarComponentCatalog {
        id: "system-cpu",
        name: UiTextKey::BarsComponentSystemCpuName,
        description: UiTextKey::BarsComponentSystemCpuDescription,
        category: BarComponentCategory::Performance,
    },
    BarComponentCatalog {
        id: "system-memory",
        name: UiTextKey::BarsComponentSystemMemoryName,
        description: UiTextKey::BarsComponentSystemMemoryDescription,
        category: BarComponentCategory::Performance,
    },
    BarComponentCatalog {
        id: "command-palette",
        name: UiTextKey::BarsComponentCommandPaletteName,
        description: UiTextKey::BarsComponentCommandPaletteDescription,
        category: BarComponentCategory::LayoutActions,
    },
    BarComponentCatalog {
        id: "settings",
        name: UiTextKey::BarsComponentSettingsName,
        description: UiTextKey::BarsComponentSettingsDescription,
        category: BarComponentCategory::LayoutActions,
    },
    BarComponentCatalog {
        id: "Space: 5",
        name: UiTextKey::BarsComponentSpaceName,
        description: UiTextKey::BarsComponentSpaceDescription,
        category: BarComponentCategory::LayoutActions,
    },
    BarComponentCatalog {
        id: "text:Text",
        name: UiTextKey::BarsComponentTextName,
        description: UiTextKey::BarsComponentTextDescription,
        category: BarComponentCategory::LayoutActions,
    },
    BarComponentCatalog {
        id: "icon:settings",
        name: UiTextKey::BarsComponentIconName,
        description: UiTextKey::BarsComponentIconDescription,
        category: BarComponentCategory::LayoutActions,
    },
    BarComponentCatalog {
        id: "|",
        name: UiTextKey::BarsComponentSeparatorName,
        description: UiTextKey::BarsComponentSeparatorDescription,
        category: BarComponentCategory::LayoutActions,
    },
];

pub(super) fn layout_toml_editor_window_content(
    root: &mut WorkbenchView,
    input: &Entity<InputState>,
    window: &mut Window,
    cx: &mut Context<WorkbenchView>,
) -> Div {
    if root
        .overlays
        .layout_toml_editor
        .as_ref()
        .is_some_and(|session| matches!(session.target(), LayoutEditorTarget::Bars))
    {
        return bars_toml_editor_window_content(root, input, window, cx);
    }
    standard_layout_toml_editor_window_content(root, input, cx)
}

fn standard_layout_toml_editor_window_content(
    root: &mut WorkbenchView,
    input: &Entity<InputState>,
    cx: &mut Context<WorkbenchView>,
) -> Div {
    let appearance = root.theme_runtime();
    let theme = appearance.ui;
    let style = appearance.style;
    let Some(session) = root.overlays.layout_toml_editor.as_ref() else {
        return div();
    };
    let editor = session.editor();
    div()
        .flex()
        .flex_col()
        .size_full()
        .bg(appearance.editor.background)
        .child(
            div()
                .flex_none()
                .px(gpui::rems(1.0))
                .py(gpui::rems(0.375))
                .border_b(style.border.hairline)
                .border_color(theme.border_variant)
                .text_xs()
                .text_color(theme.text_muted)
                .truncate()
                .child(editor.path().display().to_string()),
        )
        .child(
            div()
                .debug_selector(|| "layout-editor-content".into())
                .flex_1()
                .min_h_0()
                .overflow_hidden()
                .child(styled_code_editor_input(input, session.appearance()).h_full()),
        )
        .when_some(editor.error(), |this, error| {
            this.child(
                div()
                    .px(gpui::rems(1.0))
                    .py(gpui::rems(0.5))
                    .border_t(style.border.hairline)
                    .border_color(theme.border_variant)
                    .text_sm()
                    .text_color(theme.danger)
                    .child(error.to_string()),
            )
        })
        .child(layout_toml_editor_footer(root, theme, style, cx))
}

fn bars_toml_editor_window_content(
    root: &mut WorkbenchView,
    input: &Entity<InputState>,
    window: &mut Window,
    cx: &mut Context<WorkbenchView>,
) -> Div {
    root.sync_bar_state_duration_refresh(cx);
    let appearance = root.theme_runtime();
    let theme = appearance.ui;
    let style = appearance.style;
    let text = root.ui_text;
    let component_search = root.bar_component_search_input(window, cx);
    let Some(session) = root
        .overlays
        .layout_toml_editor
        .as_ref()
        .filter(|session| matches!(session.target(), LayoutEditorTarget::Bars))
    else {
        return div();
    };
    let path = session.editor().path().display().to_string();
    let editor_appearance = session.appearance();
    let error = session.editor().error();
    let query = session.bar_component_query().trim();
    let selected_region = session.bar_insert_region();
    let visible_components = BAR_COMPONENT_CATALOG
        .iter()
        .filter(|component| bar_component_matches(component, query, text))
        .collect::<Vec<_>>();
    let preview =
        match session.bars_preview() {
            Some(bars) => {
                let unavailable = bars_preview_unavailable(root, bars, cx);
                let window_preview =
                    bars_preview_surface(root, BarHost::Window, &bars.window.layout, window, cx);
                let status_preview = bars.status.enabled.then(|| {
                    bars_preview_surface(root, BarHost::Status, &bars.status.layout, window, cx)
                });
                div()
                    .debug_selector(|| "bars-editor-preview".to_string())
                    .flex()
                    .flex_col()
                    .gap(style.spacing.sm)
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.text_muted)
                            .child(text.get(UiTextKey::BarsEditorPreview)),
                    )
                    .child(window_preview)
                    .when_some(status_preview, |preview, status_preview| {
                        preview.child(status_preview)
                    })
                    .when(!bars.status.enabled, |preview| {
                        preview.child(
                            div()
                                .text_xs()
                                .text_color(theme.text_muted)
                                .child(text.get(UiTextKey::BarsEditorPreviewStatusDisabled)),
                        )
                    })
                    .when(!unavailable.is_empty(), |preview| {
                        preview.child(
                            div()
                                .id("bars-preview-unavailable-scroll")
                                .debug_selector(|| {
                                    "bars-editor-preview-unavailable-components".to_string()
                                })
                                .flex()
                                .flex_col()
                                .gap(style.spacing.xs)
                                .max_h(px(96.0))
                                .overflow_y_scroll()
                                .pt(style.spacing.xs)
                                .border_t(style.border.hairline)
                                .border_color(theme.border_variant)
                                .child(div().text_xs().text_color(theme.text_muted).child(
                                    text.get(UiTextKey::BarsEditorPreviewUnavailableModules),
                                ))
                                .children(unavailable.into_iter().map(|(component, reason)| {
                                    div().text_xs().text_color(theme.text_muted).child(format!(
                                        "{} — {}",
                                        bar_component_name(&component, text),
                                        text.get(bar_unavailable_reason_key(reason))
                                    ))
                                })),
                        )
                    })
            }
            None => div()
                .debug_selector(|| "bars-editor-preview-unavailable".to_string())
                .text_xs()
                .text_color(theme.text_muted)
                .child(text.get(UiTextKey::BarsEditorPreviewUnavailable)),
        };

    div()
        .flex()
        .flex_col()
        .size_full()
        .bg(appearance.editor.background)
        .child(
            div()
                .flex_none()
                .px(gpui::rems(1.0))
                .py(gpui::rems(0.375))
                .border_b(style.border.hairline)
                .border_color(theme.border_variant)
                .text_xs()
                .text_color(theme.text_muted)
                .truncate()
                .child(path),
        )
        .child(
            div()
                .flex()
                .flex_1()
                .min_h_0()
                .child(
                    div()
                        .debug_selector(|| "bars-editor-components".to_string())
                        .flex()
                        .flex_col()
                        .w(px(224.0))
                        .flex_none()
                        .min_h_0()
                        .border_r(style.border.hairline)
                        .border_color(theme.border_variant)
                        .child(
                            div()
                                .px(gpui::rems(0.75))
                                .pt(gpui::rems(0.75))
                                .text_xs()
                                .text_color(theme.text_muted)
                                .child(text.get(UiTextKey::BarsEditorComponents)),
                        )
                        .when_some(component_search, |panel, search| {
                            panel.child(
                                div()
                                    .debug_selector(|| "bars-editor-component-search".to_string())
                                    .px(gpui::rems(0.75))
                                    .py(gpui::rems(0.5))
                                    .child(
                                        yttt_input(&search, YtttInputKind::Search, theme, style)
                                            .small(),
                                    ),
                            )
                        })
                        .child(
                            div()
                                .debug_selector(|| "bars-editor-presets".to_string())
                                .flex_none()
                                .flex()
                                .flex_col()
                                .gap(style.spacing.xs)
                                .px(gpui::rems(0.75))
                                .pb(gpui::rems(0.5))
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(theme.text_muted)
                                        .child(text.get(UiTextKey::BarsEditorPresets)),
                                )
                                .child(
                                    div()
                                        .flex()
                                        .flex_wrap()
                                        .gap(style.spacing.xs)
                                        .child(
                                            settings_button(
                                                "bars-editor-preset-recommended",
                                                text.get(UiTextKey::BarsEditorPresetRecommended),
                                                false,
                                                theme,
                                                cx,
                                                cx.listener(|this, _, _window, cx| {
                                                    this.apply_bar_editor_preset(
                                                        BarEditorPreset::Recommended,
                                                    );
                                                    cx.notify();
                                                }),
                                            )
                                            .debug_selector(|| {
                                                "bars-editor-preset-recommended".to_string()
                                            }),
                                        )
                                        .child(
                                            settings_button(
                                                "bars-editor-preset-minimal",
                                                text.get(UiTextKey::BarsEditorPresetMinimal),
                                                false,
                                                theme,
                                                cx,
                                                cx.listener(|this, _, _window, cx| {
                                                    this.apply_bar_editor_preset(
                                                        BarEditorPreset::Minimal,
                                                    );
                                                    cx.notify();
                                                }),
                                            )
                                            .debug_selector(|| {
                                                "bars-editor-preset-minimal".to_string()
                                            }),
                                        )
                                        .child(
                                            settings_button(
                                                "bars-editor-preset-development",
                                                text.get(UiTextKey::BarsEditorPresetDevelopment),
                                                false,
                                                theme,
                                                cx,
                                                cx.listener(|this, _, _window, cx| {
                                                    this.apply_bar_editor_preset(
                                                        BarEditorPreset::Development,
                                                    );
                                                    cx.notify();
                                                }),
                                            )
                                            .debug_selector(|| {
                                                "bars-editor-preset-development".to_string()
                                            }),
                                        )
                                        .child(
                                            settings_button(
                                                "bars-editor-preset-agent",
                                                text.get(UiTextKey::BarsEditorPresetAgent),
                                                false,
                                                theme,
                                                cx,
                                                cx.listener(|this, _, _window, cx| {
                                                    this.apply_bar_editor_preset(
                                                        BarEditorPreset::Agent,
                                                    );
                                                    cx.notify();
                                                }),
                                            )
                                            .debug_selector(|| {
                                                "bars-editor-preset-agent".to_string()
                                            }),
                                        )
                                        .child(
                                            settings_button(
                                                "bars-editor-restore-defaults",
                                                text.get(UiTextKey::BarsEditorRestoreDefaults),
                                                false,
                                                theme,
                                                cx,
                                                cx.listener(|this, _, _window, cx| {
                                                    this.restore_bar_editor_defaults();
                                                    cx.notify();
                                                }),
                                            )
                                            .debug_selector(|| {
                                                "bars-editor-restore-defaults".to_string()
                                            }),
                                        ),
                                ),
                        )
                        .child(
                            div()
                                .id("bars-component-list")
                                .flex()
                                .flex_col()
                                .flex_1()
                                .min_h_0()
                                .overflow_y_scroll()
                                .px(gpui::rems(0.5))
                                .pb(gpui::rems(0.75))
                                .gap(style.spacing.sm)
                                .children(BAR_COMPONENT_CATEGORIES.into_iter().filter_map(
                                    |category| {
                                        let mut components = visible_components
                                            .iter()
                                            .copied()
                                            .filter(|component| component.category == category);
                                        let first = components.next()?;
                                        Some(
                                            div()
                                                .flex()
                                                .flex_col()
                                                .gap(style.spacing.xs)
                                                .child(
                                                    div()
                                                        .text_xs()
                                                        .text_color(theme.text_muted)
                                                        .child(bar_component_category_name(
                                                            category, text,
                                                        )),
                                                )
                                                .children(
                                                    std::iter::once(first).chain(components).map(
                                                        |component| {
                                                            bar_component_catalog_entry(
                                                                component, text, theme, style, cx,
                                                            )
                                                        },
                                                    ),
                                                ),
                                        )
                                    },
                                )),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .min_h_0()
                        .child(
                            div()
                                .debug_selector(|| "layout-editor-content".into())
                                .flex_1()
                                .min_h_0()
                                .overflow_hidden()
                                .child(styled_code_editor_input(input, editor_appearance).h_full()),
                        )
                        .when_some(error, |editor, error| {
                            editor.child(
                                div()
                                    .px(gpui::rems(1.0))
                                    .py(gpui::rems(0.5))
                                    .border_t(style.border.hairline)
                                    .border_color(theme.border_variant)
                                    .text_sm()
                                    .text_color(theme.danger)
                                    .child(error.to_string()),
                            )
                        })
                        .child(
                            div()
                                .debug_selector(|| "bars-editor-insert-region".to_string())
                                .flex_none()
                                .flex()
                                .flex_wrap()
                                .items_center()
                                .gap(style.spacing.xs)
                                .px(gpui::rems(0.75))
                                .py(gpui::rems(0.5))
                                .border_t(style.border.hairline)
                                .border_color(theme.border_variant)
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(theme.text_muted)
                                        .child(text.get(UiTextKey::BarsEditorInsertInto)),
                                )
                                .children(BarEditorRegion::ALL.into_iter().map(|region| {
                                    settings_button(
                                        format!("bars-editor-region-{}", region.path()),
                                        bar_editor_region_label(region, text),
                                        region == selected_region,
                                        theme,
                                        cx,
                                        cx.listener(move |this, _, _window, cx| {
                                            this.set_bar_insert_region(region);
                                            cx.notify();
                                        }),
                                    )
                                })),
                        )
                        .child(
                            div()
                                .flex_none()
                                .p(gpui::rems(0.75))
                                .border_t(style.border.hairline)
                                .border_color(theme.border_variant)
                                .bg(theme.statusbar_background)
                                .child(preview),
                        ),
                ),
        )
        .child(layout_toml_editor_footer(root, theme, style, cx))
}

fn bar_component_matches(component: &BarComponentCatalog, query: &str, text: UiText) -> bool {
    query.is_empty()
        || [
            component.id,
            text.get(component.name),
            text.get(component.description),
        ]
        .into_iter()
        .any(|candidate| ascii_case_insensitive_contains(candidate, query))
}

fn ascii_case_insensitive_contains(candidate: &str, query: &str) -> bool {
    candidate
        .as_bytes()
        .windows(query.len())
        .any(|part| part.eq_ignore_ascii_case(query.as_bytes()))
}

fn bar_component_catalog_entry(
    component: &BarComponentCatalog,
    text: UiText,
    theme: WorkbenchTheme,
    style: UiStyle,
    cx: &mut Context<WorkbenchView>,
) -> Div {
    let id = component.id;
    div()
        .flex()
        .flex_col()
        .gap(style.spacing.xs)
        .pb(style.spacing.xs)
        .child(
            settings_button(
                format!("bars-editor-insert-{id}"),
                format!("{} · {id}", text.get(component.name)),
                false,
                theme,
                cx,
                cx.listener(move |this, _, _window, cx| {
                    this.insert_bar_component(id);
                    cx.notify();
                }),
            )
            .debug_selector(move || format!("bars-editor-insert-{id}"))
            .w_full(),
        )
        .child(
            div()
                .text_xs()
                .text_color(theme.text_muted)
                .child(text.get(component.description)),
        )
        .child(div().text_xs().text_color(theme.text_muted).child(format!(
            "{}: {}",
            text.get(UiTextKey::BarsEditorCatalogSample),
            format!("[{}]", component.id)
        )))
        .child(div().text_xs().text_color(theme.text_muted).child(format!(
            "{}: {}",
            text.get(UiTextKey::BarsEditorCatalogScope),
            bar_component_category_name(component.category, text)
        )))
}

fn bar_component_category_name(category: BarComponentCategory, text: UiText) -> &'static str {
    text.get(match category {
        BarComponentCategory::Project => UiTextKey::BarsEditorCategoryProject,
        BarComponentCategory::EditorVim => UiTextKey::BarsEditorCategoryEditorVim,
        BarComponentCategory::Terminal => UiTextKey::BarsEditorCategoryTerminal,
        BarComponentCategory::Agent => UiTextKey::BarsEditorCategoryAgent,
        BarComponentCategory::Performance => UiTextKey::BarsEditorCategoryPerformance,
        BarComponentCategory::LayoutActions => UiTextKey::BarsEditorCategoryLayoutActions,
    })
}

fn bar_component_name<'a>(id: &'a str, text: UiText) -> &'a str {
    BAR_COMPONENT_CATALOG
        .iter()
        .find(|component| component.id == id)
        .map(|component| text.get(component.name))
        .unwrap_or(id)
}

fn bars_preview_unavailable(
    root: &WorkbenchView,
    bars: &crate::config::bars::ShellBarsSettings,
    cx: &gpui::App,
) -> Vec<(String, BarUnavailableReason)> {
    let mut unavailable = root.bar_preview_unavailable(&bars.window.layout, cx);
    if bars.status.enabled {
        unavailable.extend(root.bar_preview_unavailable(&bars.status.layout, cx));
    }
    let mut reported = std::collections::BTreeSet::new();
    unavailable
        .into_iter()
        .filter(|(module, _)| reported.insert(module.clone()))
        .collect()
}

fn bar_unavailable_reason_key(reason: BarUnavailableReason) -> UiTextKey {
    match reason {
        BarUnavailableReason::NoProject => UiTextKey::BarsEditorReasonNoProject,
        BarUnavailableReason::NoEditor => UiTextKey::BarsEditorReasonNoEditor,
        BarUnavailableReason::NoCodeEditor => UiTextKey::BarsEditorReasonNoCodeEditor,
        BarUnavailableReason::NoSelection => UiTextKey::BarsEditorReasonNoSelection,
        BarUnavailableReason::NoTerminal => UiTextKey::BarsEditorReasonNoTerminal,
        BarUnavailableReason::TerminalNotExited => UiTextKey::BarsEditorReasonTerminalNotExited,
        BarUnavailableReason::TerminalSizeUnavailable => {
            UiTextKey::BarsEditorReasonTerminalSizeUnavailable
        }
        BarUnavailableReason::NoAgent => UiTextKey::BarsEditorReasonNoAgent,
        BarUnavailableReason::AgentNotWaiting => UiTextKey::BarsEditorReasonAgentNotWaiting,
        BarUnavailableReason::AgentModelUnavailable => {
            UiTextKey::BarsEditorReasonAgentModelUnavailable
        }
        BarUnavailableReason::NoActiveChildren => UiTextKey::BarsEditorReasonNoActiveChildren,
        BarUnavailableReason::NoGit => UiTextKey::BarsEditorReasonNoGit,
        BarUnavailableReason::GitClean => UiTextKey::BarsEditorReasonGitClean,
        BarUnavailableReason::NoSsh => UiTextKey::BarsEditorReasonNoSsh,
        BarUnavailableReason::VimDisabled => UiTextKey::BarsEditorReasonVimDisabled,
        BarUnavailableReason::NoVimDetail => UiTextKey::BarsEditorReasonNoVimDetail,
        BarUnavailableReason::NoVimKeys => UiTextKey::BarsEditorReasonNoVimKeys,
        BarUnavailableReason::EditorClean => UiTextKey::BarsEditorReasonEditorClean,
        BarUnavailableReason::NoDiagnostics => UiTextKey::BarsEditorReasonNoDiagnostics,
        BarUnavailableReason::PerformanceUnavailable => {
            UiTextKey::BarsEditorReasonPerformanceUnavailable
        }
        BarUnavailableReason::NoUpdate => UiTextKey::BarsEditorReasonNoUpdate,
    }
}

fn bars_preview_surface(
    root: &WorkbenchView,
    host: BarHost,
    layout: &crate::config::bars::BarLayoutSettings,
    window: &mut Window,
    cx: &mut Context<WorkbenchView>,
) -> Div {
    let appearance = root.theme_runtime();
    let selector = match host {
        BarHost::Window => "bars-editor-window-preview",
        BarHost::Status => "bars-editor-status-preview",
    };
    div()
        .debug_selector(move || selector.to_string())
        .flex()
        .items_center()
        .min_h(appearance.style.controls.button_height)
        .px(gpui::rems(0.5))
        .rounded(appearance.style.radius.compact)
        .border(appearance.style.border.hairline)
        .border_color(appearance.ui.border_variant)
        .bg(appearance.ui.app_background)
        .child(bar_sections_content(
            root.shell_bar_preview_sections(host, layout, window, cx),
            host,
            appearance.style,
        ))
}

fn bar_editor_region_label(region: BarEditorRegion, text: UiText) -> String {
    let (host, position) = match region {
        BarEditorRegion::WindowLeft => (UiTextKey::BarsEditorWindow, UiTextKey::SettingsBarLeft),
        BarEditorRegion::WindowCenter => {
            (UiTextKey::BarsEditorWindow, UiTextKey::SettingsBarCenter)
        }
        BarEditorRegion::WindowRight => (UiTextKey::BarsEditorWindow, UiTextKey::SettingsBarRight),
        BarEditorRegion::StatusLeft => (UiTextKey::BarsEditorStatus, UiTextKey::SettingsBarLeft),
        BarEditorRegion::StatusCenter => {
            (UiTextKey::BarsEditorStatus, UiTextKey::SettingsBarCenter)
        }
        BarEditorRegion::StatusRight => (UiTextKey::BarsEditorStatus, UiTextKey::SettingsBarRight),
    };
    format!("{} {}", text.get(host), text.get(position))
}

fn layout_toml_editor_footer(
    root: &mut WorkbenchView,
    theme: WorkbenchTheme,
    style: UiStyle,
    cx: &mut Context<WorkbenchView>,
) -> Div {
    div()
        .flex_none()
        .flex()
        .items_center()
        .justify_between()
        .px(gpui::rems(0.75))
        .py(gpui::rems(0.375))
        .border_t(style.border.hairline)
        .border_color(theme.border_variant)
        .bg(theme.statusbar_background)
        .child(div().text_xs().text_color(theme.text_muted).child("TOML"))
        .child(
            div()
                .flex()
                .items_center()
                .gap(style.spacing.sm)
                .child(
                    settings_button(
                        "layout-toml-editor-cancel",
                        root.ui_text.get(UiTextKey::Cancel),
                        false,
                        theme,
                        cx,
                        cx.listener(|this, _, _window, cx| {
                            this.cancel_layout_toml_editor();
                            cx.notify();
                        }),
                    )
                    .debug_selector(|| "layout-toml-editor-cancel".to_string()),
                )
                .child(
                    settings_button(
                        "layout-toml-editor-save",
                        root.ui_text.get(UiTextKey::SettingsSave),
                        true,
                        theme,
                        cx,
                        cx.listener(|this, _, _window, cx| {
                            let _ = this.save_layout_toml_editor();
                            cx.notify();
                        }),
                    )
                    .debug_selector(|| "layout-toml-editor-save".to_string()),
                ),
        )
}

pub(super) fn push_component_notification(
    root: Entity<WorkbenchView>,
    event: NotificationEvent,
    item: ToastItem,
    action_label: &'static str,
    theme: WorkbenchTheme,
    ui_style: UiStyle,
    window: &mut Window,
    cx: &mut Context<WorkbenchView>,
) {
    window.push_notification(
        workbench_agent_notification(
            item,
            action_label,
            theme,
            ui_style,
            move |_, _window, cx| {
                root.update(cx, |root, cx| {
                    let _ = root.focus_notification_target(&event);
                    cx.notify();
                });
            },
        ),
        cx,
    );
}
