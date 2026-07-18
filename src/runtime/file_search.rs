use std::{
    cmp::Ordering,
    path::{Path, PathBuf},
    sync::Arc,
};

use nucleo_matcher::{
    Config, Matcher, Utf32Str,
    pattern::{CaseMatching, Normalization, Pattern},
};

use crate::{model::ids::ProjectId, runtime::project::ProjectServices};

pub const MAX_FILE_SEARCH_RESULTS: usize = 100;

#[derive(Clone)]
pub struct FileSearchProject {
    pub project_id: ProjectId,
    pub project_title: String,
    pub services: ProjectServices,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileSearchCandidate {
    pub project_id: ProjectId,
    pub project_title: String,
    pub relative_path: PathBuf,
    pub display_path: String,
    pub file_name: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileSearchMatch {
    pub candidate_index: usize,
    pub score: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FileSearchCollection {
    pub candidates: Vec<FileSearchCandidate>,
    pub errors: Vec<String>,
}

pub fn collect_file_search_candidates(
    projects: Vec<FileSearchProject>,
    show_hidden: bool,
) -> FileSearchCollection {
    let mut collection = FileSearchCollection::default();

    for project in projects {
        match project.services.searchable_files(show_hidden) {
            Ok(paths) => {
                collection
                    .candidates
                    .extend(paths.into_iter().map(|relative_path| {
                        let display_path = path_for_display(&relative_path);
                        let file_name = relative_path
                            .file_name()
                            .map(|name| name.to_string_lossy().into_owned())
                            .unwrap_or_else(|| display_path.clone());
                        FileSearchCandidate {
                            project_id: project.project_id.clone(),
                            project_title: project.project_title.clone(),
                            relative_path,
                            display_path,
                            file_name,
                        }
                    }));
            }
            Err(error) => collection
                .errors
                .push(format!("{}: {error}", project.project_title)),
        }
    }

    collection.candidates.sort_by(|left, right| {
        left.project_title
            .to_lowercase()
            .cmp(&right.project_title.to_lowercase())
            .then_with(|| {
                left.display_path
                    .to_lowercase()
                    .cmp(&right.display_path.to_lowercase())
            })
            .then_with(|| left.display_path.cmp(&right.display_path))
    });
    collection
}

pub fn match_file_search_candidates(
    candidates: &Arc<Vec<FileSearchCandidate>>,
    query: &str,
) -> Vec<FileSearchMatch> {
    let query = query.trim();
    if query.is_empty() {
        return candidates
            .iter()
            .take(MAX_FILE_SEARCH_RESULTS)
            .enumerate()
            .map(|(candidate_index, _)| FileSearchMatch {
                candidate_index,
                score: 0,
            })
            .collect();
    }

    let pattern = Pattern::parse(query, CaseMatching::Smart, Normalization::Smart);
    let mut matcher = Matcher::new(Config::DEFAULT.match_paths());
    let mut char_buf = Vec::new();
    let mut matches = candidates
        .iter()
        .enumerate()
        .filter_map(|(candidate_index, candidate)| {
            let score = pattern.score(
                Utf32Str::new(&candidate.display_path, &mut char_buf),
                &mut matcher,
            )?;
            Some(FileSearchMatch {
                candidate_index,
                score,
            })
        })
        .collect::<Vec<_>>();

    matches.sort_by(|left, right| {
        right.score.cmp(&left.score).then_with(|| {
            compare_candidates(candidates, left.candidate_index, right.candidate_index)
        })
    });
    matches.truncate(MAX_FILE_SEARCH_RESULTS);
    matches
}

fn compare_candidates(
    candidates: &[FileSearchCandidate],
    left_index: usize,
    right_index: usize,
) -> Ordering {
    let left = &candidates[left_index];
    let right = &candidates[right_index];
    left.display_path
        .len()
        .cmp(&right.display_path.len())
        .then_with(|| left.display_path.cmp(&right.display_path))
        .then_with(|| left.project_title.cmp(&right.project_title))
}

fn path_for_display(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(path: &str) -> FileSearchCandidate {
        FileSearchCandidate {
            project_id: ProjectId::new("project"),
            project_title: "project".to_string(),
            relative_path: PathBuf::from(path),
            display_path: path.to_string(),
            file_name: Path::new(path)
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into_owned(),
        }
    }

    #[test]
    fn fuzzy_matching_accepts_non_contiguous_path_queries() {
        let candidates = Arc::new(vec![
            candidate("src/main.rs"),
            candidate("src/runtime/project.rs"),
        ]);

        let matches = match_file_search_candidates(&candidates, "smn");

        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].candidate_index, 0);
    }

    #[test]
    fn fuzzy_matching_limits_results() {
        let candidates = Arc::new(
            (0..150)
                .map(|index| candidate(&format!("src/file-{index}.rs")))
                .collect(),
        );

        let matches = match_file_search_candidates(&candidates, "file");

        assert_eq!(matches.len(), MAX_FILE_SEARCH_RESULTS);
    }
}
