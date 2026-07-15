use super::*;
use gpui_component::StyledExt as _;

const MAX_EAGER_DIFF_LINES: usize = 2_000;
const MAX_EAGER_DIFF_BYTES: usize = 512 * 1024;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum GitDiffViewMode {
    #[default]
    Unified,
    Split,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum GitSplitRow {
    Hunk(usize),
    Lines {
        left: Option<usize>,
        right: Option<usize>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum GitDiffSidebarRow {
    Folder {
        group_index: usize,
        path: SharedString,
        collapsed: bool,
    },
    File {
        file_index: usize,
        filename: SharedString,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum GitDiffPanelContent {
    Loading,
    Ready(Arc<GitDiffResult>),
    Error(String),
}

#[derive(Clone, Debug)]
pub(super) struct GitDiffPanel {
    pub(super) project_id: ProjectId,
    pub(super) project_path: PathBuf,
    pub(super) mode: GitDiffMode,
    pub(super) view_mode: GitDiffViewMode,
    pub(super) ignore_whitespace: bool,
    pub(super) selected_file: usize,
    pub(super) collapsed_folders: BTreeSet<String>,
    pub(super) content: GitDiffPanelContent,
    pub(super) diff_scroll_handle: UniformListScrollHandle,
    pub(super) file_scroll_handle: UniformListScrollHandle,
    pub(super) focus_handle: Option<FocusHandle>,
    pub(super) sidebar_rows: Arc<Vec<GitDiffSidebarRow>>,
    pub(super) split_rows: Arc<Vec<GitSplitRow>>,
    pub(super) syntax_highlights: Arc<Vec<Vec<(Range<usize>, HighlightStyle)>>>,
    pub(super) unified_view_rows: Arc<Vec<ReadonlyCodeRow>>,
    pub(super) split_left_view_rows: Arc<Vec<ReadonlyCodeRow>>,
    pub(super) split_right_view_rows: Arc<Vec<ReadonlyCodeRow>>,
    pub(super) unified_horizontal_scroll_handle: ScrollHandle,
    pub(super) split_left_horizontal_scroll_handle: ScrollHandle,
    pub(super) split_right_horizontal_scroll_handle: ScrollHandle,
    pub(super) unified_content_width: f32,
    pub(super) split_content_width: f32,
}

impl WorkbenchView {
    pub fn git_diff_panel_is_open(&self) -> bool {
        self.overlays.git_diff_panel.is_some()
    }

    pub fn open_git_branch_switcher(&mut self) -> Result<(), WorkbenchError> {
        let (project_id, project_path) = self.selected_project_git_target()?;
        self.overlays.git_diff_panel = None;
        self.open_palette(PaletteKind::GitBranch);
        self.palette.git_branch_generation = self.palette.git_branch_generation.wrapping_add(1);
        let generation = self.palette.git_branch_generation;
        self.palette.git_branch_project_id = Some(project_id.clone());
        self.palette.git_branches.clear();
        self.palette.git_branch_loading = true;
        self.palette.git_branch_switching = false;
        self.palette.git_branch_error = None;
        self.palette.pending_git_branch_switch = None;
        self.palette.pending_git_branch_load = Some((project_id, project_path, generation));
        Ok(())
    }

    pub fn open_git_diff_panel(&mut self) -> Result<(), WorkbenchError> {
        let (project_id, project_path) = self.selected_project_git_target()?;
        self.close_palette();
        self.overlays.git_diff_generation = self.overlays.git_diff_generation.wrapping_add(1);
        let generation = self.overlays.git_diff_generation;
        self.overlays.git_diff_panel = Some(GitDiffPanel {
            project_id: project_id.clone(),
            project_path: project_path.clone(),
            mode: GitDiffMode::Unstaged,
            view_mode: GitDiffViewMode::Unified,
            ignore_whitespace: false,
            selected_file: 0,
            collapsed_folders: BTreeSet::new(),
            content: GitDiffPanelContent::Loading,
            diff_scroll_handle: UniformListScrollHandle::new(),
            file_scroll_handle: UniformListScrollHandle::new(),
            focus_handle: None,
            sidebar_rows: Arc::new(Vec::new()),
            split_rows: Arc::new(Vec::new()),
            syntax_highlights: Arc::new(Vec::new()),
            unified_view_rows: Arc::new(Vec::new()),
            split_left_view_rows: Arc::new(Vec::new()),
            split_right_view_rows: Arc::new(Vec::new()),
            unified_horizontal_scroll_handle: ScrollHandle::new(),
            split_left_horizontal_scroll_handle: ScrollHandle::new(),
            split_right_horizontal_scroll_handle: ScrollHandle::new(),
            unified_content_width: 900.0,
            split_content_width: 700.0,
        });
        self.overlays.pending_git_diff_load = Some((project_id, project_path, generation));
        self.sync_input_owner_state();
        Ok(())
    }

    pub fn close_git_diff_panel(&mut self) {
        self.overlays.git_diff_panel = None;
        self.overlays.pending_git_diff_load = None;
        self.sync_input_owner_state();
    }

    pub fn git_diff_mode(&self) -> Option<GitDiffMode> {
        self.overlays
            .git_diff_panel
            .as_ref()
            .map(|panel| panel.mode)
    }

    pub(super) fn git_diff_view_mode(&self) -> Option<GitDiffViewMode> {
        self.overlays
            .git_diff_panel
            .as_ref()
            .map(|panel| panel.view_mode)
    }

    pub fn set_git_diff_mode(&mut self, mode: GitDiffMode) -> bool {
        let Some(panel) = self.overlays.git_diff_panel.as_mut() else {
            return false;
        };
        if panel.mode == mode {
            return false;
        }
        panel.mode = mode;
        self.queue_git_diff_reload();
        true
    }

    pub(super) fn set_git_diff_view_mode(&mut self, view_mode: GitDiffViewMode) -> bool {
        let Some(panel) = self.overlays.git_diff_panel.as_mut() else {
            return false;
        };
        if panel.view_mode == view_mode {
            return false;
        }
        panel.view_mode = view_mode;
        true
    }

    pub fn toggle_git_diff_whitespace(&mut self) -> bool {
        let Some(panel) = self.overlays.git_diff_panel.as_mut() else {
            return false;
        };
        panel.ignore_whitespace = !panel.ignore_whitespace;
        self.queue_git_diff_reload();
        true
    }

    pub fn select_git_diff_file(&mut self, index: usize) -> bool {
        let Some(panel) = self.overlays.git_diff_panel.as_mut() else {
            return false;
        };
        let GitDiffPanelContent::Ready(result) = &panel.content else {
            return false;
        };
        if index >= result.files.len() || panel.selected_file == index {
            return false;
        }
        panel.selected_file = index;
        panel.diff_scroll_handle = UniformListScrollHandle::new();
        sync_selected_git_diff(panel);
        if let Some(row_index) = panel.sidebar_rows.iter().position(|row| {
            matches!(
                row,
                GitDiffSidebarRow::File { file_index, .. } if *file_index == index
            )
        }) {
            panel
                .file_scroll_handle
                .scroll_to_item(row_index, gpui::ScrollStrategy::Nearest);
        }
        true
    }

    pub(super) fn handle_git_diff_key_down(
        &mut self,
        event: &KeyDownEvent,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.overlays.git_diff_panel.is_none() {
            return false;
        }
        let key = event.keystroke.key.as_str();
        match key {
            "escape" => self.close_git_diff_panel(),
            "tab" => {
                let mode = match self.git_diff_mode().unwrap_or_default() {
                    GitDiffMode::Unstaged => GitDiffMode::Staged,
                    GitDiffMode::Staged => GitDiffMode::Unstaged,
                };
                self.set_git_diff_mode(mode);
            }
            "s" => {
                let mode = match self.git_diff_view_mode().unwrap_or_default() {
                    GitDiffViewMode::Unified => GitDiffViewMode::Split,
                    GitDiffViewMode::Split => GitDiffViewMode::Unified,
                };
                self.set_git_diff_view_mode(mode);
            }
            "w" => {
                self.toggle_git_diff_whitespace();
            }
            "up" => {
                self.select_relative_git_diff_file(false);
            }
            "down" => {
                self.select_relative_git_diff_file(true);
            }
            "c" if event.keystroke.modifiers.platform || event.keystroke.modifiers.control => {
                if let Some(text) = self.selected_git_diff_text() {
                    cx.write_to_clipboard(ClipboardItem::new_string(text));
                }
            }
            _ => return false,
        }
        true
    }

    fn select_relative_git_diff_file(&mut self, forward: bool) -> bool {
        let Some(panel) = self.overlays.git_diff_panel.as_ref() else {
            return false;
        };
        let GitDiffPanelContent::Ready(result) = &panel.content else {
            return false;
        };
        if result.files.is_empty() {
            return false;
        }
        let next = if forward {
            (panel.selected_file + 1) % result.files.len()
        } else if panel.selected_file == 0 {
            result.files.len() - 1
        } else {
            panel.selected_file - 1
        };
        self.select_git_diff_file(next)
    }

    fn toggle_git_diff_folder(&mut self, folder: &str) -> bool {
        let Some(panel) = self.overlays.git_diff_panel.as_mut() else {
            return false;
        };
        if !panel.collapsed_folders.remove(folder) {
            panel.collapsed_folders.insert(folder.to_string());
        }
        sync_git_diff_sidebar(panel);
        true
    }

    fn queue_git_diff_reload(&mut self) {
        let Some(panel) = self.overlays.git_diff_panel.as_mut() else {
            return;
        };
        self.overlays.git_diff_generation = self.overlays.git_diff_generation.wrapping_add(1);
        let generation = self.overlays.git_diff_generation;
        panel.content = GitDiffPanelContent::Loading;
        panel.selected_file = 0;
        panel.sidebar_rows = Arc::new(Vec::new());
        panel.split_rows = Arc::new(Vec::new());
        panel.syntax_highlights = Arc::new(Vec::new());
        panel.unified_view_rows = Arc::new(Vec::new());
        panel.split_left_view_rows = Arc::new(Vec::new());
        panel.split_right_view_rows = Arc::new(Vec::new());
        panel.unified_horizontal_scroll_handle = ScrollHandle::new();
        panel.split_left_horizontal_scroll_handle = ScrollHandle::new();
        panel.split_right_horizontal_scroll_handle = ScrollHandle::new();
        panel.unified_content_width = 900.0;
        panel.split_content_width = 700.0;
        panel.diff_scroll_handle = UniformListScrollHandle::new();
        panel.file_scroll_handle = UniformListScrollHandle::new();
        self.overlays.pending_git_diff_load = Some((
            panel.project_id.clone(),
            panel.project_path.clone(),
            generation,
        ));
    }

    fn selected_git_diff_text(&self) -> Option<String> {
        let panel = self.overlays.git_diff_panel.as_ref()?;
        let GitDiffPanelContent::Ready(result) = &panel.content else {
            return None;
        };
        let file = result.files.get(panel.selected_file)?;
        let mut text = format!("diff --git a/{0} b/{0}\n", file.path());
        for line in file.lines() {
            let prefix = match line.kind {
                GitDiffLineKind::Added => "+",
                GitDiffLineKind::Removed => "-",
                GitDiffLineKind::Context => " ",
                GitDiffLineKind::Hunk => "",
            };
            text.push_str(prefix);
            text.push_str(&line.content);
            text.push('\n');
        }
        Some(text)
    }

    pub(super) fn git_branch_palette_items(&self) -> Vec<PaletteItem> {
        let mut items = Vec::new();
        if self.palette.git_branch_loading {
            items.push(git_branch_state_item(
                self.ui_text.get(UiTextKey::GitBranchesLoading),
                None,
            ));
            return items;
        }
        if let Some(error) = &self.palette.git_branch_error {
            items.push(git_branch_state_item(
                self.ui_text.get(UiTextKey::GitBranchSwitchFailed),
                Some(error.clone()),
            ));
        }

        items.extend(self.palette.git_branches.iter().map(|branch| {
            PaletteItem {
                id: branch.id(),
                title: branch.name.clone(),
                subtitle: Some(
                    self.ui_text
                        .get(match branch.kind {
                            crate::runtime::git_status::GitBranchKind::Local => {
                                UiTextKey::GitBranchLocal
                            }
                            crate::runtime::git_status::GitBranchKind::Remote => {
                                UiTextKey::GitBranchRemote
                            }
                        })
                        .to_string(),
                ),
                status: branch
                    .current
                    .then(|| self.ui_text.get(UiTextKey::PaletteStatusActive).to_string()),
                keybinding: None,
                command: CommandId::GitBranchSwitch,
                enabled: !self.palette.git_branch_switching && !branch.current,
                disabled_reason: branch.current.then(|| {
                    self.ui_text
                        .get(UiTextKey::GitBranchAlreadyActive)
                        .to_string()
                }),
            }
        }));
        items
    }

    pub(super) fn queue_git_branch_switch(&mut self, branch_id: &str) -> bool {
        if self.palette.git_branch_loading || self.palette.git_branch_switching {
            return false;
        }
        let Some(branch) = self
            .palette
            .git_branches
            .iter()
            .find(|branch| branch.id() == branch_id)
            .cloned()
        else {
            return false;
        };
        if branch.current {
            self.close_palette();
            return true;
        }
        let Some(project_id) = self.palette.git_branch_project_id.clone() else {
            return false;
        };
        let Some(project_path) = self
            .workspace
            .project(&project_id)
            .map(|project| project.path.clone())
        else {
            return false;
        };
        self.palette.git_branch_switching = true;
        self.palette.git_branch_error = None;
        self.palette.pending_git_branch_switch = Some((
            project_id,
            project_path,
            branch,
            self.palette.git_branch_generation,
        ));
        true
    }

    pub(super) fn flush_pending_git_operations(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some((project_id, project_path, generation)) =
            self.palette.pending_git_branch_load.take()
        {
            let task = cx.background_spawn({
                let project_path = project_path.clone();
                async move { read_project_git_branches(&project_path) }
            });
            cx.spawn_in(window, async move |this, cx| {
                let result = task.await;
                let _ = this.update_in(cx, |root, _window, cx| {
                    if root.palette.git_branch_generation != generation
                        || root.palette.git_branch_project_id.as_ref() != Some(&project_id)
                    {
                        return;
                    }
                    root.palette.git_branch_loading = false;
                    match result {
                        Ok(branches) => {
                            root.palette.git_branches = branches;
                            root.palette.git_branch_error = None;
                        }
                        Err(error) => {
                            root.palette.git_branches.clear();
                            root.palette.git_branch_error = Some(error);
                        }
                    }
                    cx.notify();
                });
            })
            .detach();
        }

        if let Some((project_id, project_path, branch, generation)) =
            self.palette.pending_git_branch_switch.take()
        {
            let task = cx.background_spawn({
                let project_path = project_path.clone();
                let branch = branch.clone();
                async move { switch_project_git_branch(&project_path, &branch) }
            });
            cx.spawn_in(window, async move |this, cx| {
                let result = task.await;
                let _ = this.update_in(cx, |root, window, cx| {
                    if root.palette.git_branch_generation != generation {
                        return;
                    }
                    root.palette.git_branch_switching = false;
                    match result {
                        Ok(()) => {
                            root.palette.git_branch_error = None;
                            root.refresh_project_tree(project_id.clone(), window, cx);
                            if root
                                .palette
                                .active_palette
                                .as_ref()
                                .is_some_and(|palette| palette.kind == PaletteKind::GitBranch)
                            {
                                root.close_palette();
                            }
                            root.load_error = None;
                        }
                        Err(error) => {
                            root.palette.git_branch_error = Some(error.clone());
                            root.load_error = Some(error);
                        }
                    }
                    cx.notify();
                });
            })
            .detach();
        }

        if let Some((project_id, project_path, generation)) =
            self.overlays.pending_git_diff_load.take()
        {
            let Some((mode, ignore_whitespace, collapsed_folders)) = self
                .overlays
                .git_diff_panel
                .as_ref()
                .filter(|panel| {
                    panel.project_id == project_id && panel.project_path == project_path
                })
                .map(|panel| {
                    (
                        panel.mode,
                        panel.ignore_whitespace,
                        panel.collapsed_folders.clone(),
                    )
                })
            else {
                return;
            };
            let task = cx.background_spawn({
                let project_path = project_path.clone();
                async move {
                    read_project_git_diff_result(&project_path, mode, ignore_whitespace).map(
                        |diff| {
                            let sidebar_rows = git_diff_sidebar_rows(&diff, &collapsed_folders);
                            (diff, sidebar_rows)
                        },
                    )
                }
            });
            cx.spawn_in(window, async move |this, cx| {
                let result = task.await;
                let _ = this.update_in(cx, |root, _window, cx| {
                    if root.overlays.git_diff_generation != generation {
                        return;
                    }
                    let Some(panel) = root.overlays.git_diff_panel.as_mut() else {
                        return;
                    };
                    if panel.project_id != project_id || panel.project_path != project_path {
                        return;
                    }
                    match result {
                        Ok((diff, sidebar_rows)) => {
                            panel.content = GitDiffPanelContent::Ready(Arc::new(diff));
                            panel.sidebar_rows = Arc::new(sidebar_rows);
                        }
                        Err(error) => {
                            panel.content = GitDiffPanelContent::Error(error);
                            panel.sidebar_rows = Arc::new(Vec::new());
                        }
                    }
                    sync_selected_git_diff(panel);
                    cx.notify();
                });
            })
            .detach();
        }
    }

    pub(super) fn render_git_diff_panel(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Div> {
        let theme = self.theme_runtime.ui;
        let editor_theme = self.theme_runtime.editor;
        let panel_background = git_diff_panel_background(theme);
        let editor_appearance = EditorAppearance::from(&self.app_settings.editor);
        let panel = self.overlays.git_diff_panel.as_mut()?;
        let focus_handle = panel
            .focus_handle
            .get_or_insert_with(|| cx.focus_handle())
            .clone();
        if !focus_handle.contains_focused(window, cx) {
            let deferred_focus_handle = focus_handle.clone();
            window.defer(cx, move |window, cx| {
                deferred_focus_handle.focus(window, cx);
            });
        }
        let eagerly_render_selected_file = match &panel.content {
            GitDiffPanelContent::Ready(result) => result
                .files
                .get(panel.selected_file)
                .is_some_and(should_eagerly_render_git_diff),
            GitDiffPanelContent::Loading | GitDiffPanelContent::Error(_) => false,
        };
        if eagerly_render_selected_file && panel.syntax_highlights.is_empty() {
            sync_git_diff_syntax_highlights(panel, &cx.theme().highlight_theme);
        }
        if eagerly_render_selected_file && panel.unified_view_rows.is_empty() {
            sync_git_diff_view_rows(panel, theme, editor_theme);
        }
        let content = panel.content.clone();
        let mode = panel.mode;
        let view_mode = panel.view_mode;
        let ignore_whitespace = panel.ignore_whitespace;
        let selected_file = panel.selected_file;
        let diff_scroll_handle = panel.diff_scroll_handle.clone();
        let file_scroll_handle = panel.file_scroll_handle.clone();
        let sidebar_rows = panel.sidebar_rows.clone();
        let split_rows = panel.split_rows.clone();
        let unified_view_rows = panel.unified_view_rows.clone();
        let split_left_view_rows = panel.split_left_view_rows.clone();
        let split_right_view_rows = panel.split_right_view_rows.clone();
        let unified_horizontal_scroll_handle = panel.unified_horizontal_scroll_handle.clone();
        let split_left_horizontal_scroll_handle = panel.split_left_horizontal_scroll_handle.clone();
        let split_right_horizontal_scroll_handle =
            panel.split_right_horizontal_scroll_handle.clone();
        let unified_content_width = panel.unified_content_width;
        let split_content_width = panel.split_content_width;
        let (left_source, right_source) = match mode {
            GitDiffMode::Unstaged => (
                self.ui_text.get(UiTextKey::GitDiffSourceIndex),
                self.ui_text.get(UiTextKey::GitDiffSourceWorkingTree),
            ),
            GitDiffMode::Staged => (
                self.ui_text.get(UiTextKey::GitDiffSourceHead),
                self.ui_text.get(UiTextKey::GitDiffSourceIndex),
            ),
        };
        let (file_count, total_added, total_removed) = match &content {
            GitDiffPanelContent::Ready(result) => (
                result.files.len(),
                result.total_added(),
                result.total_removed(),
            ),
            GitDiffPanelContent::Loading | GitDiffPanelContent::Error(_) => (0, 0, 0),
        };

        let body = match content {
            GitDiffPanelContent::Loading => git_diff_message(
                self.ui_text.get(UiTextKey::GitDiffLoading),
                theme.text_muted,
            ),
            GitDiffPanelContent::Error(error) => git_diff_message(error, theme.danger),
            GitDiffPanelContent::Ready(result) if result.files.is_empty() => {
                git_diff_message(self.ui_text.get(UiTextKey::GitDiffClean), theme.text_muted)
            }
            GitDiffPanelContent::Ready(result) => div()
                .flex()
                .flex_1()
                .min_h_0()
                .child(self.render_git_diff_sidebar(
                    result.clone(),
                    sidebar_rows,
                    selected_file,
                    &file_scroll_handle,
                    theme,
                    cx,
                ))
                .child(git_diff_code_pane(
                    result,
                    selected_file,
                    view_mode,
                    unified_view_rows,
                    split_left_view_rows,
                    split_right_view_rows,
                    split_rows,
                    unified_content_width,
                    split_content_width,
                    diff_scroll_handle,
                    unified_horizontal_scroll_handle,
                    split_left_horizontal_scroll_handle,
                    split_right_horizontal_scroll_handle,
                    self.ui_text.get(UiTextKey::GitDiffBinaryUnavailable),
                    left_source,
                    right_source,
                    theme,
                    editor_theme,
                    editor_appearance,
                ))
                .into_any_element(),
        };

        Some(
            div()
                .debug_selector(|| "git-diff-overlay".to_string())
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .bg(rgba(0x000000b3))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(
                    div()
                        .debug_selector(|| "git-diff-panel".to_string())
                        .track_focus(&focus_handle)
                        .flex()
                        .flex_col()
                        .w(relative(0.96))
                        .h(relative(0.90))
                        .min_h_0()
                        .rounded_lg()
                        .border_1()
                        .border_color(theme.border)
                        .bg(panel_background)
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .justify_between()
                                .gap_4()
                                .px_5()
                                .py_3()
                                .border_b_1()
                                .border_color(theme.border)
                                .bg(theme.surface_elevated)
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap_3()
                                        .min_w_0()
                                        .child(
                                            div()
                                                .font_semibold()
                                                .text_color(theme.text)
                                                .child(self.ui_text.get(UiTextKey::GitDiffTitle)),
                                        )
                                        .when(file_count > 0, |this| {
                                            this.child(
                                                div()
                                                    .h(px(18.0))
                                                    .border_l_1()
                                                    .border_color(theme.border),
                                            )
                                            .child(
                                                div().text_sm().text_color(theme.text_muted).child(
                                                    format!(
                                                        "{} {} ·",
                                                        file_count,
                                                        self.ui_text.get(if file_count == 1 {
                                                            UiTextKey::GitDiffFile
                                                        } else {
                                                            UiTextKey::GitDiffFiles
                                                        })
                                                    ),
                                                ),
                                            )
                                            .child(
                                                div()
                                                    .text_sm()
                                                    .text_color(theme.success)
                                                    .child(format!("+{total_added}")),
                                            )
                                            .child(
                                                div()
                                                    .text_sm()
                                                    .text_color(theme.danger)
                                                    .child(format!("-{total_removed}")),
                                            )
                                        }),
                                )
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap_2()
                                        .when(file_count > 0, |this| {
                                            this.child(git_diff_header_button(
                                                "git-diff-copy",
                                                self.ui_text.get(UiTextKey::GitDiffCopyHint),
                                                false,
                                                theme,
                                                cx.listener(|this, _, _window, cx| {
                                                    if let Some(text) =
                                                        this.selected_git_diff_text()
                                                    {
                                                        cx.write_to_clipboard(
                                                            ClipboardItem::new_string(text),
                                                        );
                                                    }
                                                }),
                                            ))
                                            .child(git_diff_separator(theme))
                                        })
                                        .child(git_diff_header_button(
                                            "git-diff-whitespace",
                                            self.ui_text.get(UiTextKey::GitDiffWhitespace),
                                            ignore_whitespace,
                                            theme,
                                            cx.listener(|this, _, _window, cx| {
                                                if this.toggle_git_diff_whitespace() {
                                                    cx.notify();
                                                }
                                            }),
                                        ))
                                        .child(git_diff_separator(theme))
                                        .child(
                                            div()
                                                .flex()
                                                .items_center()
                                                .rounded_md()
                                                .bg(theme.app_background)
                                                .child(git_diff_header_button(
                                                    "git-diff-unified",
                                                    self.ui_text.get(UiTextKey::GitDiffUnified),
                                                    view_mode == GitDiffViewMode::Unified,
                                                    theme,
                                                    cx.listener(|this, _, _window, cx| {
                                                        if this.set_git_diff_view_mode(
                                                            GitDiffViewMode::Unified,
                                                        ) {
                                                            cx.notify();
                                                        }
                                                    }),
                                                ))
                                                .child(git_diff_header_button(
                                                    "git-diff-split",
                                                    self.ui_text.get(UiTextKey::GitDiffSplit),
                                                    view_mode == GitDiffViewMode::Split,
                                                    theme,
                                                    cx.listener(|this, _, _window, cx| {
                                                        if this.set_git_diff_view_mode(
                                                            GitDiffViewMode::Split,
                                                        ) {
                                                            cx.notify();
                                                        }
                                                    }),
                                                )),
                                        )
                                        .child(
                                            div()
                                                .flex()
                                                .items_center()
                                                .rounded_md()
                                                .bg(theme.app_background)
                                                .child(git_diff_header_button(
                                                    "git-diff-unstaged",
                                                    self.ui_text.get(UiTextKey::GitDiffUnstaged),
                                                    mode == GitDiffMode::Unstaged,
                                                    theme,
                                                    cx.listener(|this, _, _window, cx| {
                                                        if this.set_git_diff_mode(
                                                            GitDiffMode::Unstaged,
                                                        ) {
                                                            cx.notify();
                                                        }
                                                    }),
                                                ))
                                                .child(git_diff_header_button(
                                                    "git-diff-staged",
                                                    self.ui_text.get(UiTextKey::GitDiffStaged),
                                                    mode == GitDiffMode::Staged,
                                                    theme,
                                                    cx.listener(|this, _, _window, cx| {
                                                        if this
                                                            .set_git_diff_mode(GitDiffMode::Staged)
                                                        {
                                                            cx.notify();
                                                        }
                                                    }),
                                                )),
                                        )
                                        .child(git_diff_separator(theme))
                                        .child(
                                            div()
                                                .id("git-diff-close")
                                                .debug_selector(|| "git-diff-close".to_string())
                                                .flex()
                                                .items_center()
                                                .justify_center()
                                                .size(px(28.0))
                                                .rounded_md()
                                                .cursor_pointer()
                                                .text_color(theme.text_muted)
                                                .hover(move |this| {
                                                    this.bg(theme.hover_surface)
                                                        .text_color(theme.text)
                                                })
                                                .on_click(cx.listener(|this, _, _window, cx| {
                                                    this.close_git_diff_panel();
                                                    cx.notify();
                                                }))
                                                .child("×"),
                                        ),
                                ),
                        )
                        .child(body)
                        .child(git_diff_footer(&self.ui_text, theme)),
                ),
        )
    }

    fn render_git_diff_sidebar(
        &self,
        result: Arc<GitDiffResult>,
        rows: Arc<Vec<GitDiffSidebarRow>>,
        selected_file: usize,
        scroll_handle: &UniformListScrollHandle,
        theme: WorkbenchTheme,
        cx: &mut Context<Self>,
    ) -> Div {
        let row_count = rows.len();
        let list_scroll_handle = scroll_handle.clone();

        div()
            .flex()
            .flex_col()
            .flex_none()
            .w(px(288.0))
            .h_full()
            .min_h_0()
            .border_r_1()
            .border_color(theme.border)
            .bg(theme.app_background)
            .child(
                div()
                    .flex()
                    .items_center()
                    .h(px(42.0))
                    .px_4()
                    .border_b_1()
                    .border_color(theme.border)
                    .text_sm()
                    .text_color(theme.text_muted)
                    .child(self.ui_text.get(UiTextKey::GitDiffFilesHeading)),
            )
            .child(
                div()
                    .id("git-diff-file-list")
                    .debug_selector(|| "git-diff-file-list".to_string())
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .child(
                        gpui::uniform_list(
                            "git-diff-file-list-rows",
                            row_count,
                            cx.processor(move |_this, range: Range<usize>, _window, cx| {
                                range
                                    .filter_map(|row_index| match rows.get(row_index)? {
                                        GitDiffSidebarRow::Folder {
                                            group_index,
                                            path,
                                            collapsed,
                                        } => {
                                            let folder = path.clone();
                                            let folder_for_click = folder.clone();
                                            let group_index = *group_index;
                                            let collapsed = *collapsed;
                                            Some(
                                                div()
                                                    .id(("git-diff-folder", group_index))
                                                    .debug_selector(move || {
                                                        format!("git-diff-folder-{group_index}")
                                                    })
                                                    .flex()
                                                    .items_center()
                                                    .gap_2()
                                                    .h(px(38.0))
                                                    .px_3()
                                                    .cursor_pointer()
                                                    .text_xs()
                                                    .text_color(theme.text_muted)
                                                    .hover(move |this| {
                                                        this.bg(theme.hover_surface)
                                                    })
                                                    .on_click(cx.listener(
                                                        move |this, _, _window, cx| {
                                                            if this.toggle_git_diff_folder(
                                                                folder_for_click.as_ref(),
                                                            ) {
                                                                cx.notify();
                                                            }
                                                        },
                                                    ))
                                                    .child(if collapsed { "▸" } else { "▾" })
                                                    .child(format!("{folder}/"))
                                                    .into_any_element(),
                                            )
                                        }
                                        GitDiffSidebarRow::File {
                                            file_index,
                                            filename,
                                        } => {
                                            let file_index = *file_index;
                                            let file = result.files.get(file_index)?;
                                            let selected = file_index == selected_file;
                                            let (status, status_color) = match file.change_kind() {
                                                GitFileChangeKind::Added => ("A", theme.success),
                                                GitFileChangeKind::Modified => {
                                                    ("M", theme.text_muted)
                                                }
                                                GitFileChangeKind::Deleted => ("D", theme.danger),
                                            };
                                            let filename = filename.clone();
                                            let added = file.added;
                                            let removed = file.removed;
                                            Some(
                                                div()
                                                    .id(("git-diff-file", file_index))
                                                    .debug_selector(move || {
                                                        format!("git-diff-file-{file_index}")
                                                    })
                                                    .flex()
                                                    .items_center()
                                                    .justify_between()
                                                    .gap_2()
                                                    .h(px(38.0))
                                                    .px_3()
                                                    .cursor_pointer()
                                                    .when(selected, |this| {
                                                        this.bg(theme.active_surface)
                                                    })
                                                    .hover(move |this| {
                                                        if selected {
                                                            this
                                                        } else {
                                                            this.bg(theme.hover_surface)
                                                        }
                                                    })
                                                    .on_click(cx.listener(
                                                        move |this, _, _window, cx| {
                                                            if this
                                                                .select_git_diff_file(file_index)
                                                            {
                                                                cx.notify();
                                                            }
                                                        },
                                                    ))
                                                    .child(
                                                        div()
                                                            .flex()
                                                            .items_center()
                                                            .gap_2()
                                                            .min_w_0()
                                                            .child(
                                                                div()
                                                                    .w(px(16.0))
                                                                    .text_xs()
                                                                    .text_color(status_color)
                                                                    .child(status),
                                                            )
                                                            .child(
                                                                div()
                                                                    .truncate()
                                                                    .text_sm()
                                                                    .text_color(theme.text)
                                                                    .child(filename),
                                                            ),
                                                    )
                                                    .child(
                                                        div()
                                                            .flex()
                                                            .items_center()
                                                            .gap_1()
                                                            .flex_none()
                                                            .children((added > 0).then(|| {
                                                                div()
                                                                    .text_xs()
                                                                    .text_color(theme.success)
                                                                    .child(format!("+{added}"))
                                                            }))
                                                            .children((removed > 0).then(|| {
                                                                div()
                                                                    .text_xs()
                                                                    .text_color(theme.danger)
                                                                    .child(format!("-{removed}"))
                                                            })),
                                                    )
                                                    .when(selected, |this| {
                                                        this.child(
                                                            div()
                                                                .debug_selector(move || {
                                                                    format!(
                                                                        "git-diff-selected-file-{file_index}"
                                                                    )
                                                                })
                                                                .size(px(0.0)),
                                                        )
                                                    })
                                                    .into_any_element(),
                                            )
                                        }
                                    })
                                    .collect()
                            }),
                        )
                        .w_full()
                        .flex_1()
                        .h_full()
                        .py_2()
                        .track_scroll(&list_scroll_handle),
                    ),
            )
    }

    fn selected_project_git_target(&self) -> Result<(ProjectId, PathBuf), WorkbenchError> {
        let project_id = self
            .workspace
            .selected_project_id()
            .cloned()
            .ok_or(WorkspaceError::NoSelectedProject)?;
        let project_path = self
            .workspace
            .project(&project_id)
            .map(|project| project.path.clone())
            .ok_or_else(|| WorkspaceError::ProjectNotFound(project_id.as_str().to_string()))?;
        Ok((project_id, project_path))
    }
}

fn git_branch_state_item(title: &str, subtitle: Option<String>) -> PaletteItem {
    PaletteItem {
        id: "git-branch-state".to_string(),
        title: title.to_string(),
        subtitle,
        status: None,
        keybinding: None,
        command: CommandId::GitBranchSwitch,
        enabled: false,
        disabled_reason: None,
    }
}

fn should_eagerly_render_git_diff(file: &GitFileDiff) -> bool {
    file.line_count() > 0
        && file.line_count() <= MAX_EAGER_DIFF_LINES
        && file.content_bytes() <= MAX_EAGER_DIFF_BYTES
}

fn git_diff_sidebar_rows(
    result: &GitDiffResult,
    collapsed_folders: &BTreeSet<String>,
) -> Vec<GitDiffSidebarRow> {
    let mut groups = BTreeMap::<String, Vec<usize>>::new();
    for (index, file) in result.files.iter().enumerate() {
        let parent = Path::new(file.path())
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .map(|parent| parent.to_string_lossy().into_owned())
            .unwrap_or_default();
        groups.entry(parent).or_default().push(index);
    }

    let mut rows = Vec::with_capacity(result.files.len() + groups.len());
    for (group_index, (folder, indices)) in groups.into_iter().enumerate() {
        let collapsed = collapsed_folders.contains(&folder);
        if !folder.is_empty() {
            rows.push(GitDiffSidebarRow::Folder {
                group_index,
                path: folder.clone().into(),
                collapsed,
            });
        }
        if collapsed {
            continue;
        }
        rows.extend(indices.into_iter().map(|file_index| {
            let file = &result.files[file_index];
            let filename = Path::new(file.path())
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| file.path().to_string());
            GitDiffSidebarRow::File {
                file_index,
                filename: filename.into(),
            }
        }));
    }
    rows
}

fn sync_git_diff_sidebar(panel: &mut GitDiffPanel) {
    let GitDiffPanelContent::Ready(result) = &panel.content else {
        panel.sidebar_rows = Arc::new(Vec::new());
        return;
    };
    panel.sidebar_rows = Arc::new(git_diff_sidebar_rows(result, &panel.collapsed_folders));
}

fn sync_selected_git_diff(panel: &mut GitDiffPanel) {
    panel.syntax_highlights = Arc::new(Vec::new());
    panel.unified_view_rows = Arc::new(Vec::new());
    panel.split_left_view_rows = Arc::new(Vec::new());
    panel.split_right_view_rows = Arc::new(Vec::new());
    panel.unified_horizontal_scroll_handle = ScrollHandle::new();
    panel.split_left_horizontal_scroll_handle = ScrollHandle::new();
    panel.split_right_horizontal_scroll_handle = ScrollHandle::new();
    panel.unified_content_width = 900.0;
    panel.split_content_width = 700.0;
    let GitDiffPanelContent::Ready(result) = &panel.content else {
        panel.split_rows = Arc::new(Vec::new());
        return;
    };
    if result.files.is_empty() {
        panel.selected_file = 0;
        panel.split_rows = Arc::new(Vec::new());
        return;
    }
    panel.selected_file = panel.selected_file.min(result.files.len() - 1);
    let file = &result.files[panel.selected_file];
    let max_line_chars = file.max_line_chars() as f32;
    panel.unified_content_width = (max_line_chars * 8.0 + 150.0).max(900.0);
    panel.split_content_width = (max_line_chars * 8.0 + 100.0).max(700.0);
    panel.split_rows = Arc::new(git_split_rows(file.line_count(), |index| {
        file.line(index).map(|line| line.kind)
    }));
}

fn sync_git_diff_syntax_highlights(
    panel: &mut GitDiffPanel,
    theme: &gpui_component::highlighter::HighlightTheme,
) {
    let GitDiffPanelContent::Ready(result) = &panel.content else {
        return;
    };
    let Some(file) = result.files.get(panel.selected_file) else {
        return;
    };
    let line_count = file.line_count();
    let resolution = EditorLanguageCatalog::builtin().resolve_for_path(file.path(), None);
    let mut source = String::with_capacity(file.content_bytes() + line_count);
    let mut line_ranges = Vec::with_capacity(line_count);
    for line in file.lines() {
        let start = source.len();
        source.push_str(&line.content);
        let end = source.len();
        source.push('\n');
        line_ranges.push(start..end);
    }
    let rope = Rope::from(source.as_str());
    let mut highlighter = SyntaxHighlighter::new(&resolution.highlighter_name);
    highlighter.update(None, &rope, None);
    let mut highlights = Vec::with_capacity(line_count);
    for range in line_ranges {
        let line_start = range.start;
        let line_end = range.end;
        let styles = highlighter
            .styles(&(line_start..line_end), theme)
            .into_iter()
            .filter_map(|(style_range, style)| {
                let start = style_range.start.max(line_start) - line_start;
                let end = style_range.end.min(line_end).saturating_sub(line_start);
                (start < end).then_some((start..end, style))
            })
            .collect();
        highlights.push(styles);
    }
    panel.syntax_highlights = Arc::new(highlights);
}

fn sync_git_diff_view_rows(
    panel: &mut GitDiffPanel,
    theme: WorkbenchTheme,
    editor_theme: EditorTheme,
) {
    let GitDiffPanelContent::Ready(result) = &panel.content else {
        return;
    };
    let Some(file) = result.files.get(panel.selected_file) else {
        return;
    };
    let mut unified_rows = Vec::with_capacity(file.line_count());
    for (index, line) in file.lines().enumerate() {
        unified_rows.push(git_diff_view_row(
            line,
            Arc::new(
                panel
                    .syntax_highlights
                    .get(index)
                    .cloned()
                    .unwrap_or_default(),
            ),
            theme,
            editor_theme,
        ));
    }

    let mut left_rows = Vec::with_capacity(panel.split_rows.len());
    let mut right_rows = Vec::with_capacity(panel.split_rows.len());
    for row in panel.split_rows.iter() {
        match row {
            GitSplitRow::Hunk(line_index) => {
                let Some(line) = file.line(*line_index) else {
                    continue;
                };
                left_rows.push(ReadonlyCodeRow::hunk(
                    line.content.clone(),
                    editor_theme.active_line,
                    editor_theme.active_line_number,
                ));
                right_rows.push(ReadonlyCodeRow::hunk(
                    line.content.clone(),
                    editor_theme.active_line,
                    editor_theme.active_line_number,
                ));
            }
            GitSplitRow::Lines { left, right } => {
                let left_highlights = Arc::new(
                    left.and_then(|index| panel.syntax_highlights.get(index).cloned())
                        .unwrap_or_default(),
                );
                let right_highlights = Arc::new(
                    right
                        .and_then(|index| panel.syntax_highlights.get(index).cloned())
                        .unwrap_or_default(),
                );
                left_rows.push(git_split_side_view_row(
                    *left,
                    true,
                    file,
                    left_highlights,
                    theme,
                    editor_theme,
                ));
                right_rows.push(git_split_side_view_row(
                    *right,
                    false,
                    file,
                    right_highlights,
                    theme,
                    editor_theme,
                ));
            }
        }
    }

    panel.unified_view_rows = Arc::new(unified_rows);
    panel.split_left_view_rows = Arc::new(left_rows);
    panel.split_right_view_rows = Arc::new(right_rows);
}

fn git_diff_view_row(
    line: &GitDiffLine,
    highlights: Arc<Vec<(Range<usize>, HighlightStyle)>>,
    theme: WorkbenchTheme,
    editor_theme: EditorTheme,
) -> ReadonlyCodeRow {
    if line.kind == GitDiffLineKind::Hunk {
        return ReadonlyCodeRow::hunk(
            line.content.clone(),
            editor_theme.active_line,
            editor_theme.active_line_number,
        );
    }
    ReadonlyCodeRow::code(
        [line.old_line, line.new_line],
        git_diff_line_prefix(line.kind),
        line.content.clone(),
        highlights,
        git_diff_line_background(line.kind, theme, editor_theme),
        git_diff_line_accent(line.kind, theme),
    )
}

fn git_split_side_view_row(
    line_index: Option<usize>,
    left: bool,
    file: &GitFileDiff,
    highlights: Arc<Vec<(Range<usize>, HighlightStyle)>>,
    theme: WorkbenchTheme,
    editor_theme: EditorTheme,
) -> ReadonlyCodeRow {
    let Some(line) = line_index.and_then(|index| file.line(index)) else {
        return ReadonlyCodeRow::phantom(editor_theme.background);
    };
    ReadonlyCodeRow::code(
        [if left { line.old_line } else { line.new_line }, None],
        git_diff_line_prefix(line.kind),
        line.content.clone(),
        highlights,
        git_diff_line_background(line.kind, theme, editor_theme),
        git_diff_line_accent(line.kind, theme),
    )
}

fn git_diff_line_prefix(kind: GitDiffLineKind) -> &'static str {
    match kind {
        GitDiffLineKind::Added => "+",
        GitDiffLineKind::Removed => "-",
        GitDiffLineKind::Context => " ",
        GitDiffLineKind::Hunk => "",
    }
}

fn git_split_rows(
    line_count: usize,
    mut line_kind: impl FnMut(usize) -> Option<GitDiffLineKind>,
) -> Vec<GitSplitRow> {
    let mut rows = Vec::new();
    let mut index = 0;
    while index < line_count {
        let Some(kind) = line_kind(index) else {
            index += 1;
            continue;
        };
        match kind {
            GitDiffLineKind::Hunk => {
                rows.push(GitSplitRow::Hunk(index));
                index += 1;
            }
            GitDiffLineKind::Context => {
                rows.push(GitSplitRow::Lines {
                    left: Some(index),
                    right: Some(index),
                });
                index += 1;
            }
            GitDiffLineKind::Removed => {
                let removed_start = index;
                while index < line_count && line_kind(index) == Some(GitDiffLineKind::Removed) {
                    index += 1;
                }
                let added_start = index;
                while index < line_count && line_kind(index) == Some(GitDiffLineKind::Added) {
                    index += 1;
                }
                let removed_len = added_start - removed_start;
                let added_len = index - added_start;
                for row in 0..removed_len.max(added_len) {
                    rows.push(GitSplitRow::Lines {
                        left: (row < removed_len).then_some(removed_start + row),
                        right: (row < added_len).then_some(added_start + row),
                    });
                }
            }
            GitDiffLineKind::Added => {
                rows.push(GitSplitRow::Lines {
                    left: None,
                    right: Some(index),
                });
                index += 1;
            }
        }
    }
    rows
}

fn git_diff_code_pane(
    result: Arc<GitDiffResult>,
    selected_file: usize,
    view_mode: GitDiffViewMode,
    unified_rows: Arc<Vec<ReadonlyCodeRow>>,
    split_left_rows: Arc<Vec<ReadonlyCodeRow>>,
    split_right_rows: Arc<Vec<ReadonlyCodeRow>>,
    split_rows: Arc<Vec<GitSplitRow>>,
    unified_content_width: f32,
    split_content_width: f32,
    vertical_scroll: UniformListScrollHandle,
    unified_horizontal_scroll: ScrollHandle,
    split_left_horizontal_scroll: ScrollHandle,
    split_right_horizontal_scroll: ScrollHandle,
    binary_unavailable: &'static str,
    left_source: &'static str,
    right_source: &'static str,
    theme: WorkbenchTheme,
    editor_theme: EditorTheme,
    editor_appearance: EditorAppearance,
) -> Div {
    let Some(file) = result.files.get(selected_file) else {
        return div().flex().flex_1();
    };
    let file_path = file.path().to_string();
    let binary = file.binary;
    let font_scale = (editor_appearance.font_size / EditorAppearance::default().font_size).max(0.5);
    let unified_content_width = (unified_content_width * font_scale).max(900.0);
    let split_content_width = (split_content_width * font_scale).max(700.0);
    let file_header = div()
        .flex()
        .items_center()
        .h(px(42.0))
        .px_5()
        .border_b_1()
        .border_color(theme.border)
        .bg(editor_theme.active_line)
        .text_size(px(editor_appearance.font_size))
        .line_height(relative(editor_appearance.line_height))
        .font_family(editor_appearance.resolved_font_family())
        .text_color(editor_theme.line_number)
        .child(file_path);

    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .min_h_0()
        .bg(editor_theme.background)
        .child(file_header)
        .when(binary, |this| {
            this.child(
                div()
                    .flex()
                    .flex_1()
                    .items_center()
                    .justify_center()
                    .text_color(editor_theme.line_number)
                    .child(binary_unavailable),
            )
        })
        .when(!binary && view_mode == GitDiffViewMode::Unified, |this| {
            this.child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .child(git_diff_unified_code_view(
                        result.clone(),
                        selected_file,
                        unified_rows,
                        vertical_scroll.clone(),
                        unified_horizontal_scroll,
                        editor_appearance.clone(),
                        theme,
                        editor_theme,
                        unified_content_width,
                    )),
            )
        })
        .when(!binary && view_mode == GitDiffViewMode::Split, |this| {
            this.child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .child(
                        div()
                            .flex()
                            .flex_none()
                            .h(px(34.0))
                            .border_b_1()
                            .border_color(theme.border)
                            .child(
                                git_diff_source_header(
                                    "git-diff-split-left-header",
                                    left_source,
                                    &editor_appearance,
                                    editor_theme,
                                )
                                .flex_basis(relative(0.5))
                                .flex_shrink(1.0),
                            )
                            .child(div().w(px(1.0)).h_full().flex_none().bg(theme.border))
                            .child(
                                git_diff_source_header(
                                    "git-diff-split-right-header",
                                    right_source,
                                    &editor_appearance,
                                    editor_theme,
                                )
                                .flex_basis(relative(0.5))
                                .flex_shrink(1.0),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_1()
                            .min_h_0()
                            .min_w_0()
                            .child(
                                div()
                                    .debug_selector(|| "git-diff-split-left-pane".to_string())
                                    .flex()
                                    .flex_basis(relative(0.5))
                                    .flex_shrink(1.0)
                                    .min_w_0()
                                    .overflow_hidden()
                                    .child(git_diff_split_code_view(
                                        result.clone(),
                                        selected_file,
                                        split_rows.clone(),
                                        split_left_rows,
                                        true,
                                        vertical_scroll.clone(),
                                        split_left_horizontal_scroll,
                                        editor_appearance.clone(),
                                        theme,
                                        editor_theme,
                                        split_content_width,
                                        "git-diff-split-left",
                                        "git-diff-split-left-row",
                                    )),
                            )
                            .child(div().w(px(1.0)).h_full().flex_none().bg(theme.border))
                            .child(
                                div()
                                    .debug_selector(|| "git-diff-split-right-pane".to_string())
                                    .flex()
                                    .flex_basis(relative(0.5))
                                    .flex_shrink(1.0)
                                    .min_w_0()
                                    .overflow_hidden()
                                    .child(git_diff_split_code_view(
                                        result.clone(),
                                        selected_file,
                                        split_rows,
                                        split_right_rows,
                                        false,
                                        vertical_scroll,
                                        split_right_horizontal_scroll,
                                        editor_appearance,
                                        theme,
                                        editor_theme,
                                        split_content_width,
                                        "git-diff-split-right",
                                        "git-diff-split-right-row",
                                    )),
                            ),
                    ),
            )
        })
}

fn git_diff_unified_code_view(
    result: Arc<GitDiffResult>,
    selected_file: usize,
    eager_rows: Arc<Vec<ReadonlyCodeRow>>,
    vertical_scroll: UniformListScrollHandle,
    horizontal_scroll: ScrollHandle,
    appearance: EditorAppearance,
    theme: WorkbenchTheme,
    editor_theme: EditorTheme,
    content_width: f32,
) -> ReadonlyCodeView {
    let eager = result
        .files
        .get(selected_file)
        .is_some_and(should_eagerly_render_git_diff);
    let view = if eager {
        ReadonlyCodeView::new(
            "git-diff-unified",
            eager_rows,
            vertical_scroll,
            horizontal_scroll,
            appearance,
            editor_theme,
            theme.border,
        )
    } else {
        let row_count = result
            .files
            .get(selected_file)
            .map(GitFileDiff::line_count)
            .unwrap_or_default();
        let empty_highlights = Arc::new(Vec::new());
        ReadonlyCodeView::new_lazy(
            "git-diff-unified",
            row_count,
            move |index| {
                let line = result.files.get(selected_file)?.line(index)?;
                Some(git_diff_view_row(
                    line,
                    empty_highlights.clone(),
                    theme,
                    editor_theme,
                ))
            },
            vertical_scroll,
            horizontal_scroll,
            appearance,
            editor_theme,
            theme.border,
        )
    };
    view.number_columns(2)
        .content_width(content_width)
        .row_debug_prefix("git-diff-line")
}

#[allow(clippy::too_many_arguments)]
fn git_diff_split_code_view(
    result: Arc<GitDiffResult>,
    selected_file: usize,
    split_rows: Arc<Vec<GitSplitRow>>,
    eager_rows: Arc<Vec<ReadonlyCodeRow>>,
    left: bool,
    vertical_scroll: UniformListScrollHandle,
    horizontal_scroll: ScrollHandle,
    appearance: EditorAppearance,
    theme: WorkbenchTheme,
    editor_theme: EditorTheme,
    content_width: f32,
    id: &'static str,
    row_debug_prefix: &'static str,
) -> ReadonlyCodeView {
    let eager = result
        .files
        .get(selected_file)
        .is_some_and(should_eagerly_render_git_diff);
    let view = if eager {
        ReadonlyCodeView::new(
            id,
            eager_rows,
            vertical_scroll,
            horizontal_scroll,
            appearance,
            editor_theme,
            theme.border,
        )
    } else {
        let row_count = split_rows.len();
        let empty_highlights = Arc::new(Vec::new());
        ReadonlyCodeView::new_lazy(
            id,
            row_count,
            move |index| {
                let file = result.files.get(selected_file)?;
                match split_rows.get(index)? {
                    GitSplitRow::Hunk(line_index) => {
                        let line = file.line(*line_index)?;
                        Some(ReadonlyCodeRow::hunk(
                            line.content.clone(),
                            editor_theme.active_line,
                            editor_theme.active_line_number,
                        ))
                    }
                    GitSplitRow::Lines {
                        left: left_line,
                        right: right_line,
                    } => Some(git_split_side_view_row(
                        if left { *left_line } else { *right_line },
                        left,
                        file,
                        empty_highlights.clone(),
                        theme,
                        editor_theme,
                    )),
                }
            },
            vertical_scroll,
            horizontal_scroll,
            appearance,
            editor_theme,
            theme.border,
        )
    };
    view.content_width(content_width)
        .row_debug_prefix(row_debug_prefix)
}

fn git_diff_source_header(
    debug_selector: &'static str,
    label: &'static str,
    appearance: &EditorAppearance,
    theme: EditorTheme,
) -> Div {
    div()
        .debug_selector(move || debug_selector.to_string())
        .flex()
        .items_center()
        .min_w_0()
        .px_4()
        .bg(theme.active_line)
        .text_size(px(appearance.font_size))
        .line_height(relative(appearance.line_height))
        .font_family(appearance.resolved_font_family())
        .text_color(theme.active_line_number)
        .child(label)
}

fn git_diff_line_background(
    kind: GitDiffLineKind,
    theme: WorkbenchTheme,
    editor_theme: EditorTheme,
) -> Rgba {
    match kind {
        GitDiffLineKind::Added => with_alpha(theme.success, 0.13),
        GitDiffLineKind::Removed => with_alpha(theme.danger, 0.13),
        GitDiffLineKind::Context | GitDiffLineKind::Hunk => editor_theme.background,
    }
}

fn git_diff_line_accent(kind: GitDiffLineKind, theme: WorkbenchTheme) -> Rgba {
    match kind {
        GitDiffLineKind::Added => theme.success,
        GitDiffLineKind::Removed => theme.danger,
        GitDiffLineKind::Context | GitDiffLineKind::Hunk => rgba(0x00000000),
    }
}

fn with_alpha(mut color: Rgba, alpha: f32) -> Rgba {
    color.a = alpha;
    color
}

fn git_diff_panel_background(theme: WorkbenchTheme) -> Rgba {
    yttt_panel_style(YtttPanelKind::Editor, theme).background
}

fn git_diff_message(message: impl Into<SharedString>, color: Rgba) -> AnyElement {
    div()
        .flex()
        .flex_1()
        .items_center()
        .justify_center()
        .px_6()
        .text_color(color)
        .child(message.into())
        .into_any_element()
}

fn git_diff_header_button<H>(
    id: &'static str,
    label: &'static str,
    active: bool,
    theme: WorkbenchTheme,
    on_click: H,
) -> Stateful<Div>
where
    H: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
{
    div()
        .id(id)
        .debug_selector(move || id.to_string())
        .flex()
        .items_center()
        .justify_center()
        .h(px(30.0))
        .px_3()
        .rounded_md()
        .cursor_pointer()
        .bg(if active {
            theme.accent
        } else {
            rgba(0x00000000)
        })
        .text_sm()
        .text_color(if active { theme.text } else { theme.text_muted })
        .hover(move |this| {
            if active {
                this
            } else {
                this.bg(theme.hover_surface).text_color(theme.text)
            }
        })
        .on_click(on_click)
        .child(label)
}

fn git_diff_separator(theme: WorkbenchTheme) -> Div {
    div().w(px(1.0)).h(px(22.0)).mx_1().bg(theme.border)
}

fn git_diff_footer(text: &UiText, theme: WorkbenchTheme) -> Div {
    div()
        .flex()
        .items_center()
        .gap_5()
        .h(px(46.0))
        .px_4()
        .border_t_1()
        .border_color(theme.border)
        .bg(theme.surface_elevated)
        .child(git_diff_hint(
            "Esc",
            text.get(UiTextKey::GitDiffCloseHint),
            theme,
        ))
        .child(git_diff_hint(
            "Tab",
            text.get(UiTextKey::GitDiffStageHint),
            theme,
        ))
        .child(git_diff_hint(
            "S",
            text.get(UiTextKey::GitDiffSplitHint),
            theme,
        ))
        .child(git_diff_hint(
            "↑↓",
            text.get(UiTextKey::GitDiffNavigateHint),
            theme,
        ))
        .child(git_diff_hint(
            if cfg!(target_os = "macos") {
                "⌘C"
            } else {
                "Ctrl+C"
            },
            text.get(UiTextKey::GitDiffCopyHint),
            theme,
        ))
}

fn git_diff_hint(key: &str, action: &'static str, theme: WorkbenchTheme) -> Div {
    div()
        .flex()
        .items_center()
        .gap_1()
        .child(
            div()
                .px_2()
                .py_1()
                .rounded_sm()
                .border_1()
                .border_color(theme.border)
                .bg(theme.app_background)
                .text_xs()
                .text_color(theme.text_muted)
                .child(key.to_string()),
        )
        .child(div().text_xs().text_color(theme.text_muted).child(action))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn diff_line(
        kind: GitDiffLineKind,
        old_line: Option<usize>,
        new_line: Option<usize>,
        content: &str,
    ) -> GitDiffLine {
        GitDiffLine {
            kind,
            old_line,
            new_line,
            content: content.to_string(),
        }
    }

    #[test]
    fn diff_panel_background_is_opaque_for_translucent_window_themes() {
        let mut theme = WorkbenchTheme::one_dark();
        theme.surface.a = 0.04;

        assert_eq!(git_diff_panel_background(theme), theme.surface.alpha(1.0));
    }

    #[test]
    fn split_rows_pair_replacements_and_preserve_unbalanced_sides() {
        let lines = [
            diff_line(GitDiffLineKind::Hunk, None, None, "@@ -1,3 +1,2 @@"),
            diff_line(GitDiffLineKind::Context, Some(1), Some(1), "same"),
            diff_line(GitDiffLineKind::Removed, Some(2), None, "old one"),
            diff_line(GitDiffLineKind::Removed, Some(3), None, "old two"),
            diff_line(GitDiffLineKind::Added, None, Some(2), "new one"),
        ];
        let rows = git_split_rows(lines.len(), |index| lines.get(index).map(|line| line.kind));

        assert!(matches!(&rows[0], GitSplitRow::Hunk(0)));
        assert!(matches!(
            &rows[1],
            GitSplitRow::Lines {
                left: Some(1),
                right: Some(1)
            }
        ));
        assert!(matches!(
            &rows[2],
            GitSplitRow::Lines {
                left: Some(2),
                right: Some(4)
            }
        ));
        assert!(matches!(
            &rows[3],
            GitSplitRow::Lines {
                left: Some(3),
                right: None
            }
        ));
    }
}
