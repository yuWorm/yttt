use super::*;

const FILE_PREVIEW_DEBOUNCE: Duration = Duration::from_millis(90);
const FILE_PREVIEW_MAX_LINES: usize = 2_000;

impl WorkbenchView {
    pub(super) fn prepare_file_finder(&mut self) {
        self.palette.file_search_generation = self.palette.file_search_generation.wrapping_add(1);
        self.palette.file_match_generation = self.palette.file_match_generation.wrapping_add(1);
        self.palette.file_preview_generation = self.palette.file_preview_generation.wrapping_add(1);
        self.palette.file_candidates = Arc::new(Vec::new());
        self.palette.file_matches.clear();
        self.palette.file_search_loading = true;
        self.palette.file_search_error = None;
        self.palette.pending_file_search_load = true;
        self.clear_file_preview();
    }

    pub(super) fn cancel_file_finder_tasks(&mut self) {
        self.palette.file_search_generation = self.palette.file_search_generation.wrapping_add(1);
        self.palette.file_match_generation = self.palette.file_match_generation.wrapping_add(1);
        self.palette.file_preview_generation = self.palette.file_preview_generation.wrapping_add(1);
        self.palette.pending_file_search_load = false;
        self.palette.file_search_loading = false;
    }

    pub(super) fn flush_pending_file_finder_operations(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.palette.pending_file_search_load {
            return;
        }
        self.palette.pending_file_search_load = false;
        let generation = self.palette.file_search_generation;
        let show_hidden = self.app_settings.project_panel.show_hidden;
        let projects = self
            .workspace
            .opened_projects()
            .iter()
            .filter_map(|project| {
                Some(FileSearchProject {
                    project_id: project.id.clone(),
                    project_title: project.location.fallback_title(),
                    services: self.project.services.get(&project.id)?.clone(),
                })
            })
            .collect::<Vec<_>>();
        let task = cx
            .background_spawn(async move { collect_file_search_candidates(projects, show_hidden) });

        cx.spawn_in(window, async move |this, cx| {
            let collection = task.await;
            let _ = this.update_in(cx, |root, window, cx| {
                if root.palette.file_search_generation != generation
                    || !matches!(
                        root.palette
                            .active_palette
                            .as_ref()
                            .map(|palette| palette.kind),
                        Some(PaletteKind::File)
                    )
                {
                    return;
                }
                root.apply_file_search_collection(collection);
                let query = root
                    .palette
                    .active_palette
                    .as_ref()
                    .map(|palette| palette.query.clone())
                    .unwrap_or_default();
                root.spawn_file_candidate_match(query, window, cx);
                cx.notify();
            });
        })
        .detach();
    }

    fn apply_file_search_collection(&mut self, collection: FileSearchCollection) {
        self.palette.file_search_loading = false;
        self.palette.file_candidates = Arc::new(collection.candidates);
        self.palette.file_search_error =
            (!collection.errors.is_empty()).then(|| collection.errors.join("; "));
    }

    pub(super) fn spawn_file_candidate_match(
        &mut self,
        query: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.palette.file_search_loading {
            return;
        }
        self.palette.file_match_generation = self.palette.file_match_generation.wrapping_add(1);
        let generation = self.palette.file_match_generation;
        let candidates = self.palette.file_candidates.clone();
        self.palette.file_matches.clear();
        self.clear_file_preview();
        let match_query = query.clone();
        let task =
            cx.background_spawn(
                async move { match_file_search_candidates(&candidates, &match_query) },
            );

        cx.spawn_in(window, async move |this, cx| {
            let matches = task.await;
            let _ = this.update_in(cx, |root, window, cx| {
                let query_is_current =
                    root.palette.active_palette.as_ref().is_some_and(|palette| {
                        palette.kind == PaletteKind::File && palette.query == query
                    });
                if root.palette.file_match_generation != generation || !query_is_current {
                    return;
                }
                root.palette.file_matches = matches;
                if let Some(active) = &mut root.palette.active_palette {
                    active.selected_index = active
                        .selected_index
                        .min(root.palette.file_matches.len().saturating_sub(1));
                }
                root.spawn_file_preview(window, cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn spawn_file_preview(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.palette.file_preview_generation = self.palette.file_preview_generation.wrapping_add(1);
        let generation = self.palette.file_preview_generation;
        let Some(candidate) = self.selected_file_search_candidate().cloned() else {
            self.clear_file_preview();
            return;
        };
        let Some(services) = self.project.services.get(&candidate.project_id).cloned() else {
            self.clear_file_preview();
            return;
        };

        self.palette.file_preview_loading = true;
        self.palette.file_preview_error = None;
        self.palette.file_preview_rows = Arc::new(Vec::new());
        self.palette.file_preview_path = Some((
            candidate.project_id.clone(),
            candidate.relative_path.clone(),
        ));
        self.palette.file_preview_vertical_scroll = UniformListScrollHandle::new();
        self.palette.file_preview_horizontal_scroll = ScrollHandle::new();
        let preview_path = candidate.relative_path.clone();
        let executor = cx.background_executor().clone();
        let task = cx.background_spawn(async move {
            executor.timer(FILE_PREVIEW_DEBOUNCE).await;
            services.read_file(&preview_path)
        });

        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |root, _window, cx| {
                if root.palette.file_preview_generation != generation
                    || root.palette.file_preview_path.as_ref()
                        != Some(&(
                            candidate.project_id.clone(),
                            candidate.relative_path.clone(),
                        ))
                {
                    return;
                }
                root.palette.file_preview_loading = false;
                match result {
                    Ok(file) => {
                        let editor_theme = root.theme_runtime().editor;
                        let highlights = Arc::new(Vec::new());
                        root.palette.file_preview_rows = Arc::new(
                            file.text
                                .lines()
                                .take(FILE_PREVIEW_MAX_LINES)
                                .enumerate()
                                .map(|(index, line)| {
                                    ReadonlyCodeRow::code(
                                        [Some(index + 1), None],
                                        "",
                                        line.to_string(),
                                        highlights.clone(),
                                        editor_theme.background,
                                        rgba(0x00000000),
                                    )
                                })
                                .collect(),
                        );
                        root.palette.file_preview_error = None;
                    }
                    Err(error) => {
                        root.palette.file_preview_rows = Arc::new(Vec::new());
                        root.palette.file_preview_error = Some(error.to_string());
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn file_finder_palette_items(&self) -> Vec<PaletteItem> {
        if self.palette.file_search_loading {
            return vec![file_finder_state_item(
                self.ui_text.get(UiTextKey::FileFinderLoading),
                None,
            )];
        }
        if self.palette.file_candidates.is_empty()
            && let Some(error) = &self.palette.file_search_error
        {
            return vec![file_finder_state_item(
                self.ui_text.get(UiTextKey::FileFinderLoadFailed),
                Some(error.clone()),
            )];
        }

        let show_project = self.workspace.opened_projects().len() > 1;
        self.palette
            .file_matches
            .iter()
            .filter_map(|file_match| {
                let candidate = self
                    .palette
                    .file_candidates
                    .get(file_match.candidate_index)?;
                Some(PaletteItem {
                    id: file_match.candidate_index.to_string(),
                    title: candidate.file_name.clone(),
                    subtitle: Some(candidate.display_path.clone()),
                    status: show_project.then(|| candidate.project_title.clone()),
                    keybinding: None,
                    command: CommandId::FileFind,
                    enabled: true,
                    disabled_reason: None,
                })
            })
            .collect()
    }

    pub(super) fn selected_file_search_candidate(&self) -> Option<&FileSearchCandidate> {
        let active = self.palette.active_palette.as_ref()?;
        if active.kind != PaletteKind::File {
            return None;
        }
        let file_match = self.palette.file_matches.get(
            active
                .selected_index
                .min(self.palette.file_matches.len().saturating_sub(1)),
        )?;
        self.palette.file_candidates.get(file_match.candidate_index)
    }

    pub(super) fn open_file_search_candidate(
        &mut self,
        candidate_index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(candidate) = self.palette.file_candidates.get(candidate_index).cloned() else {
            return false;
        };
        self.close_palette();
        self.spawn_project_file_open(candidate.project_id, candidate.relative_path, window, cx);
        true
    }

    pub(super) fn file_finder_preview_element(&self) -> AnyElement {
        let runtime = self.theme_runtime();
        let theme = runtime.ui;
        let ui_style = runtime.style;
        let candidate = self.selected_file_search_candidate();
        let title = candidate
            .map(|candidate| candidate.display_path.clone())
            .unwrap_or_else(|| {
                self.ui_text
                    .get(UiTextKey::FileFinderPreviewTitle)
                    .to_string()
            });
        let project_title = candidate
            .map(|candidate| candidate.project_title.clone())
            .unwrap_or_default();
        let body = if self.palette.file_preview_loading {
            div()
                .flex()
                .flex_1()
                .items_center()
                .justify_center()
                .text_sm()
                .text_color(theme.text_subtle)
                .child(self.ui_text.get(UiTextKey::FileFinderPreviewLoading))
                .into_any_element()
        } else if let Some(error) = &self.palette.file_preview_error {
            div()
                .flex()
                .flex_1()
                .items_center()
                .justify_center()
                .p(ui_style.spacing.xl)
                .text_sm()
                .text_color(theme.text_subtle)
                .child(error.clone())
                .into_any_element()
        } else if self.palette.file_preview_rows.is_empty() {
            div()
                .flex()
                .flex_1()
                .items_center()
                .justify_center()
                .text_sm()
                .text_color(theme.text_subtle)
                .child(self.ui_text.get(UiTextKey::FileFinderPreviewEmpty))
                .into_any_element()
        } else {
            div()
                .flex()
                .flex_1()
                .min_h_0()
                .child(
                    ReadonlyCodeView::new(
                        "file-finder-preview",
                        self.palette.file_preview_rows.clone(),
                        self.palette.file_preview_vertical_scroll.clone(),
                        self.palette.file_preview_horizontal_scroll.clone(),
                        EditorAppearance::from(&self.app_settings.editor),
                        runtime.editor,
                        theme.border,
                    )
                    .number_columns(1)
                    .content_width(1_000.),
                )
                .into_any_element()
        };

        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(runtime.editor.background)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(ui_style.spacing.xs)
                    .border_b(ui_style.border.hairline)
                    .border_color(theme.border)
                    .px(ui_style.spacing.lg)
                    .py(ui_style.spacing.md)
                    .child(
                        div()
                            .text_sm()
                            .text_color(theme.text)
                            .truncate()
                            .child(title),
                    )
                    .when(!project_title.is_empty(), |this| {
                        this.child(
                            div()
                                .text_xs()
                                .text_color(theme.text_subtle)
                                .child(project_title),
                        )
                    }),
            )
            .child(body)
            .into_any_element()
    }

    fn clear_file_preview(&mut self) {
        self.palette.file_preview_loading = false;
        self.palette.file_preview_path = None;
        self.palette.file_preview_rows = Arc::new(Vec::new());
        self.palette.file_preview_error = None;
    }
}

fn file_finder_state_item(title: &str, detail: Option<String>) -> PaletteItem {
    PaletteItem {
        id: "file-finder-state".to_string(),
        title: title.to_string(),
        subtitle: detail,
        status: None,
        keybinding: None,
        command: CommandId::FileFind,
        enabled: false,
        disabled_reason: Some(title.to_string()),
    }
}
