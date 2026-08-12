use gpui_component::{
    Icon,
    menu::{ContextMenuExt as _, PopupMenuItem},
    tooltip::Tooltip,
};

#[derive(Clone, Copy)]
struct AgentSessionTooltipText {
    resume_hint: &'static str,
    session_id_label: &'static str,
    model_label: &'static str,
    transcript_label: &'static str,
}

use super::*;

fn agent_session_field_matches(value: &str, normalized_query: &str) -> bool {
    value.contains(normalized_query) || value.to_lowercase().contains(normalized_query)
}

fn agent_session_matches_search(session: &AgentSession, normalized_query: &str) -> bool {
    normalized_query.is_empty()
        || agent_session_field_matches(&session.title, normalized_query)
        || agent_session_field_matches(&session.id, normalized_query)
        || agent_session_field_matches(session.provider.display_name(), normalized_query)
        || session
            .model
            .as_deref()
            .is_some_and(|model| agent_session_field_matches(model, normalized_query))
        || session.transcript_path.as_ref().is_some_and(|path| {
            agent_session_field_matches(path.to_string_lossy().as_ref(), normalized_query)
        })
}

fn agent_session_tooltip_field(
    selector: &'static str,
    label: &'static str,
    value: String,
    theme: WorkbenchTheme,
    ui_style: UiStyle,
) -> Div {
    div()
        .flex()
        .items_start()
        .min_w_0()
        .w_full()
        .gap(ui_style.spacing.xs)
        .text_xs()
        .child(
            div()
                .flex_none()
                .text_color(theme.text_subtle)
                .child(format!("{label}:")),
        )
        .child(
            div()
                .debug_selector(move || selector.to_string())
                .min_w_0()
                .flex_1()
                .truncate()
                .text_color(theme.text_muted)
                .child(value),
        )
}

impl WorkbenchView {
    fn work_item_view(
        &mut self,
        group_id: TabGroupId,
        item: Option<&WorkItemId>,
        group_active: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        match item {
            Some(WorkItemId::File(document_id)) => {
                self.file_work_item_view(document_id, group_active, window, cx)
            }
            Some(WorkItemId::Terminal(tab_id)) => {
                self.terminal_work_item_view(group_id, tab_id, group_active, window, cx)
            }
            None => div().flex().flex_1(),
        }
    }

    fn file_work_item_view(
        &mut self,
        document_id: &crate::ui::editor::DocumentId,
        group_active: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let document = self
            .project
            .project_editor_runtime
            .document(document_id)
            .cloned();
        if group_active
            && self.project.pending_editor_focus_document_id.as_ref() == Some(document_id)
            && self.foreground_input_owner_kind() == InputOwnerKind::Editor
            && let Some(document) = &document
        {
            let document = document.clone();
            window.defer(cx, move |window, cx| {
                document.update(cx, |document, document_cx| {
                    document.focus(window, document_cx);
                });
            });
            self.project.pending_editor_focus_document_id = None;
        }

        div()
            .debug_selector(|| "active-file-editor".to_string())
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .bg(self.theme_runtime().editor.background)
            .child(div().flex_1().min_h_0().children(document))
    }

    fn terminal_work_item_view(
        &mut self,
        group_id: TabGroupId,
        tab_id: &str,
        group_active: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let Some((project_id, project_path, project_title, tab_title, layout, focused_pane_id)) =
            self.terminal_tab_layout_clone(tab_id)
        else {
            return project_empty_terminal_state(cx, &self.ui_text, &self.theme_runtime().ui);
        };
        let tree_input = RenderTerminalTreeInput {
            group_id,
            group_active,
            project_id: &project_id,
            project_path: &project_path,
            project_title: &project_title,
            tab_id,
            tab_title: &tab_title,
            focused_pane_id: focused_pane_id.as_deref(),
        };

        div()
            .flex()
            .flex_1()
            .text_color(self.theme_runtime().ui.text)
            .child(self.terminal_split_view_for_layout(&layout, &tree_input, window, cx))
    }

    pub(super) fn workbench_tab_items(&self, cx: &Context<Self>) -> Vec<WorkbenchTabItem> {
        let terminal_items = visible_tab_items(&self.workspace);
        let Some(project_id) = self.workspace.selected_project_id() else {
            return Vec::new();
        };
        let Some(project) = self.workspace.project(project_id) else {
            return Vec::new();
        };
        let file_items = self
            .project
            .project_editor_runtime
            .workspace()
            .session(project_id)
            .map(|session| {
                session
                    .file_ids()
                    .iter()
                    .map(|document_id| {
                        let (dirty, missing_on_disk) = self
                            .project
                            .project_editor_runtime
                            .document(document_id)
                            .map(|document| {
                                let document = document.read(cx);
                                (
                                    document.model().is_dirty(),
                                    document.model().is_missing_on_disk(),
                                )
                            })
                            .unwrap_or_default();
                        FileTabSnapshot {
                            id: document_id.clone(),
                            relative_path: project
                                .location
                                .local_path()
                                .and_then(|root| document_id.canonical_path.strip_prefix(root).ok())
                                .unwrap_or(&document_id.canonical_path)
                                .to_path_buf(),
                            dirty,
                            missing_on_disk,
                        }
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let active = self.active_work_item();
        let mut items = merge_work_item_tabs(&terminal_items, &file_items, active.as_ref());
        let terminal_ids = terminal_items
            .iter()
            .map(|item| item.id.clone())
            .collect::<Vec<_>>();
        if let Some(session) = self
            .project
            .project_editor_runtime
            .workspace()
            .session(project_id)
        {
            let order = session.ordered_items(&terminal_ids);
            items.sort_by_key(|item| {
                order
                    .iter()
                    .position(|ordered_id| ordered_id == &item.id)
                    .unwrap_or(usize::MAX)
            });
        }
        items
    }

    pub(super) fn reconcile_selected_work_area(&mut self) {
        let Some((project_id, terminal_ids)) = self.selected_project_work_item_ids() else {
            return;
        };
        if let Some(session) = self
            .project
            .project_editor_runtime
            .workspace_mut()
            .session_mut(&project_id)
        {
            session.reconcile_work_area(&terminal_ids);
        }
    }

    pub(super) fn selected_work_area_snapshot(
        &self,
    ) -> Option<(ProjectId, WorkAreaNode, TabGroupId)> {
        let project_id = self.workspace.selected_project_id()?.clone();
        let session = self
            .project
            .project_editor_runtime
            .workspace()
            .session(&project_id)?;
        Some((
            project_id,
            session.work_area().clone(),
            session.active_group_id(),
        ))
    }

    pub(super) fn work_area_view(
        &mut self,
        project_id: &ProjectId,
        node: &WorkAreaNode,
        active_group_id: TabGroupId,
        all_tab_items: &[WorkbenchTabItem],
        project_panel_visible: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        match node {
            WorkAreaNode::Group(group) => self.work_area_group_view(
                project_id,
                group,
                active_group_id,
                all_tab_items,
                project_panel_visible,
                window,
                cx,
            ),
            WorkAreaNode::Split {
                id,
                axis,
                ratio,
                first,
                second,
            } => {
                let basis = split_child_basis(*ratio);
                let mut container = div().flex().flex_1().min_w_0().min_h_0();
                if *axis == WorkAreaSplitAxis::Column {
                    container = container.flex_col();
                }
                let first = self.work_area_view(
                    project_id,
                    first,
                    active_group_id,
                    all_tab_items,
                    project_panel_visible,
                    window,
                    cx,
                );
                let second = self.work_area_view(
                    project_id,
                    second,
                    active_group_id,
                    all_tab_items,
                    project_panel_visible,
                    window,
                    cx,
                );
                container
                    .child(split_child(first, basis.left))
                    .child(self.work_area_resize_handle(*id, *axis, cx))
                    .child(split_child(second, basis.right))
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn work_area_group_view(
        &mut self,
        project_id: &ProjectId,
        group: &TabGroup,
        active_group_id: TabGroupId,
        all_tab_items: &[WorkbenchTabItem],
        project_panel_visible: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let group_id = group.id();
        let group_active = group_id == active_group_id;
        let mut tab_items = Vec::with_capacity(group.items().len());
        for item_id in group.items() {
            if let Some(mut item) = all_tab_items
                .iter()
                .find(|item| &item.id == item_id)
                .cloned()
            {
                item.state = if group.active_item() == Some(item_id) {
                    crate::ui::components::SelectableState::Active
                } else {
                    crate::ui::components::SelectableState::Inactive
                };
                tab_items.push(item);
            }
        }

        let appearance = self.theme_runtime();
        let drop_group_name =
            SharedString::from(format!("work-area-drop-group-{}", group_id.raw()));
        let target_project_id = project_id.clone();
        let preview_edge = self
            .work_area_drop_target
            .as_ref()
            .filter(|target| target.project_id == *project_id && target.group_id == group_id)
            .and_then(|target| target.edge);
        let mut drop_target = div()
            .debug_selector(move || format!("work-area-drop-preview-{}", group_id.raw()))
            .invisible()
            .absolute()
            .bg(appearance.ui.selection)
            .border(appearance.style.border.emphasized)
            .border_color(appearance.ui.accent)
            .group_drag_over::<DraggedWorkbenchTab>(drop_group_name.clone(), |style| {
                style.visible()
            })
            .can_drop(move |dragged, _, _| {
                dragged
                    .downcast_ref::<DraggedWorkbenchTab>()
                    .is_some_and(|drag| drag.project_id == target_project_id)
            })
            .on_drop(
                cx.listener(move |this, dragged: &DraggedWorkbenchTab, _window, cx| {
                    let _ = this.drop_work_item_on_group(dragged, group_id);
                    cx.stop_propagation();
                    cx.notify();
                }),
            );
        drop_target = match preview_edge {
            None => drop_target.inset_0(),
            Some(WorkAreaDropEdge::Top) => drop_target.top_0().left_0().right_0().h(relative(0.5)),
            Some(WorkAreaDropEdge::Bottom) => {
                drop_target.bottom_0().left_0().right_0().h(relative(0.5))
            }
            Some(WorkAreaDropEdge::Left) => {
                drop_target.top_0().bottom_0().left_0().w(relative(0.5))
            }
            Some(WorkAreaDropEdge::Right) => {
                drop_target.top_0().bottom_0().right_0().w(relative(0.5))
            }
        };

        let active_item = group.active_item().cloned();
        let content = div()
            .debug_selector(move || format!("work-area-group-content-{}", group_id.raw()))
            .flex()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .relative()
            .overflow_hidden()
            .group(drop_group_name)
            .child(self.work_item_view(group_id, active_item.as_ref(), group_active, window, cx))
            .child(drop_target);

        let mut group_view = div()
            .debug_selector(move || format!("work-area-group-{}", group_id.raw()))
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .relative()
            .on_drag_move::<DraggedWorkbenchTab>(cx.listener(
                move |this, event: &DragMoveEvent<DraggedWorkbenchTab>, _window, cx| {
                    this.update_work_area_drop_target(group_id, event, cx);
                },
            ))
            .child(project_tabs(
                project_id.clone(),
                group_id,
                tab_items,
                appearance.ui,
                appearance.style,
                self.icon_theme.clone(),
                self.ui_text,
                |work_item| {
                    cx.listener(move |this, event: &ClickEvent, _window, cx| {
                        let _ =
                            this.handle_work_item_tab_click(work_item.clone(), event.click_count());
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
                    cx.listener(move |this, dragged: &DraggedWorkbenchTab, _window, cx| {
                        let _ = this.move_dragged_work_item_tab(dragged, group_id, target_index);
                        cx.stop_propagation();
                        cx.notify();
                    })
                },
                group_active,
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
            .child(content);
        group_view
            .interactivity()
            .capture_any_mouse_down(cx.listener(move |this, _, _window, cx| {
                let _ = this.activate_work_area_group(group_id);
                cx.notify();
            }));
        group_view
    }

    pub fn selected_project_panel_visible(&self) -> bool {
        let Some(project_id) = self.workspace.selected_project_id() else {
            return false;
        };
        self.project
            .project_editor_runtime
            .workspace()
            .session(project_id)
            .is_some_and(|session| session.project_panel_visible())
    }

    pub(super) fn project_file_panel(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Div> {
        let project_id = self.workspace.selected_project_id()?.clone();
        let tree = self.ensure_project_tree_view(&project_id, window, cx)?;
        let session = self
            .project
            .project_editor_runtime
            .workspace()
            .session(&project_id)?;
        let panel_width = session.project_panel_width();
        let root_load_state = session.file_tree().directory_load_state(Path::new(""));
        let root_is_empty = session.file_tree().visible_rows().is_empty();
        let has_root_snapshot = session.file_tree().has_snapshot(Path::new(""));
        let appearance = self.theme_runtime();
        let theme = appearance.ui;
        let ui_style = appearance.style;
        if self.project.pending_project_tree_focus {
            tree.update(cx, |tree, tree_cx| tree.focus(window, tree_cx));
            self.project.pending_project_tree_focus = false;
        }
        let tree_has_keyboard_focus = tree.read(cx).is_focused(window, cx);
        let tree_is_editing = tree.read(cx).is_editing();
        let active_panel_page = self.project.active_panel_page;

        let files_content = match root_load_state {
            ProjectTreeLoadState::Loading | ProjectTreeLoadState::Unloaded
                if !has_root_snapshot =>
            {
                div()
                    .debug_selector(|| "project-file-panel-loading".to_string())
                    .flex()
                    .flex_1()
                    .items_center()
                    .justify_center()
                    .px(ui_style.spacing.xl)
                    .text_sm()
                    .text_color(theme.text_subtle)
                    .child(self.ui_text.get(UiTextKey::ProjectFilesLoading))
                    .into_any_element()
            }
            ProjectTreeLoadState::Error(error) if !has_root_snapshot => {
                let retry_project_id = project_id.clone();
                div()
                    .debug_selector(|| "project-file-panel-error".to_string())
                    .flex()
                    .flex_col()
                    .flex_1()
                    .items_center()
                    .justify_center()
                    .gap(ui_style.spacing.lg)
                    .px(ui_style.spacing.xl)
                    .text_center()
                    .text_sm()
                    .text_color(theme.text_muted)
                    .child(error)
                    .child(
                        yttt_button(
                            "project-file-panel-retry",
                            self.ui_text.get(UiTextKey::ProjectFilesRetry),
                            YtttButtonVariant::Secondary,
                            theme,
                            ui_style,
                            cx,
                        )
                        .on_click(cx.listener(
                            move |this, _, window, cx| {
                                this.refresh_project_tree(retry_project_id.clone(), window, cx);
                                cx.notify();
                            },
                        )),
                    )
                    .into_any_element()
            }
            ProjectTreeLoadState::Loaded if root_is_empty && !tree_is_editing => {
                let empty_tree = tree.clone();
                let empty_workbench = cx.weak_entity();
                let new_file_label = self.ui_text.get(UiTextKey::ProjectFilesNewFile).to_string();
                let new_directory_label = self
                    .ui_text
                    .get(UiTextKey::ProjectFilesNewDirectory)
                    .to_string();
                let refresh_label = self.ui_text.get(UiTextKey::ProjectFilesRefresh).to_string();
                div()
                    .debug_selector(|| "project-file-panel-empty".to_string())
                    .flex()
                    .flex_1()
                    .items_center()
                    .justify_center()
                    .px(ui_style.spacing.xl)
                    .text_sm()
                    .text_color(theme.text_subtle)
                    .child(self.ui_text.get(UiTextKey::ProjectFilesEmptyDirectory))
                    .context_menu(move |menu, _, _| {
                        let new_file_tree = empty_tree.clone();
                        let new_directory_tree = empty_tree.clone();
                        let refresh_tree = empty_tree.clone();
                        let new_file_workbench = empty_workbench.clone();
                        let new_directory_workbench = empty_workbench.clone();
                        menu.item(PopupMenuItem::new(new_file_label.clone()).on_click(
                            move |_, window, cx| {
                                new_file_tree.update(cx, |tree, tree_cx| {
                                    tree.begin_create_selected(false, window, tree_cx);
                                });
                                let _ = new_file_workbench.update(cx, |_, workbench_cx| {
                                    workbench_cx.notify();
                                });
                            },
                        ))
                        .item(PopupMenuItem::new(new_directory_label.clone()).on_click(
                            move |_, window, cx| {
                                new_directory_tree.update(cx, |tree, tree_cx| {
                                    tree.begin_create_selected(true, window, tree_cx);
                                });
                                let _ = new_directory_workbench.update(cx, |_, workbench_cx| {
                                    workbench_cx.notify();
                                });
                            },
                        ))
                        .item(PopupMenuItem::separator())
                        .item(
                            PopupMenuItem::new(refresh_label.clone()).on_click(move |_, _, cx| {
                                refresh_tree.update(cx, |tree, tree_cx| {
                                    tree.request_refresh(tree_cx);
                                });
                            }),
                        )
                    })
                    .into_any_element()
            }
            _ => div()
                .debug_selector(|| "project-file-tree".to_string())
                .flex()
                .flex_1()
                .overflow_hidden()
                .child(tree)
                .into_any_element(),
        };
        let content = match active_panel_page {
            ProjectPanelPage::Files => div()
                .debug_selector(|| "project-panel-page-files".to_string())
                .flex()
                .flex_1()
                .min_h_0()
                .child(files_content),
            ProjectPanelPage::AgentSessions => {
                let search_input = self.agent_sessions_search_input(window, cx);
                self.agent_sessions_panel_content(&search_input, theme, ui_style, cx)
            }
        };

        let files_tab_workbench = cx.weak_entity();
        let files_tab_tooltip = self.ui_text.get(UiTextKey::ProjectFiles);
        let files_refresh_label = self.ui_text.get(UiTextKey::ProjectFilesRefresh);
        let panel_tab_style =
            yttt_icon_button_style(YtttIconButtonKind::SidebarHeader, theme, ui_style);
        let files_tab = yttt_icon_button(
            "project-panel-tab-files",
            IconName::FolderOpen,
            YtttIconButtonKind::SidebarHeader,
            theme,
            ui_style,
            move |_, _, cx| {
                let _ = files_tab_workbench.update(cx, |workbench, workbench_cx| {
                    workbench.activate_project_panel_page(ProjectPanelPage::Files, workbench_cx);
                });
            },
        )
        .debug_selector(|| "project-panel-tab-files".to_string())
        .when(active_panel_page == ProjectPanelPage::Files, |this| {
            this.bg(theme.ghost_element_selected)
                .text_color(panel_tab_style.active_text)
        })
        .tooltip(move |window, cx| Tooltip::new(files_tab_tooltip).build(window, cx))
        .context_menu(move |menu, _, _| {
            menu.item(PopupMenuItem::new(files_refresh_label).action(Box::new(ProjectPanelRefresh)))
        });
        let agent_sessions_tab = self.agent_sessions_enabled().then(|| {
            let sessions_tab_workbench = cx.weak_entity();
            let sessions_refresh_workbench = sessions_tab_workbench.clone();
            let sessions_tab_tooltip = self.ui_text.get(UiTextKey::AgentSessions);
            let sessions_refresh_label = self.ui_text.get(UiTextKey::AgentSessionsRefresh);
            yttt_icon_button(
                "project-panel-tab-agent-sessions",
                IconName::Bot,
                YtttIconButtonKind::SidebarHeader,
                theme,
                ui_style,
                move |_, _, cx| {
                    let _ = sessions_tab_workbench.update(cx, |workbench, workbench_cx| {
                        workbench.activate_project_panel_page(
                            ProjectPanelPage::AgentSessions,
                            workbench_cx,
                        );
                    });
                },
            )
            .debug_selector(|| "project-panel-tab-agent-sessions".to_string())
            .when(
                active_panel_page == ProjectPanelPage::AgentSessions,
                |this| {
                    this.bg(theme.ghost_element_selected)
                        .text_color(panel_tab_style.active_text)
                },
            )
            .tooltip(move |window, cx| Tooltip::new(sessions_tab_tooltip).build(window, cx))
            .context_menu(move |menu, _, _| {
                let sessions_refresh_workbench = sessions_refresh_workbench.clone();
                menu.item(
                    PopupMenuItem::new(sessions_refresh_label).on_click(move |_, _, cx| {
                        let _ = sessions_refresh_workbench.update(cx, |workbench, workbench_cx| {
                            workbench.refresh_agent_sessions();
                            workbench_cx.notify();
                        });
                    }),
                )
            })
        });
        let placeholder_tab = |id: &'static str, icon: IconName| {
            div()
                .id(id)
                .debug_selector(move || id.to_string())
                .flex()
                .items_center()
                .justify_center()
                .size(panel_tab_style.size)
                .text_color(panel_tab_style.text)
                .child(Icon::new(icon).size(panel_tab_style.icon_size))
        };
        let search_tab_placeholder =
            placeholder_tab("project-panel-tab-search-placeholder", IconName::Search);
        let git_tab_placeholder =
            placeholder_tab("project-panel-tab-git-placeholder", IconName::Network);
        let terminal_tab_placeholder = placeholder_tab(
            "project-panel-tab-terminal-placeholder",
            IconName::SquareTerminal,
        );
        let panel_tab_strip = div()
            .debug_selector(|| "project-panel-tab-strip".to_string())
            .flex()
            .items_center()
            .gap(ui_style.spacing.xs)
            .child(files_tab)
            .when_some(agent_sessions_tab, |strip, tab| strip.child(tab))
            .child(search_tab_placeholder)
            .child(git_tab_placeholder)
            .child(terminal_tab_placeholder);

        let resize_handle = self.sidebar_resize_handle(SidebarSide::Right, cx);
        Some(
            div()
                .debug_selector(|| "project-file-panel".to_string())
                .flex()
                .flex_col()
                .flex_none()
                .relative()
                .h_full()
                .w(px(panel_width))
                .overflow_hidden()
                .bg(theme.panel_background)
                .when(
                    tree_has_keyboard_focus && active_panel_page == ProjectPanelPage::Files,
                    |panel| {
                        panel.child(
                            div()
                                .debug_selector(|| "project-file-panel-focus-indicator".to_string())
                                .absolute()
                                .top(px(6.0))
                                .right(px(6.0))
                                .size(px(5.0))
                                .rounded_full()
                                .bg(theme.accent.alpha(0.72)),
                        )
                    },
                )
                .child(
                    div()
                        .debug_selector(|| "project-panel-tabs".to_string())
                        .flex()
                        .items_center()
                        .justify_center()
                        .w_full()
                        .h(ui_style.icon_buttons.toolbar_size)
                        .flex_none()
                        .border_b(ui_style.border.hairline)
                        .border_color(theme.border_variant)
                        .px(ui_style.spacing.xs)
                        .child(panel_tab_strip),
                )
                .child(content)
                .child(resize_handle),
        )
    }

    fn agent_sessions_panel_content(
        &mut self,
        search_input: &Entity<InputState>,
        theme: WorkbenchTheme,
        ui_style: UiStyle,
        cx: &mut Context<Self>,
    ) -> Div {
        let sessions = self.agent_sessions.sessions.clone();
        let search_query = search_input.read(cx).value().trim().to_lowercase();
        let search_active = !search_query.is_empty();
        let visible_sessions = sessions
            .iter()
            .enumerate()
            .filter(|(_, session)| agent_session_matches_search(session, &search_query))
            .collect::<Vec<_>>();
        let providers = self.agent_session_agents();
        let visible_providers = if search_active {
            providers
                .iter()
                .copied()
                .filter(|provider| {
                    visible_sessions
                        .iter()
                        .any(|(_, session)| session.provider == *provider)
                })
                .collect::<Vec<_>>()
        } else {
            providers.clone()
        };
        let count_label = if search_active {
            format!("{} / {}", visible_sessions.len(), sessions.len())
        } else {
            sessions.len().to_string()
        };
        let header_title = match providers.as_slice() {
            [provider] => provider.display_name(),
            [] => self.primary_agent().display_name(),
            _ => self.ui_text.get(UiTextKey::AgentSessions),
        };
        let header = div()
            .flex()
            .items_center()
            .justify_between()
            .h(ui_style.icon_buttons.toolbar_size)
            .flex_none()
            .border_b(ui_style.border.hairline)
            .border_color(theme.border_variant)
            .px(ui_style.spacing.md)
            .text_xs()
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme.text_muted)
                    .child(header_title),
            )
            .child(div().text_color(theme.text_subtle).child(count_label));
        let remote = self
            .workspace
            .selected_project_id()
            .and_then(|project_id| self.workspace.project(project_id))
            .is_some_and(|project| project.location.local_path().is_none());
        let show_search = !remote
            && !self.agent_sessions.loading
            && self.agent_sessions.error.is_none()
            && !sessions.is_empty();
        let search = div()
            .debug_selector(|| "agent-sessions-search".to_string())
            .flex_none()
            .px(ui_style.spacing.md)
            .pt(ui_style.spacing.md)
            .pb(ui_style.spacing.xs)
            .child(
                div()
                    .debug_selector(|| "agent-sessions-search-input".to_string())
                    .h(ui_style.icon_buttons.toolbar_size)
                    .child(
                        yttt_input(search_input, YtttInputKind::Search, theme, ui_style)
                            .small()
                            .h(ui_style.icon_buttons.toolbar_size)
                            .prefix(IconName::Search)
                            .cleanable(true),
                    ),
            );
        let body = if remote {
            self.agent_sessions_message(
                "agent-sessions-remote",
                self.ui_text
                    .get(UiTextKey::AgentSessionsRemoteUnavailable)
                    .to_string(),
                theme,
                ui_style,
            )
        } else if self.agent_sessions.loading {
            self.agent_sessions_message(
                "agent-sessions-loading",
                self.ui_text
                    .get(UiTextKey::AgentSessionsLoading)
                    .to_string(),
                theme,
                ui_style,
            )
        } else if let Some(error) = self.agent_sessions.error.clone() {
            div()
                .debug_selector(|| "agent-sessions-error".to_string())
                .flex()
                .flex_col()
                .flex_1()
                .items_center()
                .justify_center()
                .gap(ui_style.spacing.lg)
                .px(ui_style.spacing.xl)
                .text_center()
                .text_sm()
                .text_color(theme.text_muted)
                .child(error)
                .child(
                    yttt_button(
                        "agent-sessions-retry",
                        self.ui_text.get(UiTextKey::ProjectFilesRetry),
                        YtttButtonVariant::Secondary,
                        theme,
                        ui_style,
                        cx,
                    )
                    .on_click(cx.listener(|this, _, _window, cx| {
                        this.refresh_agent_sessions();
                        cx.notify();
                    })),
                )
        } else if self.agent_sessions.sessions.is_empty() {
            self.agent_sessions_message(
                "agent-sessions-empty",
                self.ui_text.get(UiTextKey::AgentSessionsEmpty).to_string(),
                theme,
                ui_style,
            )
        } else if visible_sessions.is_empty() {
            self.agent_sessions_message(
                "agent-sessions-no-matches",
                self.ui_text
                    .get(UiTextKey::AgentSessionsNoMatches)
                    .to_string(),
                theme,
                ui_style,
            )
        } else {
            let tooltip_text = AgentSessionTooltipText {
                resume_hint: self.ui_text.get(UiTextKey::AgentSessionsResumeHint),
                session_id_label: self.ui_text.get(UiTextKey::AgentSessionsSessionId),
                model_label: self.ui_text.get(UiTextKey::AgentSessionsModel),
                transcript_label: self.ui_text.get(UiTextKey::AgentSessionsTranscript),
            };
            let mut list = div()
                .flex()
                .flex_col()
                .size_full()
                .overflow_y_scrollbar()
                .py(ui_style.spacing.xs);
            if providers.len() == 1 {
                let rows = visible_sessions
                    .iter()
                    .map(|(index, session)| {
                        self.agent_session_row(
                            *index,
                            session,
                            true,
                            false,
                            theme,
                            ui_style,
                            tooltip_text,
                            cx,
                        )
                    })
                    .collect::<Vec<_>>();
                list = list.children(rows);
            } else {
                for provider in visible_providers {
                    let provider_id = provider.id();
                    let expanded = search_active
                        || self.agent_sessions.expanded_providers.contains(provider_id);
                    let provider_session_count = visible_sessions
                        .iter()
                        .filter(|(_, session)| session.provider == provider)
                        .count();
                    let rows = if expanded {
                        visible_sessions
                            .iter()
                            .filter(|(_, session)| session.provider == provider)
                            .map(|(index, session)| {
                                self.agent_session_row(
                                    *index,
                                    session,
                                    false,
                                    true,
                                    theme,
                                    ui_style,
                                    tooltip_text,
                                    cx,
                                )
                            })
                            .collect::<Vec<_>>()
                    } else {
                        Vec::new()
                    };
                    let group_header = div()
                        .id(SharedString::from(format!(
                            "agent-session-provider-{provider_id}"
                        )))
                        .debug_selector(move || format!("agent-session-provider-{provider_id}"))
                        .cursor_pointer()
                        .flex()
                        .items_center()
                        .h(px(30.0))
                        .gap(ui_style.spacing.xs)
                        .mx(ui_style.spacing.xs)
                        .px(ui_style.spacing.sm)
                        .rounded_sm()
                        .hover(move |style| style.bg(theme.hover_surface))
                        .on_click(cx.listener(move |this, _, _window, cx| {
                            if !this.agent_sessions.expanded_providers.remove(provider_id) {
                                this.agent_sessions.expanded_providers.insert(provider_id);
                            }
                            cx.notify();
                        }))
                        .child(
                            Icon::new(if expanded {
                                IconName::ChevronDown
                            } else {
                                IconName::ChevronRight
                            })
                            .size(px(12.0))
                            .text_color(theme.text_subtle),
                        )
                        .child(agent_type_icon(
                            format!("agent-session-provider-icon-{provider_id}").into(),
                            provider_id,
                            theme,
                        ))
                        .child(
                            div()
                                .min_w_0()
                                .flex_1()
                                .truncate()
                                .text_sm()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(theme.text_muted)
                                .child(provider.display_name()),
                        )
                        .child(
                            div()
                                .flex_none()
                                .text_xs()
                                .text_color(theme.text_subtle)
                                .child(provider_session_count.to_string()),
                        );
                    list = list.child(
                        div()
                            .flex()
                            .flex_col()
                            .child(group_header)
                            .when(expanded, |group| group.children(rows)),
                    );
                }
            }
            div()
                .debug_selector(|| "agent-sessions-list".to_string())
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .child(list)
        };

        div()
            .debug_selector(|| "project-panel-page-agent-sessions".to_string())
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .child(header)
            .when(show_search, |panel| panel.child(search))
            .child(body)
    }

    fn agent_session_row(
        &self,
        index: usize,
        session: &AgentSession,
        show_provider_icon: bool,
        inset: bool,
        theme: WorkbenchTheme,
        ui_style: UiStyle,
        tooltip_text: AgentSessionTooltipText,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<Div> {
        let title = self.agent_session_title(session);
        let tooltip_title = title.clone();
        let age = self.agent_session_age_label(session.updated_at_ms);
        let provider = session.provider.display_name();
        let tooltip_meta = if age.is_empty() {
            provider.to_string()
        } else {
            format!("{provider} · {age}")
        };
        let session_id = session.id.clone();
        let model = session.model.clone();
        let transcript = session
            .transcript_path
            .as_ref()
            .and_then(|path| path.file_name())
            .map(|name| name.to_string_lossy().into_owned());
        div()
            .id(("agent-session-row", index))
            .debug_selector(move || format!("agent-session-row-{index}"))
            .cursor_pointer()
            .flex()
            .items_center()
            .gap(ui_style.spacing.sm)
            .mx(ui_style.spacing.xs)
            .when(inset, |row| row.ml(ui_style.spacing.xl))
            .px(ui_style.spacing.sm)
            .py(ui_style.spacing.sm)
            .rounded_sm()
            .hover(move |style| style.bg(theme.hover_surface))
            .on_click(cx.listener(move |this, event: &ClickEvent, _window, cx| {
                if event.click_count() >= 2 {
                    if let Err(error) = this.resume_agent_session(index) {
                        this.load_error = Some(error);
                    }
                    cx.notify();
                }
            }))
            .tooltip(move |window, cx| {
                let title = tooltip_title.clone();
                let meta = tooltip_meta.clone();
                let session_id = session_id.clone();
                let model = model.clone();
                let transcript = transcript.clone();
                Tooltip::element(move |_, _| {
                    div()
                        .debug_selector(|| "agent-session-tooltip".to_string())
                        .flex()
                        .flex_col()
                        .min_w_0()
                        .w(px(420.0))
                        .overflow_hidden()
                        .gap(ui_style.spacing.xs)
                        .child(
                            div()
                                .debug_selector(|| "agent-session-tooltip-title".to_string())
                                .min_w_0()
                                .w_full()
                                .whitespace_normal()
                                .text_sm()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(theme.text)
                                .child(title.clone()),
                        )
                        .child(
                            div()
                                .min_w_0()
                                .w_full()
                                .truncate()
                                .text_xs()
                                .text_color(theme.text_subtle)
                                .child(meta.clone()),
                        )
                        .child(agent_session_tooltip_field(
                            "agent-session-tooltip-session-id",
                            tooltip_text.session_id_label,
                            session_id.clone(),
                            theme,
                            ui_style,
                        ))
                        .when_some(model.clone(), |tooltip, model| {
                            tooltip.child(agent_session_tooltip_field(
                                "agent-session-tooltip-model",
                                tooltip_text.model_label,
                                model,
                                theme,
                                ui_style,
                            ))
                        })
                        .when_some(transcript.clone(), |tooltip, transcript| {
                            tooltip.child(agent_session_tooltip_field(
                                "agent-session-tooltip-transcript",
                                tooltip_text.transcript_label,
                                transcript,
                                theme,
                                ui_style,
                            ))
                        })
                        .child(
                            div()
                                .pt(ui_style.spacing.xs)
                                .border_t(ui_style.border.hairline)
                                .border_color(theme.border_variant)
                                .min_w_0()
                                .w_full()
                                .whitespace_normal()
                                .text_xs()
                                .text_color(theme.text_subtle)
                                .child(tooltip_text.resume_hint),
                        )
                })
                .build(window, cx)
            })
            .when(show_provider_icon, |row| {
                row.child(agent_type_icon(
                    format!("agent-session-provider-icon-{index}").into(),
                    session.provider.id(),
                    theme,
                ))
            })
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .truncate()
                    .text_sm()
                    .text_color(theme.text)
                    .child(title),
            )
            .when(!age.is_empty(), |row| {
                row.child(
                    div()
                        .flex_none()
                        .text_xs()
                        .text_color(theme.text_subtle)
                        .child(age),
                )
            })
    }

    fn agent_sessions_message(
        &self,
        selector: &'static str,
        message: String,
        theme: WorkbenchTheme,
        ui_style: UiStyle,
    ) -> Div {
        div()
            .debug_selector(move || selector.to_string())
            .flex()
            .flex_1()
            .items_center()
            .justify_center()
            .px(ui_style.spacing.xl)
            .text_center()
            .text_sm()
            .text_color(theme.text_subtle)
            .child(message)
    }

    pub(super) fn terminal_split_view_for_layout(
        &mut self,
        layout: &LayoutNode,
        tree_input: &RenderTerminalTreeInput<'_>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        match layout {
            LayoutNode::Pane(pane) => self.render_terminal_pane(
                RenderTerminalPaneInput {
                    group_id: tree_input.group_id,
                    project_id: tree_input.project_id,
                    project_path: tree_input.project_path,
                    project_title: tree_input.project_title,
                    pane,
                    tab_id: tree_input.tab_id,
                    tab_title: tree_input.tab_title,
                    is_focused: tree_input.group_active
                        && tree_input.focused_pane_id == Some(pane.id.as_str()),
                },
                window,
                cx,
            ),
            LayoutNode::Split(split) => {
                let basis = split_child_basis(split.ratio);
                let mut container = div().flex().flex_1();
                if split.direction == SplitDirection::Vertical {
                    container = container.flex_col();
                }

                let left = self.terminal_split_view_for_layout(&split.left, tree_input, window, cx);
                let right =
                    self.terminal_split_view_for_layout(&split.right, tree_input, window, cx);

                container
                    .child(split_child(left, basis.left))
                    .child(self.split_resize_handle(split.direction, cx))
                    .child(split_child(right, basis.right))
            }
        }
    }

    pub(super) fn selected_tab_layout_clone(
        &self,
    ) -> Option<(String, PathBuf, String, String, String, LayoutNode)> {
        let selected_project_id = self.workspace.selected_project_id()?;
        let project = self.workspace.project(selected_project_id)?;
        let tab = project
            .layout
            .tabs
            .iter()
            .find(|tab| tab.id == project.selected_tab_id)?;

        let project_path = match &project.location {
            ProjectLocation::Local { path } => path.clone(),
            ProjectLocation::Ssh { root, .. } => PathBuf::from(root.as_str()),
        };
        Some((
            selected_project_id.as_str().to_string(),
            tab.cwd.clone().unwrap_or(project_path),
            project.layout.project.name.clone(),
            project.selected_tab_id.clone(),
            tab.title.clone(),
            tab.layout.clone(),
        ))
    }

    fn terminal_tab_layout_clone(
        &self,
        tab_id: &str,
    ) -> Option<(String, PathBuf, String, String, LayoutNode, Option<String>)> {
        let selected_project_id = self.workspace.selected_project_id()?;
        let project = self.workspace.project(selected_project_id)?;
        let tab = project.layout.tabs.iter().find(|tab| tab.id == tab_id)?;
        let project_path = match &project.location {
            ProjectLocation::Local { path } => path.clone(),
            ProjectLocation::Ssh { root, .. } => PathBuf::from(root.as_str()),
        };
        let focused_pane_id = project
            .tab_state(tab_id)
            .and_then(|state| state.focused_pane_id.clone());
        Some((
            selected_project_id.as_str().to_string(),
            tab.cwd.clone().unwrap_or(project_path),
            project.layout.project.name.clone(),
            tab.title.clone(),
            tab.layout.clone(),
            focused_pane_id,
        ))
    }

    fn pending_eager_terminal_pane_contexts(&self) -> Vec<TerminalPaneContext> {
        let mut contexts = Vec::new();
        let shell = self.resolved_terminal_shell();
        for project in self.workspace.opened_projects() {
            let project_path = match &project.location {
                ProjectLocation::Local { path } => path.clone(),
                ProjectLocation::Ssh { root, .. } => PathBuf::from(root.as_str()),
            };
            for tab in &project.layout.tabs {
                if !tab.startup.is_eager()
                    || !self.layout_has_uninitialized_terminal_pane(
                        project.id.as_str(),
                        &tab.id,
                        &tab.layout,
                    )
                {
                    continue;
                }
                collect_terminal_pane_contexts(
                    project.id.as_str(),
                    &project_path,
                    &project.layout.project.name,
                    &tab.id,
                    &tab.title,
                    &shell,
                    &self.terminal.environment,
                    &tab.layout,
                    None,
                    &self.terminal.terminal_input_gate,
                    &mut contexts,
                );
            }
        }
        contexts.retain(|context| {
            let key = terminal_pane_key(&context.project_id, &context.tab_id, &context.pane.id);
            !self.terminal.terminal_panes.contains_key(&key)
        });
        contexts
    }

    fn layout_has_uninitialized_terminal_pane(
        &self,
        project_id: &str,
        tab_id: &str,
        layout: &LayoutNode,
    ) -> bool {
        match layout {
            LayoutNode::Pane(pane) => {
                let key = terminal_pane_key(project_id, tab_id, &pane.id);
                !self.terminal.terminal_panes.contains_key(&key)
            }
            LayoutNode::Split(split) => {
                self.layout_has_uninitialized_terminal_pane(project_id, tab_id, &split.left)
                    || self.layout_has_uninitialized_terminal_pane(project_id, tab_id, &split.right)
            }
        }
    }

    pub(super) fn ensure_eager_terminal_panes(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        for context in self.pending_eager_terminal_pane_contexts() {
            self.ensure_terminal_pane(context, window, cx);
        }
    }

    fn ensure_terminal_pane(
        &mut self,
        mut context: TerminalPaneContext,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<TerminalPaneView> {
        let key = terminal_pane_key(&context.project_id, &context.tab_id, &context.pane.id);
        if let Some(pane_view) = self.terminal.terminal_panes.get(&key) {
            return pane_view.clone();
        }
        let project_id = ProjectId::new(&context.project_id);
        context.ssh =
            self.workspace
                .project(&project_id)
                .and_then(|project| match &project.location {
                    ProjectLocation::Ssh { connection_id, .. } => Some(SshTerminalContext {
                        connection_id: connection_id.clone(),
                    }),
                    ProjectLocation::Local { .. } => None,
                });
        context.agent_hook_client = if context.ssh.is_none() {
            self.agent_manager.hook_client()
        } else {
            None
        };
        let agent_address =
            AgentPaneAddress::new(&context.project_id, &context.tab_id, &context.pane.id);
        if let Some((launch, snapshot)) = self.agent_manager.prepare_pane(
            agent_address.clone(),
            &context.pane.command,
            context.ssh.is_some(),
        ) {
            context.agent_launch = Some(launch);
            if let Err(error) = self.record_agent_runtime_snapshot(agent_address.clone(), snapshot)
            {
                self.load_error = Some(error.to_string());
            }
        } else if !self.agent_manager.has_retained_snapshot(&agent_address)
            && let Err(error) = self.workspace.clear_agent_snapshot(
                &project_id,
                &agent_address.tab_id,
                &agent_address.pane_id,
            )
        {
            self.load_error = Some(error.to_string());
        }

        let project_id = context.project_id.clone();
        let tab_id = context.tab_id.clone();
        let pane_id = context.pane.id.clone();
        let terminal_config = self.theme_runtime().to_terminal_config();
        let theme = self.theme_runtime().ui;
        let start_processes = self.terminal.start_processes;
        let pane_view = if start_processes {
            cx.new(|_| TerminalPaneView::new_deferred(context, terminal_config, theme))
        } else {
            cx.new(|cx| {
                TerminalPaneView::new_without_processes(context, terminal_config, theme, cx)
            })
        };
        let subscription = cx.subscribe_in(&pane_view, window, Self::on_terminal_pane_event);
        self.terminal
            .terminal_pane_subscriptions
            .insert(key.clone(), subscription);
        self.terminal.terminal_panes.insert(key, pane_view.clone());
        if start_processes {
            pane_view.update(cx, |pane, cx| {
                pane.start_terminal(cx);
            });
        } else {
            if let Err(error) =
                self.workspace
                    .mark_pane_running(&ProjectId::new(&project_id), &tab_id, &pane_id)
            {
                self.load_error = Some(error.to_string());
            }
            let running_agent = (
                pane_view.read(cx).agent_instance_id().cloned(),
                pane_view.read(cx).generation(),
            );
            if let (Some(instance_id), generation) = running_agent
                && let Some((address, snapshot)) =
                    self.agent_manager.process_started(&instance_id, generation)
                && let Err(error) = self.record_agent_runtime_snapshot(address, snapshot)
            {
                self.load_error = Some(error.to_string());
            }
        }
        self.sync_agent_process_monitoring(window, cx);
        pane_view
    }

    pub(super) fn render_terminal_pane(
        &mut self,
        input: RenderTerminalPaneInput<'_>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let context = TerminalPaneContext {
            project_id: input.project_id.to_string(),
            project_path: input.project_path.to_path_buf(),
            project_title: input.project_title.to_string(),
            tab_id: input.tab_id.to_string(),
            tab_title: input.tab_title.to_string(),
            pane: input.pane.clone(),
            shell: self.resolved_terminal_shell(),
            environment: self.terminal.environment.clone(),
            is_focused: input.is_focused,
            terminal_input_gate: self.terminal.terminal_input_gate.clone(),
            ssh: None,
            agent_launch: None,
            agent_hook_client: None,
        };
        let pane_view = self.ensure_terminal_pane(context, window, cx);

        let pane_id = input.pane.id.clone();
        let pending_focus_matches =
            self.terminal
                .pending_terminal_focus
                .as_ref()
                .is_some_and(|target| {
                    target.project_id.as_str() == input.project_id
                        && target.tab_id == input.tab_id
                        && target.pane_id == pane_id
                });
        if pending_focus_matches
            && self.should_auto_focus_workspace()
            && pane_view.update(cx, |pane, cx| pane.focus_terminal(window, cx))
        {
            self.terminal.pending_terminal_focus = None;
        }
        let terminal_has_keyboard_focus = self.vim.surface() == WorkbenchSurface::Terminal
            && pane_view.read(cx).terminal_is_focused(window, cx);

        let appearance = self.theme_runtime();
        let focus_indicator_pane_id = pane_id.clone();
        let terminal_input_allowed = self.terminal_input_allowed();
        let mut wrapper = div().flex().flex_1().relative();
        let group_id = input.group_id;
        let project_id = ProjectId::new(input.project_id);
        let tab_id = input.tab_id.to_string();
        let focused_pane_id = pane_id.clone();
        wrapper.interactivity().on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _window, cx| {
                if !this.terminal_input_allowed() {
                    cx.stop_propagation();
                    return;
                }
                let _ = this.focus_work_area_terminal_pane(
                    group_id,
                    &project_id,
                    &tab_id,
                    &focused_pane_id,
                );
                cx.notify();
            }),
        );
        wrapper = wrapper
            .child(pane_view)
            .when(terminal_has_keyboard_focus, |pane| {
                pane.child(
                    div()
                        .debug_selector(move || {
                            format!("terminal-pane-focus-indicator-{focus_indicator_pane_id}")
                        })
                        .absolute()
                        .top(px(6.0))
                        .right(px(6.0))
                        .size(px(5.0))
                        .rounded_full()
                        .bg(appearance.ui.accent.alpha(0.72)),
                )
            });
        if !terminal_input_allowed {
            let project_id = ProjectId::new(input.project_id);
            let tab_id = input.tab_id.to_string();
            wrapper = wrapper.child(
                div()
                    .absolute()
                    .inset_0()
                    .bg(rgba(0x00000000))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _event, _window, cx| {
                            if matches!(
                                this.foreground_input_owner_kind(),
                                InputOwnerKind::Workspace | InputOwnerKind::Editor
                            ) {
                                let _ = this.focus_work_area_terminal_pane(
                                    group_id,
                                    &project_id,
                                    &tab_id,
                                    &pane_id,
                                );
                                cx.notify();
                            }
                            cx.stop_propagation();
                        }),
                    ),
            );
        }
        wrapper
    }

    pub(super) fn prune_terminal_panes(&mut self) {
        let mut live_keys = HashSet::new();
        for project in self.workspace.opened_projects() {
            for tab in &project.layout.tabs {
                collect_terminal_pane_keys(
                    project.id.as_str(),
                    &tab.id,
                    &tab.layout,
                    &mut live_keys,
                );
            }
        }

        self.terminal
            .terminal_panes
            .retain(|key, _pane| live_keys.contains(key));
        self.terminal
            .terminal_pane_subscriptions
            .retain(|key, _subscription| live_keys.contains(key));
    }

    pub(super) fn update_terminal_agent_title(
        &mut self,
        address: &AgentPaneAddress,
        provider_id: &str,
        title: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        let key = terminal_pane_key(&address.project_id, &address.tab_id, &address.pane_id);
        let Some(pane) = self.terminal.terminal_panes.get(&key).cloned() else {
            return;
        };
        let provider_display_name = BuiltinAgent::ALL
            .into_iter()
            .find(|agent| agent.id() == provider_id)
            .map(BuiltinAgent::display_name)
            .unwrap_or(provider_id);
        pane.update(cx, |pane, cx| {
            pane.set_agent_session_title(provider_display_name, title, cx);
        });
    }

    pub(super) fn record_agent_runtime_snapshot(
        &mut self,
        address: AgentPaneAddress,
        snapshot: AgentSnapshot,
    ) -> Result<(), WorkspaceError> {
        let result = self.workspace.record_agent_snapshot(
            &ProjectId::new(&address.project_id),
            &address.tab_id,
            &address.pane_id,
            snapshot,
        );
        if let Some(error) = self.agent_manager.take_error() {
            self.load_error = combine_load_messages(self.load_error.take(), Some(error));
        }
        result
    }
    pub(super) fn record_agent_event_snapshot(
        &mut self,
        address: AgentPaneAddress,
        snapshot: AgentSnapshot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), WorkspaceError> {
        let title = snapshot
            .session
            .as_ref()
            .and_then(|session| session.title.as_deref());
        self.update_terminal_agent_title(&address, snapshot.provider_id.as_str(), title, cx);
        let notification = self.agent_transition_notification(&address, &snapshot);
        let result = self.record_agent_runtime_snapshot(address, snapshot);
        if result.is_ok()
            && let Some(notification) = notification
        {
            self.present_notification(notification, window, cx);
        }
        result
    }

    pub(super) fn agent_transition_notification(
        &self,
        address: &AgentPaneAddress,
        snapshot: &AgentSnapshot,
    ) -> Option<NotificationEvent> {
        let project_id = ProjectId::new(&address.project_id);
        let project = self.workspace.project(&project_id)?;
        let tab = project.layout.tab(&address.tab_id)?;
        let pane = tab.layout.find_pane(&address.pane_id)?;
        notification_for_agent_transition(AgentTransitionNotificationInput {
            previous_state: project
                .tab_state(&address.tab_id)
                .and_then(|tab| {
                    tab.pane_states
                        .iter()
                        .find(|pane| pane.pane_id == address.pane_id)
                })
                .and_then(|pane| pane.agent_snapshot.as_ref())
                .map(AgentSnapshot::view_state),
            state: snapshot.view_state(),
            project_id: address.project_id.clone(),
            tab_id: address.tab_id.clone(),
            pane_id: address.pane_id.clone(),
            project_title: project.location.fallback_title(),
            tab_title: tab.title.clone(),
            pane_title: pane.title.clone(),
            summary: snapshot
                .session
                .as_ref()
                .and_then(|session| session.title.clone())
                .filter(|title| !title.trim().is_empty()),
        })
    }

    fn notification_matches_current_agent_state(&self, event: &NotificationEvent) -> bool {
        let project_id = ProjectId::new(&event.project_id);
        let state = self
            .workspace
            .project(&project_id)
            .and_then(|project| project.tab_state(&event.tab_id))
            .and_then(|tab| {
                tab.pane_states
                    .iter()
                    .find(|pane| pane.pane_id == event.pane_id)
            })
            .and_then(|pane| pane.agent_snapshot.as_ref())
            .map(AgentSnapshot::view_state);
        matches!(
            (event.kind, state),
            (
                NotificationKind::AgentWaiting,
                Some(AgentViewState::Waiting)
            ) | (
                NotificationKind::AgentCompleted | NotificationKind::AgentFailed,
                Some(
                    AgentViewState::Completed
                        | AgentViewState::Failed
                        | AgentViewState::Interrupted
                )
            )
        )
    }

    pub(super) fn present_notification(
        &mut self,
        event: NotificationEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let root = cx.entity();
        let action_label = self.ui_text.get(UiTextKey::OpenNotificationTarget);
        let appearance = self.theme_runtime();
        let theme = appearance.ui;
        let ui_style = appearance.style;
        self.handle_terminal_notification(event.clone());
        let item = toast_item_for_event(&event, &self.ui_text);
        push_component_notification(root, event, item, action_label, theme, ui_style, window, cx);
        cx.notify();
    }

    pub(super) fn on_terminal_pane_event(
        &mut self,
        pane: &Entity<TerminalPaneView>,
        event: &TerminalPaneEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            TerminalPaneEvent::Notification(event) => {
                if !self.notification_matches_current_agent_state(event) {
                    self.present_notification(event.clone(), window, cx);
                }
            }
            TerminalPaneEvent::Started(event) => {
                if let Err(error) = self.handle_terminal_pane_started(event.clone()) {
                    self.load_error = Some(error.to_string());
                }
                if let Some(instance_id) = &event.agent_instance_id
                    && let Some((address, snapshot)) = self
                        .agent_manager
                        .process_started(instance_id, event.generation)
                    && let Err(error) = self.record_agent_runtime_snapshot(address, snapshot)
                {
                    self.load_error = Some(error.to_string());
                }
                cx.notify();
            }
            TerminalPaneEvent::StartFailed(event) => {
                self.load_error = Some(event.message.clone());
                if let Some(instance_id) = &event.agent_instance_id
                    && let Some(address) = self
                        .agent_manager
                        .process_start_failed(instance_id, event.generation)
                    && let Err(error) = self.workspace.clear_agent_snapshot(
                        &ProjectId::new(&address.project_id),
                        &address.tab_id,
                        &address.pane_id,
                    )
                {
                    self.load_error =
                        combine_load_messages(self.load_error.take(), Some(error.to_string()));
                }
                if let Some(error) = self.agent_manager.take_error() {
                    self.load_error = combine_load_messages(self.load_error.take(), Some(error));
                }
                cx.notify();
            }
            TerminalPaneEvent::IoError { message, fatal, .. } => {
                self.load_error = Some(message.clone());
                if *fatal {
                    let pane = pane.clone();
                    cx.defer_in(window, move |_, _window, cx| {
                        pane.update(cx, |pane, cx| {
                            pane.terminate_after_fatal_io(cx);
                        });
                    });
                }
                cx.notify();
            }
            TerminalPaneEvent::AgentStatusFrame { frame, .. } => {
                if let Ok(Some((address, snapshot))) = self.agent_manager.ingest_title(frame)
                    && let Err(error) =
                        self.record_agent_event_snapshot(address, snapshot, window, cx)
                {
                    self.load_error = Some(error.to_string());
                }
                cx.notify();
            }
            TerminalPaneEvent::TitleChanged { .. } => {
                cx.notify();
            }
            TerminalPaneEvent::Exited(event) => {
                let reason = match event.exit_reason {
                    yttt_terminal::ExitReason::Completed => AgentExitReason::Completed,
                    yttt_terminal::ExitReason::Failed => AgentExitReason::Failed,
                    yttt_terminal::ExitReason::KilledByUser => AgentExitReason::KilledByUser,
                };
                if let Some(instance_id) = &event.agent_instance_id {
                    let code = match event.status {
                        yttt_terminal::ProcessStatus::Running => None,
                        yttt_terminal::ProcessStatus::Exited { code } => code,
                    };
                    let exit = AgentProcessExit { code, reason };
                    match self
                        .agent_manager
                        .process_exited(instance_id, event.generation, exit)
                    {
                        Some(AgentPaneExitOutcome::Snapshot { address, snapshot }) => {
                            let result = if snapshot.view_state() == AgentViewState::Failed {
                                self.workspace.clear_agent_snapshot(
                                    &ProjectId::new(&address.project_id),
                                    &address.tab_id,
                                    &address.pane_id,
                                )
                            } else {
                                self.record_agent_runtime_snapshot(address, snapshot)
                            };
                            if let Err(error) = result {
                                self.load_error = Some(error.to_string());
                            }
                        }
                        Some(AgentPaneExitOutcome::ResumeFailed { address }) => {
                            let project_id = ProjectId::new(&address.project_id);
                            if let Err(error) = self.workspace.clear_agent_snapshot(
                                &project_id,
                                &address.tab_id,
                                &address.pane_id,
                            ) {
                                self.load_error = Some(error.to_string());
                            }
                            let key = terminal_pane_key(
                                &address.project_id,
                                &address.tab_id,
                                &address.pane_id,
                            );
                            self.terminal.terminal_panes.remove(&key);
                            self.terminal.terminal_pane_subscriptions.remove(&key);
                            self.terminal.agent_process_observations.remove(&address);
                        }
                        None => {}
                    }
                } else {
                    let address =
                        AgentPaneAddress::new(&event.project_id, &event.tab_id, &event.pane_id);
                    if self
                        .terminal
                        .agent_process_observations
                        .get(&address)
                        .is_some_and(|observation| observation.generation == event.generation)
                    {
                        self.terminal.agent_process_observations.remove(&address);
                        self.finish_detected_agent(&address, event.generation, reason, window, cx);
                    }
                }
                match self.handle_terminal_pane_exit(event.clone()) {
                    Ok(PaneExitCloseOutcome::PaneKept) => {}
                    Ok(_) => {
                        let address =
                            AgentPaneAddress::new(&event.project_id, &event.tab_id, &event.pane_id);
                        self.terminal.agent_process_observations.remove(&address);
                        self.agent_manager.forget_pane(&address);
                    }
                    Err(error) => {
                        self.load_error = Some(error.to_string());
                    }
                }
                cx.notify();
            }
        }
    }
}
