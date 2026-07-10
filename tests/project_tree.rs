use std::{cell::RefCell, ffi::OsString, fs, path::Path, rc::Rc};

use gpui::AppContext as _;
use tempfile::tempdir;
use yttt::{
    runtime::git_status::{GitFileStatus, parse_git_status_porcelain},
    ui::project_tree::{
        DirectorySnapshot, ProjectFileTree, ProjectTreeEntry, ProjectTreeEntryKind,
        ProjectTreeFsError, ProjectTreeLoadState, ProjectTreeRenderSnapshot, ProjectTreeView,
        ProjectTreeViewEvent, scan_project_directory,
    },
};

#[test]
fn scan_sorts_directories_before_files_case_insensitively() {
    let root = tempdir().unwrap();
    for directory in ["zDir", "adir", "empty"] {
        fs::create_dir(root.path().join(directory)).unwrap();
    }
    fs::write(root.path().join("B.rs"), "b").unwrap();
    fs::write(root.path().join("a.rs"), "a").unwrap();
    fs::write(root.path().join(".hidden"), "hidden").unwrap();

    let snapshot = scan_project_directory(root.path(), Path::new(""), false).unwrap();

    let names = snapshot
        .entries
        .iter()
        .map(|entry| entry.name.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    assert_eq!(names, ["adir", "empty", "zDir", "a.rs", "B.rs"]);
    assert_eq!(snapshot.relative_directory, Path::new(""));
    assert_eq!(snapshot.entries[0].kind, ProjectTreeEntryKind::Directory);
    assert_eq!(snapshot.entries[3].kind, ProjectTreeEntryKind::File);
}

#[test]
fn scan_filters_hidden_entries_and_keeps_empty_directories() {
    let root = tempdir().unwrap();
    fs::create_dir(root.path().join("empty")).unwrap();
    fs::write(root.path().join(".env"), "TOKEN=test").unwrap();

    let visible = scan_project_directory(root.path(), Path::new(""), false).unwrap();
    let all = scan_project_directory(root.path(), Path::new(""), true).unwrap();
    let empty = scan_project_directory(root.path(), Path::new("empty"), false).unwrap();

    assert_eq!(visible.entries.len(), 1);
    assert_eq!(visible.entries[0].relative_path, Path::new("empty"));
    assert_eq!(all.entries.len(), 2);
    assert!(empty.entries.is_empty());
}

#[cfg(unix)]
#[test]
fn scan_classifies_symlinks_without_traversing_symlink_directories() {
    use std::os::unix::fs::symlink;

    let root = tempdir().unwrap();
    fs::create_dir(root.path().join("real-dir")).unwrap();
    fs::write(root.path().join("real.txt"), "text").unwrap();
    symlink("real-dir", root.path().join("dir-link")).unwrap();
    symlink("real.txt", root.path().join("file-link")).unwrap();

    let snapshot = scan_project_directory(root.path(), Path::new(""), false).unwrap();
    let dir_link = snapshot
        .entries
        .iter()
        .find(|entry| entry.name == "dir-link")
        .unwrap();
    let file_link = snapshot
        .entries
        .iter()
        .find(|entry| entry.name == "file-link")
        .unwrap();

    assert_eq!(dir_link.kind, ProjectTreeEntryKind::SymlinkDirectory);
    assert_eq!(file_link.kind, ProjectTreeEntryKind::SymlinkFile);
    assert!(matches!(
        scan_project_directory(root.path(), Path::new("dir-link"), false),
        Err(ProjectTreeFsError::SymlinkDirectory { .. })
    ));
}

#[test]
fn scan_rejects_directories_outside_the_project() {
    let root = tempdir().unwrap();

    assert!(matches!(
        scan_project_directory(root.path(), Path::new("../outside"), false),
        Err(ProjectTreeFsError::PathOutsideProject { .. })
    ));
}

#[test]
fn expanding_an_unloaded_directory_requests_one_scan() {
    let mut tree = ProjectFileTree::new("/project");

    let request = tree.request_expand(Path::new("src")).unwrap();

    assert_eq!(request.relative_directory, Path::new("src"));
    assert_eq!(request.generation, tree.generation());
    assert!(tree.request_expand(Path::new("src")).is_none());
}

#[test]
fn applying_snapshots_flattens_expanded_rows_and_collapse_keeps_cache() {
    let mut tree = ProjectFileTree::new("/project");
    let root_request = tree.request_expand(Path::new("")).unwrap();
    assert!(tree.apply_snapshot(
        root_request.generation,
        snapshot(
            "",
            [
                entry("src", ProjectTreeEntryKind::Directory),
                entry("README.md", ProjectTreeEntryKind::File)
            ]
        ),
    ));
    let src_request = tree.request_expand(Path::new("src")).unwrap();
    assert!(tree.apply_snapshot(
        src_request.generation,
        snapshot("src", [entry("src/main.rs", ProjectTreeEntryKind::File)]),
    ));

    let rows = tree.visible_rows();
    assert_eq!(
        rows.iter()
            .map(|row| (row.relative_path.to_string_lossy().into_owned(), row.depth))
            .collect::<Vec<_>>(),
        [
            ("src".to_string(), 0),
            ("src/main.rs".to_string(), 1),
            ("README.md".to_string(), 0)
        ]
    );

    tree.collapse(Path::new("src"));

    assert_eq!(tree.visible_rows().len(), 2);
    assert!(tree.has_snapshot(Path::new("src")));
    assert!(tree.request_expand(Path::new("src")).is_none());
    assert_eq!(tree.visible_rows().len(), 3);
}

#[test]
fn refresh_preserves_expansion_and_ignores_stale_results() {
    let mut tree = ProjectFileTree::new("/project");
    let root_request = tree.request_expand(Path::new("")).unwrap();
    tree.apply_snapshot(
        root_request.generation,
        snapshot("", [entry("src", ProjectTreeEntryKind::Directory)]),
    );
    let old_src_request = tree.request_expand(Path::new("src")).unwrap();

    let refresh_request = tree.refresh();

    assert!(tree.is_expanded(Path::new("src")));
    assert!(refresh_request.generation > old_src_request.generation);
    assert!(!tree.apply_snapshot(
        old_src_request.generation,
        snapshot("src", [entry("src/stale.rs", ProjectTreeEntryKind::File)]),
    ));
    assert!(tree.apply_snapshot(
        refresh_request.generation,
        snapshot("", [entry("src", ProjectTreeEntryKind::Directory)]),
    ));
    let refreshed_src_request = tree.request_expand(Path::new("src")).unwrap();
    assert_eq!(refreshed_src_request.generation, refresh_request.generation);
}

#[test]
fn directory_errors_are_local_and_retryable() {
    let mut tree = ProjectFileTree::new("/project");
    let root_request = tree.request_expand(Path::new("")).unwrap();
    tree.apply_snapshot(
        root_request.generation,
        snapshot(
            "",
            [
                entry("bad", ProjectTreeEntryKind::Directory),
                entry("good", ProjectTreeEntryKind::Directory),
            ],
        ),
    );
    let bad_request = tree.request_expand(Path::new("bad")).unwrap();
    assert!(tree.apply_error(
        bad_request.generation,
        Path::new("bad"),
        "permission denied",
    ));

    let rows = tree.visible_rows();
    let bad = rows
        .iter()
        .find(|row| row.relative_path == Path::new("bad"))
        .unwrap();
    let good = rows
        .iter()
        .find(|row| row.relative_path == Path::new("good"))
        .unwrap();
    assert_eq!(
        bad.load_state,
        ProjectTreeLoadState::Error("permission denied".to_string())
    );
    assert_eq!(good.load_state, ProjectTreeLoadState::Unloaded);

    assert!(tree.request_expand(Path::new("bad")).is_some());
    assert_eq!(
        tree.visible_rows()
            .into_iter()
            .find(|row| row.relative_path == Path::new("bad"))
            .unwrap()
            .load_state,
        ProjectTreeLoadState::Loading
    );
}

#[test]
fn visible_rows_mark_the_selected_path() {
    let mut tree = ProjectFileTree::new("/project");
    let request = tree.request_expand(Path::new("")).unwrap();
    tree.apply_snapshot(
        request.generation,
        snapshot("", [entry("README.md", ProjectTreeEntryKind::File)]),
    );
    tree.select(Some(Path::new("README.md").to_path_buf()));

    let rows = tree.visible_rows();

    assert_eq!(rows.len(), 1);
    assert!(rows[0].selected);
}

#[test]
fn component_items_keep_empty_directories_expandable() {
    let mut tree = ProjectFileTree::new("/project");
    let root_request = tree.request_expand(Path::new("")).unwrap();
    tree.apply_snapshot(
        root_request.generation,
        snapshot("", [entry("empty", ProjectTreeEntryKind::Directory)]),
    );

    let unloaded = ProjectTreeRenderSnapshot::from_tree(&tree, None);
    let unloaded_items = unloaded.tree_items();
    let unloaded_item = &unloaded_items[0];
    assert!(unloaded_item.is_folder());
    assert_eq!(unloaded_item.children.len(), 1);
    assert!(unloaded_item.children[0].is_disabled());

    let empty_request = tree.request_expand(Path::new("empty")).unwrap();
    tree.apply_snapshot(empty_request.generation, snapshot("empty", []));
    let loaded_empty = ProjectTreeRenderSnapshot::from_tree(&tree, None);
    let loaded_items = loaded_empty.tree_items();
    let loaded_item = &loaded_items[0];
    assert!(loaded_item.is_folder());
    assert!(loaded_item.is_expanded());
    assert_eq!(loaded_item.children.len(), 1);
    assert!(loaded_item.children[0].is_disabled());
}

#[test]
fn component_synthetic_rows_use_supplied_project_tree_text() {
    let mut tree = ProjectFileTree::new("/project");
    let root_request = tree.request_expand(Path::new("")).unwrap();
    tree.apply_snapshot(
        root_request.generation,
        snapshot("", [entry("src", ProjectTreeEntryKind::Directory)]),
    );
    tree.request_expand(Path::new("src")).unwrap();

    let render = ProjectTreeRenderSnapshot::from_tree_with_text(
        &tree,
        None,
        &yttt::ui::project_tree::ProjectTreeRenderText {
            loading: "正在加载…".to_string(),
            empty_directory: "空目录".to_string(),
            retry: "重试".to_string(),
        },
    );

    assert!(
        render
            .rows()
            .iter()
            .any(|row| row.synthetic && row.label == "正在加载…")
    );
}

#[test]
fn component_path_ids_and_selection_survive_refresh() {
    let mut tree = ProjectFileTree::new("/project");
    let root_request = tree.request_expand(Path::new("")).unwrap();
    tree.apply_snapshot(
        root_request.generation,
        snapshot(
            "",
            [
                entry("src", ProjectTreeEntryKind::Directory),
                entry("README.md", ProjectTreeEntryKind::File),
            ],
        ),
    );
    tree.select(Some(Path::new("README.md").to_path_buf()));
    let before = ProjectTreeRenderSnapshot::from_tree(&tree, None);
    let before_id = before
        .row_for_path(Path::new("README.md"))
        .unwrap()
        .id
        .clone();
    assert_eq!(before.selected_index(), Some(1));

    let refresh = tree.refresh();
    tree.apply_snapshot(
        refresh.generation,
        snapshot(
            "",
            [
                entry("src", ProjectTreeEntryKind::Directory),
                entry("README.md", ProjectTreeEntryKind::File),
            ],
        ),
    );
    let after = ProjectTreeRenderSnapshot::from_tree(&tree, None);

    assert_eq!(
        after.row_for_path(Path::new("README.md")).unwrap().id,
        before_id
    );
    assert_eq!(after.selected_index(), Some(1));
}

#[test]
fn component_rows_include_git_status_tones() {
    let mut tree = ProjectFileTree::new("/project");
    let request = tree.request_expand(Path::new("")).unwrap();
    tree.apply_snapshot(
        request.generation,
        snapshot("", [entry("src/main.rs", ProjectTreeEntryKind::File)]),
    );
    let git = parse_git_status_porcelain("## main\n M src/main.rs\n");

    let render = ProjectTreeRenderSnapshot::from_tree(&tree, Some(&git));

    assert_eq!(
        render
            .row_for_path(Path::new("src/main.rs"))
            .unwrap()
            .git_status,
        Some(GitFileStatus::Modified)
    );
}

#[gpui::test]
fn project_tree_view_emits_file_and_directory_events(cx: &mut gpui::TestAppContext) {
    let mut tree = ProjectFileTree::new("/project");
    let request = tree.request_expand(Path::new("")).unwrap();
    tree.apply_snapshot(
        request.generation,
        snapshot(
            "",
            [
                entry("src", ProjectTreeEntryKind::Directory),
                entry("README.md", ProjectTreeEntryKind::File),
            ],
        ),
    );
    tree.select(Some(Path::new("README.md").to_path_buf()));
    let render = ProjectTreeRenderSnapshot::from_tree(&tree, None);
    let view = cx.new(|cx| ProjectTreeView::new(render, cx));
    cx.read(|app| {
        let tree_state = view.read(app).tree_state().clone();
        assert_eq!(tree_state.read(app).selected_index(), Some(1));
    });
    tree.select(Some(Path::new("src").to_path_buf()));
    let synced = ProjectTreeRenderSnapshot::from_tree(&tree, None);
    view.update(cx, |view, entity_cx| view.sync(synced, entity_cx));
    cx.read(|app| {
        let tree_state = view.read(app).tree_state().clone();
        assert_eq!(tree_state.read(app).selected_index(), Some(0));
    });
    let events = Rc::new(RefCell::new(Vec::new()));
    let subscription = view.update(cx, |_, entity_cx| {
        let events = events.clone();
        entity_cx.subscribe(&view, move |_, _, event, _| {
            events.borrow_mut().push(event.clone());
        })
    });

    view.update(cx, |view, entity_cx| {
        assert!(view.activate_path(Path::new("README.md"), entity_cx));
        assert!(view.activate_path(Path::new("src"), entity_cx));
    });

    assert_eq!(
        events.borrow().as_slice(),
        [
            ProjectTreeViewEvent::OpenFile(Path::new("README.md").to_path_buf()),
            ProjectTreeViewEvent::ToggleDirectory {
                path: Path::new("src").to_path_buf(),
                expanded: true,
            },
        ]
    );
    drop(subscription);
}

fn entry(path: &str, kind: ProjectTreeEntryKind) -> ProjectTreeEntry {
    let path = Path::new(path);
    ProjectTreeEntry {
        name: OsString::from(path.file_name().unwrap()),
        relative_path: path.to_path_buf(),
        kind,
    }
}

fn snapshot<const N: usize>(
    relative_directory: &str,
    entries: [ProjectTreeEntry; N],
) -> DirectorySnapshot {
    DirectorySnapshot {
        relative_directory: Path::new(relative_directory).to_path_buf(),
        entries: entries.into(),
    }
}
