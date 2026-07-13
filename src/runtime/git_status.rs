use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GitFileStatus {
    Added,
    Modified,
    Deleted,
    Untracked,
    Ignored,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GitBranchKind {
    Local,
    Remote,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitBranch {
    pub name: String,
    pub kind: GitBranchKind,
    pub current: bool,
}

impl GitBranch {
    pub fn id(&self) -> String {
        let prefix = match self.kind {
            GitBranchKind::Local => "local",
            GitBranchKind::Remote => "remote",
        };
        format!("{prefix}:{}", self.name)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GitDiffMode {
    #[default]
    Unstaged,
    Staged,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GitDiffLineKind {
    Context,
    Added,
    Removed,
    Hunk,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitDiffLine {
    pub kind: GitDiffLineKind,
    pub content: String,
    pub old_line: Option<usize>,
    pub new_line: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitDiffHunk {
    pub header: String,
    pub lines: Vec<GitDiffLine>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GitFileChangeKind {
    Added,
    Modified,
    Deleted,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitFileDiff {
    pub old_path: Option<String>,
    pub new_path: Option<String>,
    pub hunks: Vec<GitDiffHunk>,
    pub binary: bool,
    pub added: usize,
    pub removed: usize,
}

impl GitFileDiff {
    pub fn path(&self) -> &str {
        self.new_path
            .as_deref()
            .or(self.old_path.as_deref())
            .unwrap_or("unknown")
    }

    pub fn change_kind(&self) -> GitFileChangeKind {
        if self.old_path.is_none() {
            GitFileChangeKind::Added
        } else if self.new_path.is_none() {
            GitFileChangeKind::Deleted
        } else {
            GitFileChangeKind::Modified
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GitDiffResult {
    pub files: Vec<GitFileDiff>,
}

impl GitDiffResult {
    pub fn total_added(&self) -> usize {
        self.files.iter().map(|file| file.added).sum()
    }

    pub fn total_removed(&self) -> usize {
        self.files.iter().map(|file| file.removed).sum()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GitStatusSummary {
    pub added: usize,
    pub modified: usize,
    pub deleted: usize,
    pub untracked: usize,
}

impl GitStatusSummary {
    pub fn is_clean(&self) -> bool {
        self.added == 0 && self.modified == 0 && self.deleted == 0 && self.untracked == 0
    }

    pub fn compact_counters(&self) -> Option<String> {
        if self.is_clean() {
            return None;
        }

        let mut parts = Vec::new();
        if self.added > 0 {
            parts.push(format!("+{}", self.added));
        }
        if self.modified > 0 {
            parts.push(format!("~{}", self.modified));
        }
        if self.deleted > 0 {
            parts.push(format!("-{}", self.deleted));
        }

        Some(parts.join(" "))
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProjectGitStatus {
    pub branch: Option<String>,
    pub summary: GitStatusSummary,
    file_statuses: BTreeMap<PathBuf, GitFileStatus>,
}

impl ProjectGitStatus {
    pub fn file_status(&self, relative_path: &Path) -> Option<GitFileStatus> {
        self.file_statuses.get(relative_path).copied().or_else(|| {
            relative_path.ancestors().skip(1).find_map(|ancestor| {
                (self.file_statuses.get(ancestor) == Some(&GitFileStatus::Ignored))
                    .then_some(GitFileStatus::Ignored)
            })
        })
    }

    pub fn file_statuses(&self) -> &BTreeMap<PathBuf, GitFileStatus> {
        &self.file_statuses
    }
}

pub fn read_project_git_branches(project_path: &Path) -> Result<Vec<GitBranch>, String> {
    let mut branches = read_git_branch_group(project_path, "refs/heads", GitBranchKind::Local)?;
    let local_names = branches
        .iter()
        .map(|branch| branch.name.clone())
        .collect::<BTreeSet<_>>();
    let remote = read_git_branch_group(project_path, "refs/remotes", GitBranchKind::Remote)?;
    branches.extend(remote.into_iter().filter(|branch| {
        !branch.name.ends_with("/HEAD")
            && branch
                .name
                .split_once('/')
                .map(|(_, local_name)| !local_names.contains(local_name))
                .unwrap_or(true)
    }));
    branches.sort_by(|left, right| {
        right
            .current
            .cmp(&left.current)
            .then_with(|| branch_kind_order(left.kind).cmp(&branch_kind_order(right.kind)))
            .then_with(|| left.name.cmp(&right.name))
    });
    Ok(branches)
}

pub fn switch_project_git_branch(project_path: &Path, branch: &GitBranch) -> Result<(), String> {
    if branch.current {
        return Ok(());
    }
    if !is_safe_git_ref_name(&branch.name) {
        return Err(format!("Invalid Git branch name: {}", branch.name));
    }
    let mut command = Command::new("git");
    command.arg("switch");
    if branch.kind == GitBranchKind::Remote {
        command.arg("--track");
    }
    command
        .arg("--")
        .arg(&branch.name)
        .current_dir(project_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    let output = command
        .output()
        .map_err(|error| format!("Failed to run git switch: {error}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(git_stderr_message(
            &output.stderr,
            "Git could not switch branches",
        ))
    }
}

pub fn read_project_git_diff(project_path: &Path) -> Result<String, String> {
    read_project_git_diff_output(project_path, GitDiffMode::Unstaged, false)
}

pub fn read_project_git_diff_result(
    project_path: &Path,
    mode: GitDiffMode,
    ignore_whitespace: bool,
) -> Result<GitDiffResult, String> {
    read_project_git_diff_output(project_path, mode, ignore_whitespace)
        .map(|output| parse_unified_git_diff(&output))
}

fn read_project_git_diff_output(
    project_path: &Path,
    mode: GitDiffMode,
    ignore_whitespace: bool,
) -> Result<String, String> {
    let mut command = Command::new("git");
    command.arg("diff");
    if mode == GitDiffMode::Staged {
        command.arg("--cached");
    }
    command.args(["--no-ext-diff", "--no-color"]);
    if ignore_whitespace {
        command.arg("-w");
    }
    let output = command
        .arg("--")
        .current_dir(project_path)
        .stdin(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .map_err(|error| format!("Failed to read Git changes: {error}"))?;
    if !output.status.success() {
        return Err(git_stderr_message(
            &output.stderr,
            "Git could not read the working tree diff",
        ));
    }

    let mut diff = String::from_utf8(output.stdout)
        .map_err(|_| "Git diff output was not valid UTF-8".to_string())?;
    if mode == GitDiffMode::Unstaged {
        for path in read_untracked_paths(project_path)? {
            let mut command = Command::new("git");
            command.args(["diff", "--no-index", "--no-ext-diff", "--no-color"]);
            if ignore_whitespace {
                command.arg("-w");
            }
            let output = command
                .arg("--")
                .arg(null_device_path())
                .arg(&path)
                .current_dir(project_path)
                .stdin(Stdio::null())
                .stderr(Stdio::piped())
                .output()
                .map_err(|error| format!("Failed to read untracked file diff: {error}"))?;
            if !output.status.success() && output.status.code() != Some(1) {
                return Err(git_stderr_message(
                    &output.stderr,
                    "Git could not read an untracked file diff",
                ));
            }
            let untracked = String::from_utf8(output.stdout)
                .map_err(|_| "Untracked file diff was not valid UTF-8".to_string())?;
            if !diff.is_empty() && !untracked.is_empty() {
                diff.push('\n');
            }
            diff.push_str(&untracked);
        }
    }
    Ok(diff)
}

pub fn read_project_git_status(project_path: &Path) -> Option<ProjectGitStatus> {
    let output = Command::new("git")
        .args(["status", "--porcelain=v1", "-b", "--ignored=matching"])
        .current_dir(project_path)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8(output.stdout).ok()?;
    Some(parse_git_status_porcelain(&stdout))
}

pub fn parse_git_status_porcelain(output: &str) -> ProjectGitStatus {
    let mut status = ProjectGitStatus::default();

    for line in output.lines() {
        if let Some(branch) = line.strip_prefix("## ") {
            status.branch = parse_branch_name(branch);
            continue;
        }

        if line.len() < 2 {
            continue;
        }

        let state = &line[..2];
        if state == "!!" {
            if let Some(path) = status_path(line) {
                status.file_statuses.insert(path, GitFileStatus::Ignored);
            }
            continue;
        }

        if state == "??" {
            status.summary.added += 1;
            status.summary.untracked += 1;
            if let Some(path) = status_path(line) {
                status.file_statuses.insert(path, GitFileStatus::Untracked);
            }
            continue;
        }

        if let Some(file_status) = count_status_pair(state, &mut status.summary)
            && let Some(path) = status_path(line)
        {
            status.file_statuses.insert(path, file_status);
        }
    }

    status
}

fn parse_branch_name(value: &str) -> Option<String> {
    let branch = value
        .split("...")
        .next()
        .unwrap_or(value)
        .split(' ')
        .next()
        .unwrap_or(value)
        .trim();

    if branch.is_empty() {
        None
    } else {
        Some(branch.to_string())
    }
}

fn count_status_pair(value: &str, summary: &mut GitStatusSummary) -> Option<GitFileStatus> {
    if value.contains('D') {
        summary.deleted += 1;
        Some(GitFileStatus::Deleted)
    } else if value.contains('A') {
        summary.added += 1;
        Some(GitFileStatus::Added)
    } else if value
        .chars()
        .any(|status| matches!(status, 'M' | 'R' | 'C' | 'T' | 'U'))
    {
        summary.modified += 1;
        Some(GitFileStatus::Modified)
    } else {
        None
    }
}

fn status_path(line: &str) -> Option<PathBuf> {
    let path = line.get(3..)?.trim();
    let destination = path
        .rsplit_once(" -> ")
        .map(|(_, destination)| destination)
        .unwrap_or(path);
    (!destination.is_empty()).then(|| PathBuf::from(destination))
}

pub fn parse_unified_git_diff(output: &str) -> GitDiffResult {
    let mut result = GitDiffResult::default();
    let mut file: Option<GitFileDiff> = None;
    let mut hunk: Option<GitDiffHunk> = None;
    let mut old_line = 0usize;
    let mut new_line = 0usize;

    for line in output.lines() {
        if let Some(header) = line.strip_prefix("diff --git ") {
            finish_git_diff_file(&mut result, &mut file, &mut hunk);
            let (old_path, new_path) = parse_diff_git_paths(header);
            file = Some(GitFileDiff {
                old_path,
                new_path,
                hunks: Vec::new(),
                binary: false,
                added: 0,
                removed: 0,
            });
            continue;
        }

        let Some(current_file) = file.as_mut() else {
            continue;
        };
        if let Some(path) = line
            .strip_prefix("rename from ")
            .or_else(|| line.strip_prefix("copy from "))
        {
            current_file.old_path = Some(path.to_string());
            continue;
        }
        if let Some(path) = line
            .strip_prefix("rename to ")
            .or_else(|| line.strip_prefix("copy to "))
        {
            current_file.new_path = Some(path.to_string());
            continue;
        }
        if let Some(path) = line.strip_prefix("--- ") {
            current_file.old_path = git_diff_path(path, "a/");
            continue;
        }
        if let Some(path) = line.strip_prefix("+++ ") {
            current_file.new_path = git_diff_path(path, "b/");
            continue;
        }
        if line.starts_with("Binary files ") && line.ends_with(" differ") {
            current_file.binary = true;
            continue;
        }
        if line.starts_with("@@") {
            if let Some(previous) = hunk.take() {
                current_file.hunks.push(previous);
            }
            let (old_start, new_start) = parse_diff_hunk_starts(line);
            old_line = old_start;
            new_line = new_start;
            hunk = Some(GitDiffHunk {
                header: line.to_string(),
                lines: vec![GitDiffLine {
                    kind: GitDiffLineKind::Hunk,
                    content: line.to_string(),
                    old_line: None,
                    new_line: None,
                }],
            });
            continue;
        }

        let Some(current_hunk) = hunk.as_mut() else {
            continue;
        };
        if let Some(content) = line.strip_prefix('+') {
            current_hunk.lines.push(GitDiffLine {
                kind: GitDiffLineKind::Added,
                content: content.to_string(),
                old_line: None,
                new_line: Some(new_line),
            });
            current_file.added += 1;
            new_line += 1;
        } else if let Some(content) = line.strip_prefix('-') {
            current_hunk.lines.push(GitDiffLine {
                kind: GitDiffLineKind::Removed,
                content: content.to_string(),
                old_line: Some(old_line),
                new_line: None,
            });
            current_file.removed += 1;
            old_line += 1;
        } else if let Some(content) = line.strip_prefix(' ') {
            current_hunk.lines.push(GitDiffLine {
                kind: GitDiffLineKind::Context,
                content: content.to_string(),
                old_line: Some(old_line),
                new_line: Some(new_line),
            });
            old_line += 1;
            new_line += 1;
        } else if line.is_empty() {
            current_hunk.lines.push(GitDiffLine {
                kind: GitDiffLineKind::Context,
                content: String::new(),
                old_line: Some(old_line),
                new_line: Some(new_line),
            });
            old_line += 1;
            new_line += 1;
        }
    }

    finish_git_diff_file(&mut result, &mut file, &mut hunk);
    result
}

fn finish_git_diff_file(
    result: &mut GitDiffResult,
    file: &mut Option<GitFileDiff>,
    hunk: &mut Option<GitDiffHunk>,
) {
    if let Some(mut completed) = file.take() {
        if let Some(last_hunk) = hunk.take() {
            completed.hunks.push(last_hunk);
        }
        result.files.push(completed);
    }
}

fn parse_diff_git_paths(header: &str) -> (Option<String>, Option<String>) {
    let Some(separator) = header.rfind(" b/") else {
        return (None, None);
    };
    let old_path = header[..separator].trim().trim_matches('"');
    let new_path = header[separator + 1..].trim().trim_matches('"');
    (
        old_path.strip_prefix("a/").map(ToString::to_string),
        new_path.strip_prefix("b/").map(ToString::to_string),
    )
}

fn git_diff_path(path: &str, prefix: &str) -> Option<String> {
    let path = path.trim().trim_matches('"');
    if path == "/dev/null" {
        None
    } else {
        Some(path.strip_prefix(prefix).unwrap_or(path).to_string())
    }
}

fn parse_diff_hunk_starts(header: &str) -> (usize, usize) {
    let mut old_start = 1usize;
    let mut new_start = 1usize;
    for part in header.split_whitespace() {
        if let Some(value) = part.strip_prefix('-') {
            old_start = value
                .split(',')
                .next()
                .and_then(|value| value.parse().ok())
                .unwrap_or(1);
        } else if let Some(value) = part.strip_prefix('+') {
            new_start = value
                .split(',')
                .next()
                .and_then(|value| value.parse().ok())
                .unwrap_or(1);
        }
    }
    (old_start, new_start)
}

fn read_git_branch_group(
    project_path: &Path,
    reference: &str,
    kind: GitBranchKind,
) -> Result<Vec<GitBranch>, String> {
    let output = Command::new("git")
        .args([
            "for-each-ref",
            "--format=%(refname:short)\t%(HEAD)",
            reference,
        ])
        .current_dir(project_path)
        .stdin(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .map_err(|error| format!("Failed to list Git branches: {error}"))?;
    if !output.status.success() {
        return Err(git_stderr_message(
            &output.stderr,
            "Git could not list branches",
        ));
    }
    let stdout = String::from_utf8(output.stdout)
        .map_err(|_| "Git branch output was not valid UTF-8".to_string())?;
    Ok(stdout
        .lines()
        .filter_map(|line| {
            let (name, head) = line.split_once('\t').unwrap_or((line, ""));
            let name = name.trim();
            (!name.is_empty()).then(|| GitBranch {
                name: name.to_string(),
                kind,
                current: head.trim() == "*",
            })
        })
        .collect())
}

fn branch_kind_order(kind: GitBranchKind) -> u8 {
    match kind {
        GitBranchKind::Local => 0,
        GitBranchKind::Remote => 1,
    }
}

fn git_stderr_message(stderr: &[u8], fallback: &str) -> String {
    let message = String::from_utf8_lossy(stderr).trim().to_string();
    if message.is_empty() {
        fallback.to_string()
    } else {
        message
    }
}

fn is_safe_git_ref_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('-')
        && !name.ends_with('.')
        && !name.ends_with('/')
        && !name.contains("..")
        && !name.contains("@{")
        && !name.contains("//")
        && !name
            .chars()
            .any(|character| character.is_control() || " ~^:?*[\\]".contains(character))
}

fn read_untracked_paths(project_path: &Path) -> Result<Vec<PathBuf>, String> {
    let output = Command::new("git")
        .args(["ls-files", "--others", "--exclude-standard", "-z", "--"])
        .current_dir(project_path)
        .stdin(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .map_err(|error| format!("Failed to list untracked files: {error}"))?;
    if !output.status.success() {
        return Err(git_stderr_message(
            &output.stderr,
            "Git could not list untracked files",
        ));
    }
    output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
        .map(|path| {
            String::from_utf8(path.to_vec())
                .map(PathBuf::from)
                .map_err(|_| "An untracked file path was not valid UTF-8".to_string())
        })
        .collect()
}

#[cfg(not(windows))]
fn null_device_path() -> &'static str {
    "/dev/null"
}

#[cfg(windows)]
fn null_device_path() -> &'static str {
    "NUL"
}
