use super::*;

impl WorkbenchView {
    pub(super) fn active_terminal_split_view(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        self.prune_terminal_panes();

        let Some((project_id, project_path, project_title, tab_id, tab_title, layout)) =
            self.selected_tab_layout_clone()
        else {
            return project_empty_terminal_state(cx, &self.ui_text, &self.theme_runtime.ui);
        };

        let focused_pane_id = self.selected_focused_pane_id().map(ToOwned::to_owned);
        let tree_input = RenderTerminalTreeInput {
            project_id: &project_id,
            project_path: &project_path,
            project_title: &project_title,
            tab_id: &tab_id,
            tab_title: &tab_title,
            focused_pane_id: focused_pane_id.as_deref(),
        };

        div()
            .flex()
            .flex_1()
            .bg(self.theme_runtime.ui.terminal_background)
            .text_color(self.theme_runtime.ui.text)
            .child(self.terminal_split_view_for_layout(&layout, &tree_input, window, cx))
    }

    pub(super) fn active_work_item_view(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let Some(WorkItemId::File(document_id)) = self.active_work_item() else {
            return self.active_terminal_split_view(window, cx);
        };
        let document = self.project_editor_runtime.document(&document_id).cloned();
        if self.pending_editor_focus_document_id.as_ref() == Some(&document_id)
            && self.foreground_input_owner_kind() == InputOwnerKind::Editor
            && let Some(document) = &document
        {
            let document = document.clone();
            window.defer(cx, move |window, cx| {
                document.update(cx, |document, document_cx| {
                    document.focus(window, document_cx);
                });
            });
            self.pending_editor_focus_document_id = None;
        }

        let display_path = self
            .workspace
            .project(&document_id.project_id)
            .and_then(|project| document_id.canonical_path.strip_prefix(&project.path).ok())
            .unwrap_or(&document_id.canonical_path)
            .to_path_buf();
        let (language, dirty) = document
            .as_ref()
            .map(|document| {
                let document = document.read(cx);
                (
                    document.model().editor().language().to_string(),
                    document.model().is_dirty(),
                )
            })
            .unwrap_or_else(|| ("text".to_string(), false));

        div()
            .debug_selector(|| "active-file-editor".to_string())
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .bg(self.theme_runtime.editor.background)
            .child(Self::project_editor_header(
                &display_path,
                &language,
                dirty,
                self.icon_theme.resolve_file(&document_id.canonical_path),
                self.theme_runtime.ui,
            ))
            .child(div().flex_1().min_h_0().children(document))
    }

    pub(super) fn project_editor_header(
        display_path: &Path,
        language: &str,
        dirty: bool,
        icon: crate::ui::theme::icons::IconVisual,
        theme: WorkbenchTheme,
    ) -> impl IntoElement {
        div()
            .id("project-editor-header")
            .flex()
            .flex_none()
            .h(px(32.))
            .items_center()
            .gap_2()
            .px_3()
            .border_b_1()
            .border_color(theme.border)
            .bg(theme.surface_elevated)
            .child(icon_for_visual(icon, theme.text_muted))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .truncate()
                    .text_sm()
                    .text_color(theme.text)
                    .child(display_path.to_string_lossy().into_owned()),
            )
            .child(
                div()
                    .flex_none()
                    .text_xs()
                    .text_color(theme.text_muted)
                    .child(language.to_string()),
            )
            .when(dirty, |this| {
                this.child(div().size(px(6.)).rounded_full().bg(theme.accent))
            })
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
            .project_editor_runtime
            .workspace()
            .session(project_id)
            .map(|session| {
                session
                    .file_ids()
                    .iter()
                    .map(|document_id| FileTabSnapshot {
                        id: document_id.clone(),
                        relative_path: document_id
                            .canonical_path
                            .strip_prefix(&project.path)
                            .unwrap_or(&document_id.canonical_path)
                            .to_path_buf(),
                        dirty: self
                            .project_editor_runtime
                            .document(document_id)
                            .is_some_and(|document| document.read(cx).model().is_dirty()),
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let active = self.active_work_item();
        merge_work_item_tabs(&terminal_items, &file_items, active.as_ref())
    }

    pub fn selected_project_panel_visible(&self) -> bool {
        let Some(project_id) = self.workspace.selected_project_id() else {
            return false;
        };
        self.project_editor_runtime
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
            .project_editor_runtime
            .workspace()
            .session(&project_id)?;
        let panel_width = session.project_panel_width();
        let root_load_state = session.file_tree().directory_load_state(Path::new(""));
        let root_is_empty = session.file_tree().visible_rows().is_empty();
        let has_root_snapshot = session.file_tree().has_snapshot(Path::new(""));
        let theme = self.theme_runtime.ui;

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
                    .px_4()
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
                    .gap_3()
                    .px_4()
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
            ProjectTreeLoadState::Loaded if root_is_empty => div()
                .debug_selector(|| "project-file-panel-empty".to_string())
                .flex()
                .flex_1()
                .items_center()
                .justify_center()
                .px_4()
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
                        .h(px(40.0))
                        .flex_none()
                        .border_b_1()
                        .border_color(theme.border)
                        .px_3()
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
                            yttt_button(
                                "project-file-panel-refresh",
                                self.ui_text.get(UiTextKey::ProjectFilesRefresh),
                                YtttButtonVariant::Ghost,
                                theme,
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
                    project_id: tree_input.project_id,
                    project_path: tree_input.project_path,
                    project_title: tree_input.project_title,
                    pane,
                    tab_id: tree_input.tab_id,
                    tab_title: tree_input.tab_title,
                    is_focused: tree_input.focused_pane_id == Some(pane.id.as_str()),
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

        Some((
            selected_project_id.as_str().to_string(),
            project.path.clone(),
            project.layout.project.name.clone(),
            project.selected_tab_id.clone(),
            tab.title.clone(),
            tab.layout.clone(),
        ))
    }

    pub(super) fn render_terminal_pane(
        &mut self,
        input: RenderTerminalPaneInput<'_>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let key = terminal_pane_key(input.project_id, input.tab_id, &input.pane.id);
        let pane_view = if let Some(pane_view) = self.terminal_panes.get(&key) {
            pane_view.clone()
        } else {
            let context = TerminalPaneContext {
                project_id: input.project_id.to_string(),
                project_path: input.project_path.to_path_buf(),
                project_title: input.project_title.to_string(),
                tab_id: input.tab_id.to_string(),
                tab_title: input.tab_title.to_string(),
                pane: input.pane.clone(),
                is_focused: input.is_focused,
                terminal_input_gate: self.terminal_input_gate.clone(),
            };
            let terminal_config = self.theme_runtime.to_terminal_config();
            let theme = self.theme_runtime.ui;
            let pane_view = cx.new(|cx| TerminalPaneView::new(context, terminal_config, theme, cx));
            let subscription = cx.subscribe_in(&pane_view, window, Self::on_terminal_pane_event);
            self.terminal_pane_subscriptions
                .insert(key.clone(), subscription);
            self.terminal_panes.insert(key, pane_view.clone());
            pane_view
        };

        let pane_id = input.pane.id.clone();
        if self
            .pending_terminal_focus_pane_id
            .as_deref()
            .is_some_and(|pending| pending == pane_id)
            && self.should_auto_focus_workspace()
            && pane_view.update(cx, |pane, cx| pane.focus_terminal(window, cx))
        {
            self.pending_terminal_focus_pane_id = None;
        }

        let border_color = if input.is_focused {
            self.theme_runtime.ui.focused_pane_border
        } else {
            rgba(0x00000000)
        };
        let terminal_input_allowed = self.terminal_input_allowed();
        let mut wrapper = div()
            .flex()
            .flex_1()
            .relative()
            .border_1()
            .border_color(border_color)
            .bg(self.theme_runtime.ui.terminal_background);
        wrapper.interactivity().on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _window, cx| {
                if !this.terminal_input_allowed() {
                    cx.stop_propagation();
                    return;
                }
                let _ = this.focus_visible_terminal_pane(&pane_id);
                cx.notify();
            }),
        );
        wrapper = wrapper.child(pane_view);
        if !terminal_input_allowed {
            wrapper = wrapper.child(
                div()
                    .absolute()
                    .inset_0()
                    .bg(rgba(0x00000000))
                    .on_mouse_down(MouseButton::Left, |_event, _window, cx| {
                        cx.stop_propagation();
                    }),
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

        self.terminal_panes
            .retain(|key, _pane| live_keys.contains(key));
        self.terminal_pane_subscriptions
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
                self.handle_terminal_notification(event.clone());
                push_component_notification(root, event, _window, cx);
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
