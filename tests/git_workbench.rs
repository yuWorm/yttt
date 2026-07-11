use std::{fs, path::Path, process::Command};

use tempfile::tempdir;
use yttt::runtime::git_status::{
    GitBranch, GitBranchKind, GitDiffLineKind, GitDiffMode, read_project_git_branches,
    read_project_git_diff, read_project_git_diff_result, read_project_git_status,
    switch_project_git_branch,
};

fn git(project_path: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(project_path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn initialized_repository() -> (tempfile::TempDir, std::path::PathBuf) {
    let temp = tempdir().unwrap();
    let project_path = temp.path().join("project");
    fs::create_dir(&project_path).unwrap();
    git(&project_path, &["init"]);
    git(&project_path, &["config", "user.email", "test@example.com"]);
    git(&project_path, &["config", "user.name", "YTTT Test"]);
    fs::write(project_path.join("tracked.txt"), "base\n").unwrap();
    git(&project_path, &["add", "tracked.txt"]);
    git(&project_path, &["commit", "-m", "initial"]);
    git(&project_path, &["branch", "-M", "main"]);
    (temp, project_path)
}

#[test]
fn branch_list_marks_current_branch_and_switches_local_branch() {
    let (_temp, project_path) = initialized_repository();
    git(&project_path, &["branch", "feature"]);

    let branches = read_project_git_branches(&project_path).unwrap();
    assert!(branches.iter().any(|branch| {
        branch.name == "main" && branch.kind == GitBranchKind::Local && branch.current
    }));
    let feature = branches
        .iter()
        .find(|branch| branch.name == "feature")
        .cloned()
        .unwrap();

    switch_project_git_branch(&project_path, &feature).unwrap();

    assert_eq!(
        read_project_git_status(&project_path)
            .unwrap()
            .branch
            .as_deref(),
        Some("feature")
    );
}

#[test]
fn branch_switch_rejects_option_like_ref_names() {
    let (_temp, project_path) = initialized_repository();
    let branch = GitBranch {
        name: "--detach".to_string(),
        kind: GitBranchKind::Local,
        current: false,
    };

    let error = switch_project_git_branch(&project_path, &branch).unwrap_err();

    assert!(error.contains("Invalid Git branch name"));
    assert_eq!(
        read_project_git_status(&project_path)
            .unwrap()
            .branch
            .as_deref(),
        Some("main")
    );
}

#[test]
fn working_tree_diff_includes_tracked_and_nested_untracked_files() {
    let (_temp, project_path) = initialized_repository();
    fs::write(project_path.join("tracked.txt"), "changed\n").unwrap();
    fs::create_dir(project_path.join("newdir")).unwrap();
    fs::write(project_path.join("newdir/new.txt"), "new\n").unwrap();

    let diff = read_project_git_diff(&project_path).unwrap();

    assert!(diff.contains("-base"));
    assert!(diff.contains("+changed"));
    assert!(diff.contains("newdir/new.txt"));
    assert!(diff.contains("+new"));
}

#[test]
fn remote_only_branch_is_listed_and_checked_out_as_tracking_branch() {
    let (temp, project_path) = initialized_repository();
    let remote_path = temp.path().join("origin.git");
    git(
        &project_path,
        &["init", "--bare", remote_path.to_str().unwrap()],
    );
    git(
        &project_path,
        &["remote", "add", "origin", remote_path.to_str().unwrap()],
    );
    git(&project_path, &["push", "-u", "origin", "main"]);
    git(&project_path, &["switch", "-c", "remote-only"]);
    fs::write(project_path.join("remote.txt"), "remote\n").unwrap();
    git(&project_path, &["add", "remote.txt"]);
    git(&project_path, &["commit", "-m", "remote branch"]);
    git(&project_path, &["push", "-u", "origin", "remote-only"]);
    git(&project_path, &["switch", "main"]);
    git(&project_path, &["branch", "-D", "remote-only"]);
    git(&project_path, &["fetch", "origin"]);

    let branches = read_project_git_branches(&project_path).unwrap();
    assert!(
        !branches.iter().any(|branch| branch.name == "origin/main"),
        "a remote branch with an existing local branch must be deduplicated"
    );
    let remote = branches
        .iter()
        .find(|branch| branch.name == "origin/remote-only")
        .cloned()
        .unwrap();
    assert_eq!(remote.kind, GitBranchKind::Remote);

    switch_project_git_branch(&project_path, &remote).unwrap();

    assert_eq!(
        read_project_git_status(&project_path)
            .unwrap()
            .branch
            .as_deref(),
        Some("remote-only")
    );
}

#[test]
fn structured_diff_groups_files_hunks_and_line_numbers() {
    let (_temp, project_path) = initialized_repository();
    fs::write(project_path.join("tracked.txt"), "changed\n").unwrap();
    fs::create_dir(project_path.join("newdir")).unwrap();
    fs::write(project_path.join("newdir/new.txt"), "new\nsecond\n").unwrap();

    let diff = read_project_git_diff_result(&project_path, GitDiffMode::Unstaged, false).unwrap();

    assert_eq!(diff.files.len(), 2);
    let tracked = diff
        .files
        .iter()
        .find(|file| file.path() == "tracked.txt")
        .unwrap();
    assert_eq!((tracked.added, tracked.removed), (1, 1));
    assert!(
        tracked
            .hunks
            .iter()
            .flat_map(|hunk| &hunk.lines)
            .any(|line| line.kind == GitDiffLineKind::Removed
                && line.old_line == Some(1)
                && line.new_line.is_none()
                && line.content == "base")
    );
    assert!(
        tracked
            .hunks
            .iter()
            .flat_map(|hunk| &hunk.lines)
            .any(|line| line.kind == GitDiffLineKind::Added
                && line.old_line.is_none()
                && line.new_line == Some(1)
                && line.content == "changed")
    );
    let untracked = diff
        .files
        .iter()
        .find(|file| file.path() == "newdir/new.txt")
        .unwrap();
    assert_eq!((untracked.added, untracked.removed), (2, 0));
    assert_eq!((diff.total_added(), diff.total_removed()), (3, 1));
}

#[test]
fn staged_and_unstaged_modes_read_different_snapshots() {
    let (_temp, project_path) = initialized_repository();
    fs::write(project_path.join("tracked.txt"), "staged\n").unwrap();
    git(&project_path, &["add", "tracked.txt"]);
    fs::write(project_path.join("tracked.txt"), "unstaged\n").unwrap();

    let staged = read_project_git_diff_result(&project_path, GitDiffMode::Staged, false).unwrap();
    let unstaged =
        read_project_git_diff_result(&project_path, GitDiffMode::Unstaged, false).unwrap();

    let staged_lines = staged.files[0]
        .hunks
        .iter()
        .flat_map(|hunk| &hunk.lines)
        .map(|line| line.content.as_str())
        .collect::<Vec<_>>();
    let unstaged_lines = unstaged.files[0]
        .hunks
        .iter()
        .flat_map(|hunk| &hunk.lines)
        .map(|line| line.content.as_str())
        .collect::<Vec<_>>();
    assert!(staged_lines.contains(&"staged"));
    assert!(!staged_lines.contains(&"unstaged"));
    assert!(unstaged_lines.contains(&"unstaged"));
    assert!(!unstaged_lines.contains(&"base"));
}

#[test]
fn ignore_whitespace_suppresses_whitespace_only_changes() {
    let (_temp, project_path) = initialized_repository();
    fs::write(project_path.join("tracked.txt"), "base   \n").unwrap();

    let normal = read_project_git_diff_result(&project_path, GitDiffMode::Unstaged, false).unwrap();
    let ignoring_whitespace =
        read_project_git_diff_result(&project_path, GitDiffMode::Unstaged, true).unwrap();

    assert_eq!(normal.files.len(), 1);
    assert!(ignoring_whitespace.files.is_empty());
}
