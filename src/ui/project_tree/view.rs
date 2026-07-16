use std::{
    collections::HashMap,
    fmt::Write as _,
    path::{Path, PathBuf},
    sync::Arc,
};

use gpui::{
    App, AppContext as _, Context, Entity, EventEmitter, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, Styled as _, Subscription, Window, div, px,
};
use gpui_component::{
    ActiveTheme as _, Sizable as _,
    input::{Escape, Input, InputEvent, InputState},
    list::ListItem,
    menu::PopupMenuItem,
    tree::{TreeItem, TreeState, tree},
};

use crate::{
    runtime::git_status::{GitFileStatus, ProjectGitStatus},
    ui::{
        interaction::actions::{
            ProjectTreeCopy, ProjectTreeCut, ProjectTreeDelete, ProjectTreeNewDirectory,
            ProjectTreeNewFile, ProjectTreePaste, ProjectTreeRename,
        },
        theme::icons::{IconTheme, icon_for_visual},
    },
};

use super::{ProjectFileTree, ProjectTreeEntryKind, ProjectTreeLoadState, ProjectTreeVisibleRow};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProjectTreeViewEvent {
    SelectPath(PathBuf),
    ToggleDirectory { path: PathBuf, expanded: bool },
    OpenFile(PathBuf),
    CreateProjectLayout,
    CreateEntry { parent: PathBuf, input: String },
    RenameEntry { path: PathBuf, new_name: String },
    RequestDelete(PathBuf),
    CopyEntry(PathBuf),
    CutEntry(PathBuf),
    PasteEntry { destination_directory: PathBuf },
    SetShowHidden(bool),
    Refresh,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectTreeRenderText {
    pub loading: String,
    pub empty_directory: String,
    pub retry: String,
}

impl Default for ProjectTreeRenderText {
    fn default() -> Self {
        Self {
            loading: "Loading…".to_string(),
            empty_directory: "Empty directory".to_string(),
            retry: "Retry".to_string(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectTreeInteractionText {
    pub new_file: String,
    pub new_directory: String,
    pub create_project_layout: String,
    pub rename: String,
    pub delete: String,
    pub copy: String,
    pub cut: String,
    pub paste: String,
    pub show_hidden: String,
    pub hide_hidden: String,
    pub entry_placeholder: String,
}

impl Default for ProjectTreeInteractionText {
    fn default() -> Self {
        Self {
            new_file: "New File".to_string(),
            new_directory: "New Folder".to_string(),
            create_project_layout: "Create Project Layout".to_string(),
            rename: "Rename".to_string(),
            delete: "Delete".to_string(),
            copy: "Copy".to_string(),
            cut: "Cut".to_string(),
            paste: "Paste".to_string(),
            show_hidden: "Show Hidden Files".to_string(),
            hide_hidden: "Hide Hidden Files".to_string(),
            entry_placeholder: "name or path; end with / for a folder".to_string(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectTreeRenderRow {
    pub id: String,
    pub relative_path: Option<PathBuf>,
    pub label: String,
    pub kind: Option<ProjectTreeEntryKind>,
    pub depth: usize,
    pub expanded: bool,
    pub selected: bool,
    pub load_state: ProjectTreeLoadState,
    pub git_status: Option<GitFileStatus>,
    pub synthetic: bool,
}

#[derive(Clone)]
pub struct ProjectTreeRenderSnapshot {
    tree_items: Vec<TreeItem>,
    visible_rows: Vec<ProjectTreeRenderRow>,
    rows_by_id: Arc<HashMap<String, ProjectTreeRenderRow>>,
    selected_index: Option<usize>,
}

impl ProjectTreeRenderSnapshot {
    pub fn from_tree(tree: &ProjectFileTree, git_status: Option<&ProjectGitStatus>) -> Self {
        Self::from_tree_with_text(tree, git_status, &ProjectTreeRenderText::default())
    }

    pub fn from_tree_with_text(
        tree: &ProjectFileTree,
        git_status: Option<&ProjectGitStatus>,
        text: &ProjectTreeRenderText,
    ) -> Self {
        let source_rows = tree.visible_rows();
        let mut index = 0;
        let nodes = build_nodes(&source_rows, &mut index, 0, git_status, text);
        let tree_items = nodes.iter().map(RenderNode::tree_item).collect::<Vec<_>>();
        let mut visible_rows = Vec::new();
        let mut rows_by_id = HashMap::new();
        for node in &nodes {
            node.collect_rows(&mut visible_rows, &mut rows_by_id);
        }
        let selected_index = visible_rows.iter().position(|row| row.selected);

        Self {
            tree_items,
            visible_rows,
            rows_by_id: Arc::new(rows_by_id),
            selected_index,
        }
    }

    pub fn tree_items(&self) -> Vec<TreeItem> {
        self.tree_items.clone()
    }

    pub fn rows(&self) -> &[ProjectTreeRenderRow] {
        &self.visible_rows
    }

    pub fn selected_index(&self) -> Option<usize> {
        self.selected_index
    }

    pub fn row_for_path(&self, path: &Path) -> Option<&ProjectTreeRenderRow> {
        self.rows_by_id
            .values()
            .find(|row| row.relative_path.as_deref() == Some(path))
    }

    fn row_for_id(&self, id: &str) -> Option<&ProjectTreeRenderRow> {
        self.rows_by_id.get(id)
    }
}

#[derive(Clone)]
struct RenderNode {
    row: ProjectTreeRenderRow,
    children: Vec<RenderNode>,
}

impl RenderNode {
    fn tree_item(&self) -> TreeItem {
        TreeItem::new(self.row.id.clone(), self.row.label.clone())
            .expanded(self.row.expanded)
            .disabled(self.row.synthetic)
            .children(self.children.iter().map(Self::tree_item))
    }

    fn collect_rows(
        &self,
        visible_rows: &mut Vec<ProjectTreeRenderRow>,
        rows_by_id: &mut HashMap<String, ProjectTreeRenderRow>,
    ) {
        rows_by_id.insert(self.row.id.clone(), self.row.clone());
        visible_rows.push(self.row.clone());
        if self.row.expanded {
            for child in &self.children {
                child.collect_rows(visible_rows, rows_by_id);
            }
        } else {
            for child in &self.children {
                child.collect_hidden_rows(rows_by_id);
            }
        }
    }

    fn collect_hidden_rows(&self, rows_by_id: &mut HashMap<String, ProjectTreeRenderRow>) {
        rows_by_id.insert(self.row.id.clone(), self.row.clone());
        for child in &self.children {
            child.collect_hidden_rows(rows_by_id);
        }
    }
}

fn build_nodes(
    source_rows: &[ProjectTreeVisibleRow],
    index: &mut usize,
    depth: usize,
    git_status: Option<&ProjectGitStatus>,
    text: &ProjectTreeRenderText,
) -> Vec<RenderNode> {
    let mut nodes = Vec::new();
    while let Some(source) = source_rows.get(*index) {
        if source.depth < depth {
            break;
        }
        if source.depth > depth {
            *index += 1;
            continue;
        }

        *index += 1;
        let mut children = build_nodes(source_rows, index, depth + 1, git_status, text);
        let row = render_row(source, git_status);
        if source.kind.is_traversable() && children.is_empty() {
            children.push(synthetic_child(&row, text));
        }
        nodes.push(RenderNode { row, children });
    }
    nodes
}

fn render_row(
    source: &ProjectTreeVisibleRow,
    git_status: Option<&ProjectGitStatus>,
) -> ProjectTreeRenderRow {
    ProjectTreeRenderRow {
        id: stable_path_id(&source.relative_path),
        relative_path: Some(source.relative_path.clone()),
        label: source.name.to_string_lossy().into_owned(),
        kind: Some(source.kind),
        depth: source.depth,
        expanded: source.expanded,
        selected: source.selected,
        load_state: source.load_state.clone(),
        git_status: git_status.and_then(|status| status.file_status(&source.relative_path)),
        synthetic: false,
    }
}

fn synthetic_child(parent: &ProjectTreeRenderRow, text: &ProjectTreeRenderText) -> RenderNode {
    let label = match &parent.load_state {
        ProjectTreeLoadState::Loading => text.loading.clone(),
        ProjectTreeLoadState::Loaded => text.empty_directory.clone(),
        ProjectTreeLoadState::Error(error) => format!("{error} · {}", text.retry),
        _ => String::new(),
    };
    RenderNode {
        row: ProjectTreeRenderRow {
            id: format!("synthetic:{}", parent.id),
            relative_path: None,
            label,
            kind: None,
            depth: parent.depth + 1,
            expanded: false,
            selected: false,
            load_state: parent.load_state.clone(),
            git_status: None,
            synthetic: true,
        },
        children: Vec::new(),
    }
}

fn stable_path_id(path: &Path) -> String {
    let mut id = String::from("path:");

    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt as _;
        for byte in path.as_os_str().as_bytes() {
            let _ = write!(id, "{byte:02x}");
        }
    }

    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt as _;
        for unit in path.as_os_str().encode_wide() {
            let _ = write!(id, "{unit:04x}");
        }
    }

    #[cfg(not(any(unix, windows)))]
    {
        for byte in path.to_string_lossy().as_bytes() {
            let _ = write!(id, "{byte:02x}");
        }
    }

    id
}

const EDIT_ROW_ID: &str = "project-tree-edit-row";

#[derive(Clone, Debug, PartialEq, Eq)]
enum ProjectTreeCreatePlacement {
    Start,
    FirstChild(PathBuf),
    After(PathBuf),
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum ProjectTreeEditTarget {
    Create {
        parent: PathBuf,
        placement: ProjectTreeCreatePlacement,
        force_directory: bool,
    },
    Rename {
        path: PathBuf,
    },
}

pub struct ProjectTreeView {
    tree: Entity<TreeState>,
    snapshot: ProjectTreeRenderSnapshot,
    icon_theme: IconTheme,
    interaction_text: ProjectTreeInteractionText,
    show_hidden: bool,
    edit_target: Option<ProjectTreeEditTarget>,
    edit_input: Option<Entity<InputState>>,
    edit_subscription: Option<Subscription>,
    edit_input_needs_focus: bool,
}

impl ProjectTreeView {
    pub fn new(snapshot: ProjectTreeRenderSnapshot, cx: &mut Context<Self>) -> Self {
        Self::new_with_icon_theme(snapshot, IconTheme::default(), cx)
    }

    pub fn new_with_icon_theme(
        snapshot: ProjectTreeRenderSnapshot,
        icon_theme: IconTheme,
        cx: &mut Context<Self>,
    ) -> Self {
        let items = snapshot.tree_items();
        let selected_index = snapshot.selected_index();
        let tree = cx.new(|cx| TreeState::new(cx).items(items));
        if selected_index.is_some() {
            tree.update(cx, |state, cx| {
                state.set_selected_index(selected_index, cx);
            });
        }
        Self {
            tree,
            snapshot,
            icon_theme,
            interaction_text: ProjectTreeInteractionText::default(),
            show_hidden: false,
            edit_target: None,
            edit_input: None,
            edit_subscription: None,
            edit_input_needs_focus: false,
        }
    }

    pub fn sync(&mut self, snapshot: ProjectTreeRenderSnapshot, cx: &mut Context<Self>) {
        self.sync_with_icon_theme(snapshot, self.icon_theme.clone(), cx);
    }

    pub fn sync_with_icon_theme(
        &mut self,
        snapshot: ProjectTreeRenderSnapshot,
        icon_theme: IconTheme,
        cx: &mut Context<Self>,
    ) {
        self.snapshot = snapshot;
        self.icon_theme = icon_theme;
        self.rebuild_tree_state(cx);
        cx.notify();
    }

    pub fn set_interaction_text(
        &mut self,
        interaction_text: ProjectTreeInteractionText,
        cx: &mut Context<Self>,
    ) {
        self.interaction_text = interaction_text;
        cx.notify();
    }

    pub fn set_show_hidden(&mut self, show_hidden: bool, cx: &mut Context<Self>) {
        if self.show_hidden == show_hidden {
            return;
        }
        self.show_hidden = show_hidden;
        cx.notify();
    }

    fn toggle_show_hidden(&mut self, cx: &mut Context<Self>) {
        cx.emit(ProjectTreeViewEvent::SetShowHidden(!self.show_hidden));
    }

    pub fn tree_state(&self) -> &Entity<TreeState> {
        &self.tree
    }

    pub fn snapshot(&self) -> &ProjectTreeRenderSnapshot {
        &self.snapshot
    }

    pub fn is_editing(&self) -> bool {
        self.edit_target.is_some()
    }

    pub fn activate_path(&mut self, path: &Path, cx: &mut Context<Self>) -> bool {
        let Some(row) = self.snapshot.row_for_path(path).cloned() else {
            return false;
        };
        self.activate_row(row, cx)
    }

    pub fn request_refresh(&mut self, cx: &mut Context<Self>) {
        cx.emit(ProjectTreeViewEvent::Refresh);
    }

    fn request_project_layout_scaffold(&mut self, cx: &mut Context<Self>) {
        cx.emit(ProjectTreeViewEvent::CreateProjectLayout);
    }

    pub fn begin_create_selected(
        &mut self,
        force_directory: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let row = self.selected_row();
        self.begin_create(row, force_directory, window, cx);
    }

    fn activate_id(&mut self, id: &str, cx: &mut Context<Self>) -> bool {
        let Some(row) = self.snapshot.row_for_id(id).cloned() else {
            return false;
        };
        self.activate_row(row, cx)
    }

    fn activate_row(&mut self, row: ProjectTreeRenderRow, cx: &mut Context<Self>) -> bool {
        let Some(path) = row.relative_path else {
            return false;
        };
        match row.kind {
            Some(ProjectTreeEntryKind::Directory) => {
                cx.emit(ProjectTreeViewEvent::ToggleDirectory {
                    path,
                    expanded: !row.expanded,
                });
                true
            }
            Some(ProjectTreeEntryKind::File | ProjectTreeEntryKind::SymlinkFile) => {
                cx.emit(ProjectTreeViewEvent::OpenFile(path));
                true
            }
            Some(ProjectTreeEntryKind::SymlinkDirectory) | None => false,
        }
    }

    fn selected_row(&self) -> Option<ProjectTreeRenderRow> {
        self.snapshot
            .rows()
            .iter()
            .find(|row| row.selected && !row.synthetic)
            .cloned()
    }

    fn select_context_path(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        cx.emit(ProjectTreeViewEvent::SelectPath(path));
    }

    fn begin_create(
        &mut self,
        row: Option<ProjectTreeRenderRow>,
        force_directory: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (parent, placement) = match row {
            Some(ProjectTreeRenderRow {
                relative_path: Some(path),
                kind: Some(ProjectTreeEntryKind::Directory),
                expanded,
                ..
            }) => {
                if !expanded {
                    cx.emit(ProjectTreeViewEvent::ToggleDirectory {
                        path: path.clone(),
                        expanded: true,
                    });
                }
                (path.clone(), ProjectTreeCreatePlacement::FirstChild(path))
            }
            Some(ProjectTreeRenderRow {
                relative_path: Some(path),
                ..
            }) => {
                let parent = path.parent().unwrap_or(Path::new("")).to_path_buf();
                (parent, ProjectTreeCreatePlacement::After(path))
            }
            _ => (PathBuf::new(), ProjectTreeCreatePlacement::Start),
        };
        self.begin_edit(
            ProjectTreeEditTarget::Create {
                parent,
                placement,
                force_directory,
            },
            String::new(),
            window,
            cx,
        );
    }

    fn begin_rename(
        &mut self,
        row: ProjectTreeRenderRow,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(path) = row.relative_path else {
            return;
        };
        self.begin_edit(
            ProjectTreeEditTarget::Rename { path },
            row.label,
            window,
            cx,
        );
    }

    fn begin_edit(
        &mut self,
        target: ProjectTreeEditTarget,
        value: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.cancel_edit(cx);
        let placeholder = self.interaction_text.entry_placeholder.clone();
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .submit_on_enter(true)
                .default_value(value)
                .placeholder(placeholder)
        });
        let subscription =
            cx.subscribe_in(
                &input,
                window,
                |this, input, event, _window, cx| match event {
                    InputEvent::PressEnter { .. } | InputEvent::Blur => this.submit_edit(input, cx),
                    InputEvent::Change | InputEvent::Focus => {}
                },
            );
        self.edit_target = Some(target);
        self.edit_input = Some(input);
        self.edit_subscription = Some(subscription);
        self.edit_input_needs_focus = true;
        self.rebuild_tree_state(cx);
        cx.notify();
    }

    fn submit_edit(&mut self, input: &Entity<InputState>, cx: &mut Context<Self>) {
        let Some(target) = self.edit_target.clone() else {
            return;
        };
        let value = input
            .read(cx)
            .value()
            .trim_end_matches(['\r', '\n'])
            .to_string();
        if value.is_empty() {
            self.cancel_edit(cx);
            return;
        }
        match target {
            ProjectTreeEditTarget::Create {
                parent,
                force_directory,
                ..
            } => {
                let input = if force_directory && !value.ends_with('/') {
                    format!("{value}/")
                } else {
                    value
                };
                cx.emit(ProjectTreeViewEvent::CreateEntry { parent, input });
            }
            ProjectTreeEditTarget::Rename { path } => {
                if path.file_name().is_none_or(|name| name != value.as_str()) {
                    cx.emit(ProjectTreeViewEvent::RenameEntry {
                        path,
                        new_name: value,
                    });
                }
            }
        }
        self.cancel_edit(cx);
    }

    fn cancel_edit(&mut self, cx: &mut Context<Self>) {
        if self.edit_target.take().is_none() {
            return;
        }
        self.edit_input = None;
        self.edit_subscription = None;
        self.edit_input_needs_focus = false;
        self.rebuild_tree_state(cx);
        cx.notify();
    }

    fn rebuild_tree_state(&mut self, cx: &mut Context<Self>) {
        let mut items = self.snapshot.tree_items();
        if let Some(ProjectTreeEditTarget::Create {
            parent, placement, ..
        }) = &self.edit_target
        {
            insert_edit_item(&mut items, parent, placement);
        }
        let selected_id = self
            .selected_row()
            .and_then(|row| row.relative_path)
            .map(|path| stable_path_id(&path));
        let selected_item = selected_id
            .as_deref()
            .and_then(|id| find_tree_item(&items, id))
            .cloned();
        self.tree.update(cx, |state, tree_cx| {
            state.set_items(items, tree_cx);
            state.set_selected_item(selected_item.as_ref(), tree_cx);
        });
    }

    fn emit_selected_path_event(
        &mut self,
        event: impl FnOnce(PathBuf) -> ProjectTreeViewEvent,
        cx: &mut Context<Self>,
    ) {
        let Some(path) = self.selected_row().and_then(|row| row.relative_path) else {
            cx.propagate();
            return;
        };
        cx.emit(event(path));
    }

    fn on_new_file(&mut self, _: &ProjectTreeNewFile, window: &mut Window, cx: &mut Context<Self>) {
        self.begin_create_selected(false, window, cx);
    }

    fn on_new_directory(
        &mut self,
        _: &ProjectTreeNewDirectory,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.begin_create_selected(true, window, cx);
    }

    fn on_rename(&mut self, _: &ProjectTreeRename, window: &mut Window, cx: &mut Context<Self>) {
        let Some(row) = self.selected_row() else {
            cx.propagate();
            return;
        };
        self.begin_rename(row, window, cx);
    }

    fn on_delete(&mut self, _: &ProjectTreeDelete, _: &mut Window, cx: &mut Context<Self>) {
        self.emit_selected_path_event(ProjectTreeViewEvent::RequestDelete, cx);
    }

    fn on_copy(&mut self, _: &ProjectTreeCopy, _: &mut Window, cx: &mut Context<Self>) {
        self.emit_selected_path_event(ProjectTreeViewEvent::CopyEntry, cx);
    }

    fn on_cut(&mut self, _: &ProjectTreeCut, _: &mut Window, cx: &mut Context<Self>) {
        self.emit_selected_path_event(ProjectTreeViewEvent::CutEntry, cx);
    }

    fn on_paste(&mut self, _: &ProjectTreePaste, _: &mut Window, cx: &mut Context<Self>) {
        let destination_directory = self
            .selected_row()
            .as_ref()
            .map(operation_destination_directory)
            .unwrap_or_default();
        cx.emit(ProjectTreeViewEvent::PasteEntry {
            destination_directory,
        });
    }

    fn on_cancel_edit(&mut self, _: &Escape, _: &mut Window, cx: &mut Context<Self>) {
        if self.edit_target.is_some() {
            self.cancel_edit(cx);
        } else {
            cx.propagate();
        }
    }
}

impl EventEmitter<ProjectTreeViewEvent> for ProjectTreeView {}

impl Render for ProjectTreeView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.edit_input_needs_focus {
            self.edit_input_needs_focus = false;
            if let Some(input) = self.edit_input.clone() {
                cx.defer_in(window, move |_, window, cx| {
                    input.update(cx, |input, input_cx| input.focus(window, input_cx));
                });
            }
        }
        let rows = self.snapshot.rows_by_id.clone();
        let menu_rows = rows.clone();
        let view = cx.weak_entity();
        let menu_view = view.clone();
        let icon_theme = self.icon_theme.clone();
        let edit_target = self.edit_target.clone();
        let edit_input = self.edit_input.clone();
        let text = self.interaction_text.clone();
        let show_hidden = self.show_hidden;
        let tree = tree(&self.tree, move |ix, entry, selected, _window, cx| {
            let id = entry.item().id.as_str().to_string();
            if id == EDIT_ROW_ID {
                return render_edit_row(
                    ix,
                    entry.depth(),
                    selected,
                    None,
                    edit_target.as_ref(),
                    edit_input.as_ref(),
                    &icon_theme,
                    cx,
                );
            }
            let row = rows.get(&id).cloned();
            if row.as_ref().is_some_and(|row| {
                matches!(
                    &edit_target,
                    Some(ProjectTreeEditTarget::Rename { path })
                        if row.relative_path.as_deref() == Some(path)
                )
            }) {
                return render_edit_row(
                    ix,
                    entry.depth(),
                    selected,
                    row,
                    edit_target.as_ref(),
                    edit_input.as_ref(),
                    &icon_theme,
                    cx,
                );
            }
            render_component_row(
                ix,
                entry.depth(),
                selected,
                row,
                &icon_theme,
                view.clone(),
                cx,
            )
        })
        .context_menu(move |_ix, entry, menu, _window, cx| {
            let id = entry.item().id.as_str();
            let Some(row) = menu_rows.get(id).cloned() else {
                return menu;
            };
            let Some(path) = row.relative_path.clone() else {
                return menu;
            };
            let destination_directory = operation_destination_directory(&row);
            let _ = menu_view.update(cx, |view, view_cx| {
                view.select_context_path(path.clone(), view_cx);
            });

            let new_file_view = menu_view.clone();
            let new_file_row = row.clone();
            let new_directory_view = menu_view.clone();
            let new_directory_row = row.clone();
            let project_layout_view = menu_view.clone();
            let hidden_view = menu_view.clone();
            let rename_view = menu_view.clone();
            let rename_row = row.clone();
            let delete_view = menu_view.clone();
            let delete_path = path.clone();
            let copy_view = menu_view.clone();
            let copy_path = path.clone();
            let cut_view = menu_view.clone();
            let cut_path = path.clone();
            let paste_view = menu_view.clone();

            menu.item(
                PopupMenuItem::new(text.new_file.clone()).on_click(move |_, window, cx| {
                    let row = new_file_row.clone();
                    let _ = new_file_view.update(cx, |view, view_cx| {
                        view.begin_create(Some(row), false, window, view_cx);
                    });
                }),
            )
            .item(
                PopupMenuItem::new(text.new_directory.clone()).on_click(move |_, window, cx| {
                    let row = new_directory_row.clone();
                    let _ = new_directory_view.update(cx, |view, view_cx| {
                        view.begin_create(Some(row), true, window, view_cx);
                    });
                }),
            )
            .item(
                PopupMenuItem::new(text.create_project_layout.clone()).on_click(move |_, _, cx| {
                    let _ = project_layout_view.update(cx, |view, view_cx| {
                        view.request_project_layout_scaffold(view_cx);
                    });
                }),
            )
            .item(PopupMenuItem::separator())
            .item(
                PopupMenuItem::new(if show_hidden {
                    text.hide_hidden.clone()
                } else {
                    text.show_hidden.clone()
                })
                .checked(show_hidden)
                .on_click(move |_, _, cx| {
                    let _ = hidden_view.update(cx, |view, view_cx| {
                        view.toggle_show_hidden(view_cx);
                    });
                }),
            )
            .item(PopupMenuItem::separator())
            .item(
                PopupMenuItem::new(text.cut.clone()).on_click(move |_, _, cx| {
                    let _ = cut_view.update(cx, |_, view_cx| {
                        view_cx.emit(ProjectTreeViewEvent::CutEntry(cut_path.clone()));
                    });
                }),
            )
            .item(
                PopupMenuItem::new(text.copy.clone()).on_click(move |_, _, cx| {
                    let _ = copy_view.update(cx, |_, view_cx| {
                        view_cx.emit(ProjectTreeViewEvent::CopyEntry(copy_path.clone()));
                    });
                }),
            )
            .item(
                PopupMenuItem::new(text.paste.clone()).on_click(move |_, _, cx| {
                    let destination_directory = destination_directory.clone();
                    let _ = paste_view.update(cx, |_, view_cx| {
                        view_cx.emit(ProjectTreeViewEvent::PasteEntry {
                            destination_directory,
                        });
                    });
                }),
            )
            .item(PopupMenuItem::separator())
            .item(
                PopupMenuItem::new(text.rename.clone()).on_click(move |_, window, cx| {
                    let row = rename_row.clone();
                    let _ = rename_view.update(cx, |view, view_cx| {
                        view.begin_rename(row, window, view_cx);
                    });
                }),
            )
            .item(
                PopupMenuItem::new(text.delete.clone()).on_click(move |_, _, cx| {
                    let _ = delete_view.update(cx, |_, view_cx| {
                        view_cx.emit(ProjectTreeViewEvent::RequestDelete(delete_path.clone()));
                    });
                }),
            )
        });

        div()
            .size_full()
            .on_action(cx.listener(Self::on_new_file))
            .on_action(cx.listener(Self::on_new_directory))
            .on_action(cx.listener(Self::on_rename))
            .on_action(cx.listener(Self::on_delete))
            .on_action(cx.listener(Self::on_copy))
            .on_action(cx.listener(Self::on_cut))
            .on_action(cx.listener(Self::on_paste))
            .on_action(cx.listener(Self::on_cancel_edit))
            .child(tree)
    }
}

fn operation_destination_directory(row: &ProjectTreeRenderRow) -> PathBuf {
    if row.kind == Some(ProjectTreeEntryKind::Directory) {
        row.relative_path.clone().unwrap_or_default()
    } else {
        row.relative_path
            .as_deref()
            .and_then(Path::parent)
            .unwrap_or(Path::new(""))
            .to_path_buf()
    }
}

fn insert_edit_item(
    items: &mut Vec<TreeItem>,
    parent: &Path,
    placement: &ProjectTreeCreatePlacement,
) {
    let edit_item = TreeItem::new(EDIT_ROW_ID, "").disabled(true);
    let inserted = match placement {
        ProjectTreeCreatePlacement::Start => {
            items.insert(0, edit_item.clone());
            true
        }
        ProjectTreeCreatePlacement::FirstChild(path) => {
            insert_edit_item_under(items, &stable_path_id(path), edit_item.clone())
        }
        ProjectTreeCreatePlacement::After(path) => {
            insert_edit_item_after(items, &stable_path_id(path), edit_item.clone())
        }
    };
    if inserted {
        return;
    }
    if !parent.as_os_str().is_empty()
        && insert_edit_item_under(items, &stable_path_id(parent), edit_item.clone())
    {
        return;
    }
    items.insert(0, edit_item);
}

fn insert_edit_item_under(items: &mut [TreeItem], parent_id: &str, edit_item: TreeItem) -> bool {
    for item in items {
        if item.id.as_str() == parent_id {
            item.children.insert(0, edit_item);
            *item = item.clone().expanded(true);
            return true;
        }
        if insert_edit_item_under(&mut item.children, parent_id, edit_item.clone()) {
            return true;
        }
    }
    false
}

fn insert_edit_item_after(items: &mut Vec<TreeItem>, anchor_id: &str, edit_item: TreeItem) -> bool {
    if let Some(index) = items.iter().position(|item| item.id.as_str() == anchor_id) {
        items.insert(index + 1, edit_item);
        return true;
    }
    for item in items {
        if insert_edit_item_after(&mut item.children, anchor_id, edit_item.clone()) {
            return true;
        }
    }
    false
}

fn find_tree_item<'a>(items: &'a [TreeItem], id: &str) -> Option<&'a TreeItem> {
    for item in items {
        if item.id.as_str() == id {
            return Some(item);
        }
        if let Some(found) = find_tree_item(&item.children, id) {
            return Some(found);
        }
    }
    None
}

fn render_edit_row(
    ix: usize,
    depth: usize,
    selected: bool,
    row: Option<ProjectTreeRenderRow>,
    target: Option<&ProjectTreeEditTarget>,
    input: Option<&Entity<InputState>>,
    icon_theme: &IconTheme,
    cx: &mut App,
) -> ListItem {
    let Some(input) = input else {
        return ListItem::new(("project-tree-missing-edit-input", ix)).disabled(true);
    };
    let force_directory = matches!(
        target,
        Some(ProjectTreeEditTarget::Create {
            force_directory: true,
            ..
        })
    );
    let path = row
        .as_ref()
        .and_then(|row| row.relative_path.as_deref())
        .unwrap_or(Path::new(""));
    let kind = row.as_ref().and_then(|row| row.kind);
    let icon = if force_directory
        || matches!(
            kind,
            Some(ProjectTreeEntryKind::Directory | ProjectTreeEntryKind::SymlinkDirectory)
        ) {
        icon_theme.resolve_directory(path, false)
    } else {
        icon_theme.resolve_file(path)
    };

    ListItem::new(("project-tree-edit-row", ix))
        .selected(selected)
        .pl(px(8.0 + depth as f32 * 14.0))
        .child(
            div()
                .flex()
                .items_center()
                .gap_1()
                .w_full()
                .child(div().w(px(12.0)))
                .child(icon_for_visual(icon, cx.theme().muted_foreground))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .border_1()
                        .border_color(cx.theme().primary)
                        .child(Input::new(input).appearance(false).small()),
                ),
        )
}

fn render_component_row(
    ix: usize,
    depth: usize,
    selected: bool,
    row: Option<ProjectTreeRenderRow>,
    icon_theme: &IconTheme,
    view: gpui::WeakEntity<ProjectTreeView>,
    cx: &mut App,
) -> ListItem {
    let Some(row) = row else {
        return ListItem::new(("project-tree-missing-row", ix)).disabled(true);
    };
    if row.synthetic {
        return ListItem::new(("project-tree-synthetic-row", ix))
            .disabled(true)
            .pl(px(8.0 + depth as f32 * 14.0))
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .truncate()
                    .child(row.label),
            );
    }

    let path = row.relative_path.as_deref().unwrap_or(Path::new(""));
    let icon = match row.kind {
        Some(ProjectTreeEntryKind::Directory | ProjectTreeEntryKind::SymlinkDirectory) => {
            icon_theme.resolve_directory(path, row.expanded)
        }
        _ => icon_theme.resolve_file(path),
    };
    let status_color = match row.git_status {
        Some(GitFileStatus::Added | GitFileStatus::Untracked) => Some(cx.theme().success),
        Some(GitFileStatus::Modified) => Some(cx.theme().warning),
        Some(GitFileStatus::Deleted) => Some(cx.theme().danger),
        Some(GitFileStatus::Ignored) | None => None,
    };
    let label_color = if row.git_status == Some(GitFileStatus::Ignored) {
        cx.theme().muted_foreground
    } else {
        cx.theme().foreground
    };
    let hint = match &row.load_state {
        ProjectTreeLoadState::Loading => Some("…".to_string()),
        ProjectTreeLoadState::Error(error) => Some(error.clone()),
        _ => None,
    };
    let id = row.id.clone();
    let expanded = row.expanded;
    let is_directory = row.kind == Some(ProjectTreeEntryKind::Directory);

    ListItem::new(("project-tree-row", ix))
        .selected(selected)
        .pl(px(8.0 + depth as f32 * 14.0))
        .child(
            div()
                .flex()
                .items_center()
                .gap_1()
                .w_full()
                .child(div().w(px(12.0)).children(is_directory.then(|| {
                    icon_for_visual(
                        icon_theme.resolve_chevron(expanded),
                        cx.theme().muted_foreground,
                    )
                })))
                .child(icon_for_visual(icon, cx.theme().muted_foreground))
                .child(
                    div()
                        .flex_1()
                        .truncate()
                        .text_sm()
                        .text_color(label_color)
                        .child(row.label),
                )
                .children(
                    status_color
                        .map(|color| div().flex_none().size(px(6.0)).rounded_full().bg(color)),
                )
                .children(hint.map(|hint| {
                    div()
                        .max_w(px(120.0))
                        .truncate()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(hint)
                })),
        )
        .on_click(move |_, _, cx| {
            let _ = view.update(cx, |view, cx| {
                view.activate_id(&id, cx);
            });
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::Focusable as _;
    use std::{cell::RefCell, rc::Rc};

    fn item(path: &str, children: Vec<TreeItem>) -> TreeItem {
        TreeItem::new(stable_path_id(Path::new(path)), path)
            .expanded(true)
            .children(children)
    }

    #[test]
    fn create_row_is_first_child_of_clicked_directory() {
        let mut items = vec![item(
            "src",
            vec![
                item("src/main.rs", Vec::new()),
                item("src/lib.rs", Vec::new()),
            ],
        )];

        insert_edit_item(
            &mut items,
            Path::new("src"),
            &ProjectTreeCreatePlacement::FirstChild(PathBuf::from("src")),
        );

        assert_eq!(items[0].children[0].id.as_str(), EDIT_ROW_ID);
        assert_eq!(
            items[0].children[1].id.as_str(),
            stable_path_id(Path::new("src/main.rs"))
        );
    }

    #[test]
    fn create_row_is_after_clicked_file() {
        let mut items = vec![item(
            "src",
            vec![
                item("src/main.rs", Vec::new()),
                item("src/lib.rs", Vec::new()),
            ],
        )];

        insert_edit_item(
            &mut items,
            Path::new("src"),
            &ProjectTreeCreatePlacement::After(PathBuf::from("src/main.rs")),
        );

        assert_eq!(
            items[0].children[0].id.as_str(),
            stable_path_id(Path::new("src/main.rs"))
        );
        assert_eq!(items[0].children[1].id.as_str(), EDIT_ROW_ID);
        assert_eq!(
            items[0].children[2].id.as_str(),
            stable_path_id(Path::new("src/lib.rs"))
        );
    }

    #[test]
    fn missing_anchor_falls_back_to_parent_instead_of_tree_top() {
        let mut items = vec![
            item("src", vec![item("src/main.rs", Vec::new())]),
            item("README.md", Vec::new()),
        ];

        insert_edit_item(
            &mut items,
            Path::new("src"),
            &ProjectTreeCreatePlacement::After(PathBuf::from("src/missing.rs")),
        );

        assert_ne!(items[0].id.as_str(), EDIT_ROW_ID);
        assert_eq!(items[0].children[0].id.as_str(), EDIT_ROW_ID);
    }
    #[gpui::test]
    fn create_and_rename_inputs_submit_with_enter(cx: &mut gpui::TestAppContext) {
        cx.update(gpui_component::init);
        cx.update(|cx| cx.bind_keys(crate::ui::interaction::actions::app_startup_keybindings()));
        let mut model = ProjectFileTree::new("/project");
        let request = model.request_expand(Path::new("")).unwrap();
        model.apply_snapshot(
            request.generation,
            crate::ui::project_tree::DirectorySnapshot {
                relative_directory: PathBuf::new(),
                entries: vec![crate::ui::project_tree::ProjectTreeEntry {
                    name: "README.md".into(),
                    relative_path: PathBuf::from("README.md"),
                    kind: ProjectTreeEntryKind::File,
                }],
            },
        );
        model.select(Some(PathBuf::from("README.md")));
        let snapshot = ProjectTreeRenderSnapshot::from_tree(&model, None);
        let view_slot = Rc::new(RefCell::new(None));
        let view_slot_for_window = view_slot.clone();
        let (_root, cx) = cx.add_window_view(move |window, cx| {
            let view = cx.new(|cx| ProjectTreeView::new(snapshot, cx));
            *view_slot_for_window.borrow_mut() = Some(view.clone());
            gpui_component::Root::new(view, window, cx)
        });
        let view = view_slot.borrow_mut().take().unwrap();
        let events = Rc::new(RefCell::new(Vec::new()));
        let subscription = cx.update(|_, cx| {
            view.update(cx, |_, view_cx| {
                let events = events.clone();
                view_cx.subscribe(&view, move |_, _, event, _| {
                    events.borrow_mut().push(event.clone());
                })
            })
        });
        cx.run_until_parked();

        cx.update(|window, cx| {
            view.update(cx, |view, view_cx| {
                view.begin_create_selected(false, window, view_cx);
            });
        });
        cx.run_until_parked();

        let input = cx.update(|window, cx| {
            let view = view.read(cx);
            assert!(view.is_editing());
            let input = view
                .edit_input
                .as_ref()
                .expect("edit input must remain mounted")
                .clone();
            assert!(input.read(cx).focus_handle(cx).is_focused(window));
            input
        });
        input.update_in(cx, |input, window, input_cx| {
            input.set_value("created.txt", window, input_cx);
        });
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();

        cx.update(|_, cx| assert!(!view.read(cx).is_editing()));
        assert_eq!(
            events.borrow().as_slice(),
            [ProjectTreeViewEvent::CreateEntry {
                parent: PathBuf::new(),
                input: "created.txt".to_string(),
            }]
        );

        view.update_in(cx, |view, window, view_cx| {
            let row = view.selected_row().unwrap();
            view.begin_rename(row, window, view_cx);
        });
        cx.run_until_parked();
        let input = cx.read(|cx| view.read(cx).edit_input.as_ref().unwrap().clone());
        input.update_in(cx, |input, window, input_cx| {
            input.set_value("RENAMED.md", window, input_cx);
        });
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();

        cx.update(|_, cx| assert!(!view.read(cx).is_editing()));
        assert_eq!(
            events.borrow().as_slice(),
            [
                ProjectTreeViewEvent::CreateEntry {
                    parent: PathBuf::new(),
                    input: "created.txt".to_string(),
                },
                ProjectTreeViewEvent::RenameEntry {
                    path: PathBuf::from("README.md"),
                    new_name: "RENAMED.md".to_string(),
                },
            ]
        );
        drop(subscription);
    }

    #[gpui::test]
    fn tree_settings_and_project_layout_requests_emit_events(cx: &mut gpui::TestAppContext) {
        cx.update(gpui_component::init);
        let snapshot =
            ProjectTreeRenderSnapshot::from_tree(&ProjectFileTree::new("/project"), None);
        let view_slot = Rc::new(RefCell::new(None));
        let view_slot_for_window = view_slot.clone();
        let (_root, cx) = cx.add_window_view(move |window, cx| {
            let view = cx.new(|cx| ProjectTreeView::new(snapshot, cx));
            *view_slot_for_window.borrow_mut() = Some(view.clone());
            gpui_component::Root::new(view, window, cx)
        });
        let view = view_slot.borrow_mut().take().unwrap();
        let events = Rc::new(RefCell::new(Vec::new()));
        let subscription = cx.update(|_, cx| {
            view.update(cx, |_, view_cx| {
                let events = events.clone();
                view_cx.subscribe(&view, move |_, _, event, _| {
                    events.borrow_mut().push(event.clone());
                })
            })
        });

        view.update_in(cx, |view, _, view_cx| {
            view.toggle_show_hidden(view_cx);
            view.set_show_hidden(true, view_cx);
            view.toggle_show_hidden(view_cx);
            view.request_project_layout_scaffold(view_cx);
        });

        assert_eq!(
            events.borrow().as_slice(),
            [
                ProjectTreeViewEvent::SetShowHidden(true),
                ProjectTreeViewEvent::SetShowHidden(false),
                ProjectTreeViewEvent::CreateProjectLayout,
            ]
        );
        drop(subscription);
    }
}
