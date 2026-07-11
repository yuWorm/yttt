use std::{fs, path::Path, process::Command};

use tempfile::tempdir;
use yttt::runtime::git_status::{
    GitBranch, GitBranchKind, read_project_git_branches, read_project_git_diff,
    read_project_git_status, switch_project_git_branch,
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
