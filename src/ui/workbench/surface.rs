use gpui_component::menu::{DropdownMenu as _, PopupMenuItem};

use super::*;

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
                    .map(|document_id| FileTabSnapshot {
                        id: document_id.clone(),
                        relative_path: project
                            .location
                            .local_path()
                            .and_then(|root| document_id.canonical_path.strip_prefix(root).ok())
                            .unwrap_or(&document_id.canonical_path)
                            .to_path_buf(),
                        dirty: self
                            .project
                            .project_editor_runtime
                            .document(document_id)
                            .is_some_and(|document| document.read(cx).model().is_dirty()),
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
            .on_drag_move::<DraggedWorkbenchTab>(cx.listener(
                move |this, event: &DragMoveEvent<DraggedWorkbenchTab>, _window, cx| {
                    this.update_work_area_drop_target(group_id, event, cx);
                },
            ))
            .border(appearance.style.border.hairline)
            .border_color(if group_active {
                appearance.ui.accent
            } else {
                appearance.ui.border
            })
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
        let project_name = self
            .workspace
            .project(&project_id)?
            .layout
            .project
            .name
            .clone();
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
        let tree_is_editing = tree.read(cx).is_editing();
        let new_entry_tree = tree.clone();
        let workbench_for_new_entry = cx.weak_entity();
        let new_file_label = self.ui_text.get(UiTextKey::ProjectFilesNewFile).to_string();
        let new_directory_label = self
            .ui_text
            .get(UiTextKey::ProjectFilesNewDirectory)
            .to_string();

        let content = match root_load_state {
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
            }
            ProjectTreeLoadState::Loaded if root_is_empty && !tree_is_editing => div()
                .debug_selector(|| "project-file-panel-empty".to_string())
                .flex()
                .flex_1()
                .items_center()
                .justify_center()
                .px(ui_style.spacing.xl)
                .text_sm()
                .text_color(theme.text_subtle)
                .child(self.ui_text.get(UiTextKey::ProjectFilesEmptyDirectory)),
            _ => div()
                .debug_selector(|| "project-file-tree".to_string())
                .flex()
                .flex_1()
                .overflow_hidden()
                .child(tree),
        };

        let refresh_project_id = project_id;
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
                .bg(theme.sidebar_background)
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .h_10()
                        .flex_none()
                        .border_b(ui_style.border.hairline)
                        .border_color(theme.border)
                        .px(ui_style.spacing.lg)
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .overflow_hidden()
                                .child(
                                    div()
                                        .text_sm()
                                        .font_weight(FontWeight::MEDIUM)
                                        .truncate()
                                        .child(self.ui_text.get(UiTextKey::ProjectFiles)),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(theme.text_subtle)
                                        .truncate()
                                        .child(project_name),
                                ),
                        )
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(ui_style.spacing.xs)
                                .child(
                                    Button::new("project-file-panel-new")
                                        .ghost()
                                        .xsmall()
                                        .icon(IconName::Plus)
                                        .h(ui_style.icon_buttons.toolbar_size)
                                        .rounded(ui_style.icon_buttons.toolbar_radius)
                                        .dropdown_menu(move |menu, _, _| {
                                            let new_file_tree = new_entry_tree.clone();
                                            let new_directory_tree = new_entry_tree.clone();
                                            let new_file_workbench =
                                                workbench_for_new_entry.clone();
                                            let new_directory_workbench =
                                                workbench_for_new_entry.clone();
                                            menu.item(
                                                PopupMenuItem::new(new_file_label.clone())
                                                    .on_click(move |_, window, cx| {
                                                        new_file_tree.update(
                                                            cx,
                                                            |tree, tree_cx| {
                                                                tree.begin_create_selected(
                                                                    false, window, tree_cx,
                                                                );
                                                            },
                                                        );
                                                        let _ = new_file_workbench.update(
                                                            cx,
                                                            |_, workbench_cx| {
                                                                workbench_cx.notify();
                                                            },
                                                        );
                                                    }),
                                            )
                                            .item(
                                                PopupMenuItem::new(new_directory_label.clone())
                                                    .on_click(move |_, window, cx| {
                                                        new_directory_tree.update(
                                                            cx,
                                                            |tree, tree_cx| {
                                                                tree.begin_create_selected(
                                                                    true, window, tree_cx,
                                                                );
                                                            },
                                                        );
                                                        let _ = new_directory_workbench.update(
                                                            cx,
                                                            |_, workbench_cx| {
                                                                workbench_cx.notify();
                                                            },
                                                        );
                                                    }),
                                            )
                                        }),
                                )
                                .child(
                                    yttt_button(
                                        "project-file-panel-refresh",
                                        self.ui_text.get(UiTextKey::ProjectFilesRefresh),
                                        YtttButtonVariant::Ghost,
                                        theme,
                                        ui_style,
                                        cx,
                                    )
                                    .on_click(cx.listener(
                                        move |this, _, window, cx| {
                                            this.refresh_project_tree(
                                                refresh_project_id.clone(),
                                                window,
                                                cx,
                                            );
                                            cx.notify();
                                        },
                                    )),
                                ),
                        ),
                )
                .child(content)
                .child(resize_handle),
        )
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
        let project_id = ProjectId::new(&context.project_id);
        context.ssh =
            self.workspace
                .project(&project_id)
                .and_then(|project| match &project.location {
                    ProjectLocation::Ssh { connection_id, .. } => Some(SshTerminalContext {
                        connection_id: connection_id.clone(),
                        transport: self.ssh.transport.clone(),
                    }),
                    ProjectLocation::Local { .. } => None,
                });
        let key = terminal_pane_key(&context.project_id, &context.tab_id, &context.pane.id);
        if let Some(pane_view) = self.terminal.terminal_panes.get(&key) {
            return pane_view.clone();
        }

        let project_id = context.project_id.clone();
        let tab_id = context.tab_id.clone();
        let pane_id = context.pane.id.clone();
        let terminal_config = self.theme_runtime().to_terminal_config();
        let theme = self.theme_runtime().ui;
        let start_processes = self.terminal.start_processes;
        let pane_view = cx.new(|cx| {
            if start_processes {
                TerminalPaneView::new(context, terminal_config, theme, cx)
            } else {
                TerminalPaneView::new_without_processes(context, terminal_config, theme, cx)
            }
        });
        if pane_view.read(cx).is_running()
            && let Err(error) =
                self.workspace
                    .mark_pane_running(&ProjectId::new(&project_id), &tab_id, &pane_id)
        {
            self.load_error = Some(error.to_string());
        }
        let subscription = cx.subscribe_in(&pane_view, window, Self::on_terminal_pane_event);
        self.terminal
            .terminal_pane_subscriptions
            .insert(key.clone(), subscription);
        self.terminal.terminal_panes.insert(key, pane_view.clone());
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
            is_focused: input.is_focused,
            terminal_input_gate: self.terminal.terminal_input_gate.clone(),
            ssh: None,
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

        let appearance = self.theme_runtime();
        let ui_style = appearance.style;
        let border_color = if input.is_focused {
            appearance.ui.focused_pane_border
        } else {
            rgba(0x00000000)
        };
        let terminal_input_allowed = self.terminal_input_allowed();
        let mut wrapper = div()
            .flex()
            .flex_1()
            .relative()
            .border(ui_style.border.hairline)
            .border_color(border_color);
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
        wrapper = wrapper.child(pane_view);
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

    pub(super) fn on_terminal_pane_event(
        &mut self,
        _pane: &Entity<TerminalPaneView>,
        event: &TerminalPaneEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            TerminalPaneEvent::Notification(event) => {
                let root = cx.entity();
                let event = event.clone();
                let action_label = self.ui_text.get(UiTextKey::OpenNotificationTarget);
                let appearance = self.theme_runtime();
                let theme = appearance.ui;
                let ui_style = appearance.style;
                self.handle_terminal_notification(event.clone());
                push_component_notification(
                    root,
                    event,
                    action_label,
                    theme,
                    ui_style,
                    _window,
                    cx,
                );
                cx.notify();
            }
            TerminalPaneEvent::Started(event) => {
                if let Err(error) = self.handle_terminal_pane_started(event.clone()) {
                    self.load_error = Some(error.to_string());
                }
                cx.notify();
            }
            TerminalPaneEvent::IoError { message, .. } => {
                self.load_error = Some(message.clone());
                cx.notify();
            }
            TerminalPaneEvent::TitleChanged { .. } => {
                cx.notify();
            }
            TerminalPaneEvent::Exited(event) => {
                if let Err(error) = self.handle_terminal_pane_exit(event.clone()) {
                    self.load_error = Some(error.to_string());
                }
                cx.notify();
            }
        }
    }
}
