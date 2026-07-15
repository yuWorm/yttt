use super::*;

impl Render for WorkbenchView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        window.set_rem_size(px(self.app_settings.general.ui_font_size));
        self.ensure_active_project_file_watcher(window, cx);
        self.flush_pending_git_operations(window, cx);
        self.flush_pending_project_tree_loads(window, cx);
        self.flush_pending_document_saves(window, cx);
        self.flush_pending_focus_change_autosaves(window, cx);
        self.flush_pending_file_close_requests(window, cx);
        self.flush_pending_project_close_requests(cx);
        self.sync_input_owner_state();
        let focus_handle = self.workbench_focus_handle(cx);
        let default_active_content_focus_requested = self.onboarding.is_none()
            && !focus_handle.contains_focused(window, cx)
            && self.queue_default_active_work_item_focus();

        let body = if let Some(onboarding) = self.onboarding.as_ref() {
            let command_palette_keybinding =
                self.display_keybinding_for_command(CommandId::CommandPaletteOpen);
            onboarding_view(
                cx,
                onboarding,
                &self.ui_text,
                self.theme_runtime.ui,
                command_palette_keybinding,
            )
        } else if self.workspace.opened_projects().is_empty() {
            empty_workspace(cx, &self.ui_text, &self.theme_runtime.ui)
        } else {
            let tab_items = self.workbench_tab_items(cx);
            let project_panel_visible = self.selected_project_panel_visible();
            let split_view = self.active_work_item_view(window, cx);
            let project_file_panel = project_panel_visible
                .then(|| self.project_file_panel(window, cx))
                .flatten();

            let workbench = div()
                .flex()
                .flex_1()
                .relative()
                .bg(gpui::transparent_black())
                .text_color(self.theme_runtime.ui.text)
                .child({
                    let sidebar = project_sidebar(
                        &self.workspace,
                        self.theme_runtime.ui,
                        self.ui_text,
                        focus_handle.clone(),
                        self.app_settings.project_panel.project_sidebar_width,
                        self.sidebar_collapsed,
                        cx.listener(|this, _, _window, cx| {
                            this.toggle_sidebar();
                            cx.notify();
                        }),
                        |project_id| {
                            let project_id = ProjectId::new(project_id);
                            cx.listener(move |this, _, _window, cx| {
                                let _ = this.select_project(&project_id);
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
                        .child(project_tabs(
                            tab_items,
                            self.theme_runtime.ui,
                            self.icon_theme.clone(),
                            self.ui_text,
                            |work_item| {
                                cx.listener(move |this, event: &ClickEvent, _window, cx| {
                                    let _ = this.handle_work_item_tab_click(
                                        work_item.clone(),
                                        event.click_count(),
                                    );
                                    cx.notify();
                                })
                            },
                            |work_item| {
                                cx.listener(move |this, _event: &MouseDownEvent, _window, cx| {
                                    let _ = this.handle_work_item_tab_click(work_item.clone(), 1);
                                    cx.notify();
                                })
                            },
                            |work_item| {
                                cx.listener(move |this, _event: &ClickEvent, _window, cx| {
                                    cx.stop_propagation();
                                    let _ = this.close_work_item_tab(work_item.clone());
                                    cx.notify();
                                })
                            },
                            |target_index| {
                                cx.listener(move |this, dragged: &WorkItemId, _window, cx| {
                                    let _ = this.move_work_item_tab(dragged, target_index);
                                    cx.notify();
                                })
                            },
                            ProjectTabsToolbar::new(
                                project_panel_visible,
                                self.ui_text.get(if project_panel_visible {
                                    UiTextKey::ProjectFilesHide
                                } else {
                                    UiTextKey::ProjectFilesShow
                                }),
                                cx.listener(|this, _, _window, cx| {
                                    let _ = this.new_tab_from_toolbar();
                                    cx.notify();
                                }),
                                cx.listener(|this, _, _window, cx| {
                                    let _ = this.run_command(CommandId::PaneSplitVertical);
                                    cx.notify();
                                }),
                                cx.listener(|this, _, _window, cx| {
                                    let _ = this.run_command(CommandId::PaneSplitHorizontal);
                                    cx.notify();
                                }),
                                cx.listener(|this, _, _window, cx| {
                                    let _ = this.run_command(CommandId::ProjectPanelToggle);
                                    cx.notify();
                                }),
                            ),
                        ))
                        .child(split_view),
                );
            if let Some(project_file_panel) = project_file_panel {
                workbench.child(project_file_panel)
            } else {
                workbench
            }
        };
        let default_active_content_focus_scheduled = default_active_content_focus_requested
            && match self.active_work_item() {
                Some(WorkItemId::Terminal(_)) => {
                    self.terminal.pending_terminal_focus_pane_id.is_none()
                }
                Some(WorkItemId::File(document_id)) => self
                    .project
                    .pending_editor_focus_document_id
                    .as_ref()
                    .is_none_or(|pending| pending != &document_id),
                None => false,
            };

        let mut root = div()
            .flex()
            .flex_col()
            .size_full()
            .relative()
            .bg(self.theme_runtime.ui.app_background)
            .text_color(self.theme_runtime.ui.text)
            .line_height(relative(self.app_settings.general.ui_line_height))
            .child(workbench_titlebar(
                self.visible_titlebar_info(),
                self.theme_runtime.ui,
                self.ui_text.get(UiTextKey::CommandPaletteOpenTitle),
                self.ui_text.get(UiTextKey::CommandSettingsOpenTitle),
                cx.listener(|this, _, _window, cx| {
                    let _ = this.run_command(CommandId::GitBranchSwitch);
                    cx.notify();
                }),
                cx.listener(|this, _, _window, cx| {
                    let _ = this.run_command(CommandId::GitDiffOpen);
                    cx.notify();
                }),
                cx.listener(|this, _, _window, cx| {
                    let _ = this.run_command(CommandId::CommandPaletteOpen);
                    cx.notify();
                }),
                cx.listener(|this, _, _window, cx| {
                    let _ = this.run_command(CommandId::SettingsOpen);
                    cx.notify();
                }),
            ))
            .child(body);
        if !self.app_settings.general.ui_font_family.is_empty() {
            root = root.font_family(self.app_settings.general.ui_font_family.clone());
        }

        if let Some(active_palette) = self.palette.active_palette.clone() {
            let items = self.palette_items(active_palette.kind);
            if let Some(query_input) = self.palette_query_input(window, cx) {
                root = root.child(palette_overlay(
                    &active_palette,
                    &items,
                    &self.ui_text,
                    &query_input,
                    &self.palette.scroll_handle,
                    self.theme_runtime.ui,
                    |selected_index| {
                        cx.listener(move |this, _, window, cx| {
                            if let Some(active_palette) = &mut this.palette.active_palette {
                                active_palette.selected_index = selected_index;
                            }
                            let _ = this.confirm_palette_selection();
                            this.handle_pending_create_project_request(cx);
                            this.handle_pending_open_project_request(cx);
                            this.flush_pending_status_notifications(window, cx);
                            cx.notify();
                        })
                    },
                ));
            }
        }
        if let Some(error_item) = self.visible_error_notification_item() {
            root = root.child(error_notification_overlay(
                error_item,
                self.theme_runtime.ui,
                cx.listener(|this, _, _window, cx| {
                    this.dismiss_error_notification();
                    cx.notify();
                }),
            ));
        }
        if self.settings.settings_page.is_open {
            if let Some(search_input) = self.settings_search_input(window, cx) {
                root = root.child(settings_overlay(self, &search_input, window, cx));
            }
        }
        if self.overlays.layout_toml_editor.is_some() {
            if let Some(input) = self.layout_toml_input(window, cx) {
                root = root.child(layout_toml_editor_overlay(self, &input, cx));
            }
        }
        if let Some(panel) = self.render_git_diff_panel(window, cx) {
            root = root.child(panel);
        }
        if self.overlays.pending_tab_rename.is_some() {
            if let Some(input) = self.tab_rename_input(window, cx) {
                root = root.child(tab_rename_dialog(
                    cx,
                    &self.ui_text,
                    &input,
                    self.theme_runtime.ui,
                ));
            }
        }
        if self.overlays.pending_keybinding_edit.is_some() {
            if self.overlays.keybinding_recorder_needs_focus {
                focus_handle.focus(window, cx);
                self.overlays.keybinding_recorder_needs_focus = false;
            }
            if let Some(edit) = self.overlays.pending_keybinding_edit.as_ref() {
                root = root.child(keybinding_edit_dialog(
                    cx,
                    &self.ui_text,
                    edit.command,
                    &edit.keys,
                    edit.error.as_deref(),
                    self.theme_runtime.ui,
                ));
            }
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
                self.theme_runtime.ui,
                title,
                details,
                file_intent,
                self.dirty_close_has_save_error(cx),
            ));
        }
        if self.overlays.pending_close_project_id.is_some() {
            root = root.child(close_project_dialog(
                cx,
                &self.ui_text,
                self.theme_runtime.ui,
            ));
        }
        if let Some(conflict) = self.documents.pending_file_conflict.as_ref() {
            root = root.child(file_conflict_dialog(
                cx,
                &self.ui_text,
                self.theme_runtime.ui,
                conflict.document_id.canonical_path.display().to_string(),
                matches!(conflict.current_disk, CurrentDiskState::Missing),
            ));
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

        if self.should_auto_focus_workspace()
            && !focus_handle.contains_focused(window, cx)
            && !default_active_content_focus_scheduled
        {
            focus_handle.focus(window, cx);
        }

        root.track_focus(&focus_handle)
            .key_context(WORKSPACE_CONTEXT)
            .on_key_down(cx.listener(Self::on_key_down))
            .on_mouse_move(cx.listener(Self::on_resize_mouse_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_resize_mouse_up))
            .on_action(cx.listener(Self::on_create_project))
            .on_action(cx.listener(Self::on_open_project))
            .on_action(cx.listener(Self::on_open_command_palette))
            .on_action(cx.listener(Self::on_open_project_palette))
            .on_action(cx.listener(Self::on_opened_project_palette))
            .on_action(cx.listener(Self::on_project_panel_toggle))
            .on_action(cx.listener(Self::on_project_panel_refresh))
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

fn layout_toml_editor_overlay(
    root: &WorkbenchView,
    input: &Entity<InputState>,
    cx: &mut Context<WorkbenchView>,
) -> Div {
    let theme = root.theme_runtime.ui;
    let editor_theme = root.theme_runtime.editor;
    let Some(session) = root.overlays.layout_toml_editor.as_ref() else {
        return div();
    };
    let editor_appearance = session.appearance();
    let editor = session.editor();
    let title = editor.config().title().to_string();
    let path = editor.path().display().to_string();
    let error = editor.error().map(ToOwned::to_owned);

    capture_overlay_input(
        div()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(rgba(0x00000099))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .w(relative(0.72))
                    .max_w(px(1040.))
                    .h(px(680.))
                    .max_h(relative(0.86))
                    .rounded_md()
                    .border_1()
                    .border_color(theme.border_strong)
                    .bg(theme.surface)
                    .text_color(theme.text)
                    .overflow_hidden()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .border_b_1()
                            .border_color(theme.border)
                            .px_5()
                            .py_4()
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap_1()
                                    .min_w_0()
                                    .child(
                                        div()
                                            .text_lg()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child(title),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(theme.text_subtle)
                                            .truncate()
                                            .child(path),
                                    ),
                            )
                            .child(settings_button(
                                "layout-toml-editor-close",
                                root.ui_text.get(UiTextKey::Cancel),
                                false,
                                theme,
                                cx,
                                cx.listener(|this, _, _window, cx| {
                                    this.cancel_layout_toml_editor();
                                    cx.notify();
                                }),
                            )),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_h_0()
                            .p_4()
                            .gap_3()
                            .child(
                                div()
                                    .flex_1()
                                    .min_h_0()
                                    .rounded_sm()
                                    .bg(editor_theme.background)
                                    .overflow_hidden()
                                    .child(
                                        styled_code_editor_input(input, editor_appearance).h_full(),
                                    ),
                            )
                            .when_some(error, |this, error| {
                                this.child(
                                    div()
                                        .rounded_sm()
                                        .border_1()
                                        .border_color(theme.danger)
                                        .bg(theme.surface_elevated)
                                        .px_3()
                                        .py_2()
                                        .text_xs()
                                        .text_color(theme.danger)
                                        .child(error),
                                )
                            }),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_end()
                            .gap_2()
                            .border_t_1()
                            .border_color(theme.border)
                            .px_5()
                            .py_3()
                            .child(settings_button(
                                "layout-toml-editor-cancel",
                                root.ui_text.get(UiTextKey::Cancel),
                                false,
                                theme,
                                cx,
                                cx.listener(|this, _, _window, cx| {
                                    this.cancel_layout_toml_editor();
                                    cx.notify();
                                }),
                            ))
                            .child(settings_button(
                                "layout-toml-editor-save",
                                root.ui_text.get(UiTextKey::SettingsSave),
                                true,
                                theme,
                                cx,
                                cx.listener(|this, _, _window, cx| {
                                    let _ = this.save_layout_toml_editor();
                                    cx.notify();
                                }),
                            )),
                    ),
            ),
    )
}

fn error_notification_overlay<H>(item: ToastItem, theme: WorkbenchTheme, on_close: H) -> Div
where
    H: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
{
    div()
        .absolute()
        .top(px(48.0))
        .right(px(12.0))
        .child(workbench_closable_inline_notification(
            item, theme, on_close,
        ))
}

pub(super) fn push_component_notification(
    root: Entity<WorkbenchView>,
    event: NotificationEvent,
    action_label: &'static str,
    theme: WorkbenchTheme,
    window: &mut Window,
    cx: &mut Context<WorkbenchView>,
) {
    let item = toast_item_for_event(&event);
    let focus_event = event.clone();
    window.push_notification(
        workbench_agent_notification(item, action_label, theme).on_click(move |_, _window, cx| {
            root.update(cx, |root, cx| {
                let _ = root.focus_notification_target(&focus_event);
                cx.notify();
            });
        }),
        cx,
    );
}
