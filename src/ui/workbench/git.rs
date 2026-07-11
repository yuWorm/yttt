use super::*;
use gpui_component::StyledExt as _;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum GitDiffLineTone {
    Normal,
    Added,
    Removed,
    Header,
    Muted,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct GitDiffLine {
    pub(super) text: SharedString,
    pub(super) tone: GitDiffLineTone,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum GitDiffPanelContent {
    Loading,
    Ready(Vec<GitDiffLine>),
    Error(String),
}

#[derive(Clone, Debug)]
pub(super) struct GitDiffPanel {
    pub(super) project_id: ProjectId,
    pub(super) project_path: PathBuf,
    pub(super) branch: Option<String>,
    pub(super) content: GitDiffPanelContent,
    pub(super) scroll_handle: ScrollHandle,
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
        let branch = self
            .project
            .project_git_statuses
            .get(&project_id)
            .and_then(|status| status.branch.clone());
        self.overlays.git_diff_panel = Some(GitDiffPanel {
            project_id: project_id.clone(),
            project_path: project_path.clone(),
            branch,
            content: GitDiffPanelContent::Loading,
            scroll_handle: ScrollHandle::new(),
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
            let task = cx.background_spawn({
                let project_path = project_path.clone();
                async move { read_project_git_diff(&project_path) }
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
                    panel.content = match result {
                        Ok(diff) => GitDiffPanelContent::Ready(git_diff_lines(&diff)),
                        Err(error) => GitDiffPanelContent::Error(error),
                    };
                    cx.notify();
                });
            })
            .detach();
        }
    }

    pub(super) fn render_git_diff_panel(&mut self, cx: &mut Context<Self>) -> Option<Div> {
        let panel = self.overlays.git_diff_panel.as_ref()?;
        let path = panel.project_path.display().to_string();
        let branch = panel.branch.clone();
        let scroll_handle = panel.scroll_handle.clone();
        let theme = self.theme_runtime.ui;
        let body = match &panel.content {
            GitDiffPanelContent::Loading => div()
                .flex()
                .flex_1()
                .items_center()
                .justify_center()
                .text_color(theme.text_muted)
                .child(self.ui_text.get(UiTextKey::GitDiffLoading))
                .into_any_element(),
            GitDiffPanelContent::Error(error) => div()
                .flex()
                .flex_1()
                .items_center()
                .justify_center()
                .px_6()
                .text_color(theme.danger)
                .child(error.clone())
                .into_any_element(),
            GitDiffPanelContent::Ready(lines) if lines.is_empty() => div()
                .flex()
                .flex_1()
                .items_center()
                .justify_center()
                .text_color(theme.text_muted)
                .child(self.ui_text.get(UiTextKey::GitDiffClean))
                .into_any_element(),
            GitDiffPanelContent::Ready(lines) => div()
                .id("git-diff-scroll")
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .track_scroll(&scroll_handle)
                .children(lines.iter().map(|line| {
                    div()
                        .flex_none()
                        .min_w_full()
                        .h(px(20.0))
                        .px_4()
                        .whitespace_nowrap()
                        .text_xs()
                        .text_color(match line.tone {
                            GitDiffLineTone::Normal => theme.text,
                            GitDiffLineTone::Added => theme.success,
                            GitDiffLineTone::Removed => theme.danger,
                            GitDiffLineTone::Header => theme.accent,
                            GitDiffLineTone::Muted => theme.text_muted,
                        })
                        .child(line.text.clone())
                }))
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
                .bg(rgba(0x00000099))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(
                    div()
                        .debug_selector(|| "git-diff-panel".to_string())
                        .flex()
                        .flex_col()
                        .w(relative(0.88))
                        .h(relative(0.82))
                        .min_h_0()
                        .rounded_lg()
                        .border_1()
                        .border_color(theme.border)
                        .bg(theme.surface_elevated)
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .justify_between()
                                .gap_3()
                                .px_4()
                                .py_3()
                                .border_b_1()
                                .border_color(theme.border)
                                .child(
                                    div()
                                        .flex()
                                        .flex_col()
                                        .min_w_0()
                                        .child(
                                            div()
                                                .font_semibold()
                                                .text_color(theme.text)
                                                .child(self.ui_text.get(UiTextKey::GitDiffTitle)),
                                        )
                                        .child(
                                            div()
                                                .truncate()
                                                .text_xs()
                                                .text_color(theme.text_muted)
                                                .child(match branch {
                                                    Some(branch) => format!("{path} · {branch}"),
                                                    None => path,
                                                }),
                                        ),
                                )
                                .child(
                                    yttt_button(
                                        "git-diff-close",
                                        self.ui_text.get(UiTextKey::PaletteClose),
                                        YtttButtonVariant::Secondary,
                                        theme,
                                        cx,
                                    )
                                    .on_click(cx.listener(
                                        |this, _, _window, cx| {
                                            this.close_git_diff_panel();
                                            cx.notify();
                                        },
                                    )),
                                ),
                        )
                        .child(body),
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

fn git_diff_lines(diff: &str) -> Vec<GitDiffLine> {
    diff.lines()
        .map(|line| GitDiffLine {
            text: line.to_string().into(),
            tone: if line.starts_with("@@") {
                GitDiffLineTone::Header
            } else if line.starts_with('+') && !line.starts_with("+++") {
                GitDiffLineTone::Added
            } else if line.starts_with('-') && !line.starts_with("---") {
                GitDiffLineTone::Removed
            } else if line.starts_with("diff --git")
                || line.starts_with("index ")
                || line.starts_with("---")
                || line.starts_with("+++")
            {
                GitDiffLineTone::Muted
            } else {
                GitDiffLineTone::Normal
            },
        })
        .collect()
}
