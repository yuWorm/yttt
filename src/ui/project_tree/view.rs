use std::{
    collections::HashMap,
    fmt::Write as _,
    path::{Path, PathBuf},
    sync::Arc,
};

use gpui::{
    App, AppContext as _, Context, Entity, EventEmitter, IntoElement, ParentElement as _, Render,
    Styled as _, Window, div, px,
};
use gpui_component::{
    ActiveTheme as _, Icon, IconName,
    list::ListItem,
    tree::{TreeItem, TreeState, tree},
};

use crate::runtime::git_status::{GitFileStatus, ProjectGitStatus};

use super::{ProjectFileTree, ProjectTreeEntryKind, ProjectTreeLoadState, ProjectTreeVisibleRow};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProjectTreeViewEvent {
    ToggleDirectory { path: PathBuf, expanded: bool },
    OpenFile(PathBuf),
    Refresh,
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
        let source_rows = tree.visible_rows();
        let mut index = 0;
        let nodes = build_nodes(&source_rows, &mut index, 0, git_status);
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
        let mut children = build_nodes(source_rows, index, depth + 1, git_status);
        let row = render_row(source, git_status);
        if source.kind.is_traversable() && children.is_empty() {
            children.push(synthetic_child(&row));
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

fn synthetic_child(parent: &ProjectTreeRenderRow) -> RenderNode {
    let label = match &parent.load_state {
        ProjectTreeLoadState::Loading => "Loading…".to_string(),
        ProjectTreeLoadState::Error(error) => error.clone(),
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

pub struct ProjectTreeView {
    tree: Entity<TreeState>,
    snapshot: ProjectTreeRenderSnapshot,
}

impl ProjectTreeView {
    pub fn new(snapshot: ProjectTreeRenderSnapshot, cx: &mut Context<Self>) -> Self {
        let items = snapshot.tree_items();
        let selected_index = snapshot.selected_index();
        let tree = cx.new(|cx| TreeState::new(cx).items(items));
        if selected_index.is_some() {
            tree.update(cx, |state, cx| {
                state.set_selected_index(selected_index, cx);
            });
        }
        Self { tree, snapshot }
    }

    pub fn sync(&mut self, snapshot: ProjectTreeRenderSnapshot, cx: &mut Context<Self>) {
        let items = snapshot.tree_items();
        let selected_index = snapshot.selected_index();
        self.tree.update(cx, |state, cx| {
            state.set_items(items, cx);
            state.set_selected_index(selected_index, cx);
        });
        self.snapshot = snapshot;
        cx.notify();
    }

    pub fn tree_state(&self) -> &Entity<TreeState> {
        &self.tree
    }

    pub fn snapshot(&self) -> &ProjectTreeRenderSnapshot {
        &self.snapshot
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
}

impl EventEmitter<ProjectTreeViewEvent> for ProjectTreeView {}

impl Render for ProjectTreeView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let rows = self.snapshot.rows_by_id.clone();
        let view = cx.weak_entity();
        div()
            .size_full()
            .child(tree(&self.tree, move |ix, entry, selected, _window, cx| {
                let id = entry.item().id.as_str().to_string();
                let row = rows.get(&id).cloned();
                render_component_row(ix, entry.depth(), selected, row, view.clone(), cx)
            }))
    }
}

fn render_component_row(
    ix: usize,
    depth: usize,
    selected: bool,
    row: Option<ProjectTreeRenderRow>,
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

    let icon = match row.kind {
        Some(ProjectTreeEntryKind::Directory | ProjectTreeEntryKind::SymlinkDirectory) => {
            if row.expanded {
                IconName::FolderOpen
            } else {
                IconName::FolderClosed
            }
        }
        _ => IconName::File,
    };
    let text_color = match row.git_status {
        Some(GitFileStatus::Added | GitFileStatus::Untracked) => cx.theme().success,
        Some(GitFileStatus::Modified) => cx.theme().warning,
        Some(GitFileStatus::Deleted) => cx.theme().danger,
        None => cx.theme().foreground,
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
                    Icon::new(if expanded {
                        IconName::ChevronDown
                    } else {
                        IconName::ChevronRight
                    })
                    .size_3()
                    .text_color(cx.theme().muted_foreground)
                })))
                .child(Icon::new(icon).size_3().text_color(text_color))
                .child(
                    div()
                        .flex_1()
                        .truncate()
                        .text_sm()
                        .text_color(text_color)
                        .child(row.label),
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
