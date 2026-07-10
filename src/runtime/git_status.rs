use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GitFileStatus {
    Added,
    Modified,
    Deleted,
    Untracked,
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
        self.file_statuses.get(relative_path).copied()
    }

    pub fn file_statuses(&self) -> &BTreeMap<PathBuf, GitFileStatus> {
        &self.file_statuses
    }
}

pub fn read_project_git_status(project_path: &Path) -> Option<ProjectGitStatus> {
    let output = Command::new("git")
        .args(["status", "--porcelain=v1", "-b"])
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
