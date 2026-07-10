use std::{ffi::OsString, fs, path::Path};

use tempfile::tempdir;
use yttt::ui::project_tree::{
    DirectorySnapshot, ProjectFileTree, ProjectTreeEntry, ProjectTreeEntryKind, ProjectTreeFsError,
    ProjectTreeLoadState, scan_project_directory,
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
