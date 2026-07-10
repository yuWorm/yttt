use std::{
    collections::{BTreeSet, HashMap, HashSet},
    ffi::OsString,
    path::{Component, Path, PathBuf},
};

use super::{DirectorySnapshot, ProjectTreeEntry, ProjectTreeEntryKind};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirectoryLoadRequest {
    pub relative_directory: PathBuf,
    pub generation: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProjectTreeLoadState {
    NotApplicable,
    Unloaded,
    Loading,
    Loaded,
    Error(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectTreeVisibleRow {
    pub name: OsString,
    pub relative_path: PathBuf,
    pub kind: ProjectTreeEntryKind,
    pub depth: usize,
    pub expanded: bool,
    pub selected: bool,
    pub load_state: ProjectTreeLoadState,
}

#[derive(Clone, Debug)]
pub struct ProjectFileTree {
    root: PathBuf,
    expanded: BTreeSet<PathBuf>,
    directories: HashMap<PathBuf, DirectorySnapshot>,
    loading: HashSet<PathBuf>,
    errors: HashMap<PathBuf, String>,
    selected: Option<PathBuf>,
    generation: u64,
}

impl ProjectFileTree {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            expanded: BTreeSet::from([PathBuf::new()]),
            directories: HashMap::new(),
            loading: HashSet::new(),
            errors: HashMap::new(),
            selected: None,
            generation: 0,
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn request_expand(&mut self, path: &Path) -> Option<DirectoryLoadRequest> {
        let path = normalize_tree_path(path)?;
        if self
            .entry_for_path(&path)
            .is_some_and(|entry| !entry.kind.is_traversable())
        {
            return None;
        }

        self.expanded.insert(path.clone());
        if self.directories.contains_key(&path) || !self.loading.insert(path.clone()) {
            return None;
        }
        self.errors.remove(&path);
        Some(DirectoryLoadRequest {
            relative_directory: path,
            generation: self.generation,
        })
    }

    pub fn collapse(&mut self, path: &Path) {
        let Some(path) = normalize_tree_path(path) else {
            return;
        };
        if path.as_os_str().is_empty() {
            return;
        }
        self.expanded.remove(&path);
    }

    pub fn is_expanded(&self, path: &Path) -> bool {
        normalize_tree_path(path).is_some_and(|path| self.expanded.contains(&path))
    }

    pub fn has_snapshot(&self, path: &Path) -> bool {
        normalize_tree_path(path).is_some_and(|path| self.directories.contains_key(&path))
    }

    pub fn directory_load_state(&self, path: &Path) -> ProjectTreeLoadState {
        let Some(path) = normalize_tree_path(path) else {
            return ProjectTreeLoadState::NotApplicable;
        };
        if let Some(error) = self.errors.get(&path) {
            return ProjectTreeLoadState::Error(error.clone());
        }
        if self.loading.contains(&path) {
            return ProjectTreeLoadState::Loading;
        }
        if self.directories.contains_key(&path) {
            return ProjectTreeLoadState::Loaded;
        }
        if path.as_os_str().is_empty()
            || self
                .entry_for_path(&path)
                .is_some_and(|entry| entry.kind.is_traversable())
        {
            ProjectTreeLoadState::Unloaded
        } else {
            ProjectTreeLoadState::NotApplicable
        }
    }

    pub fn apply_snapshot(&mut self, generation: u64, snapshot: DirectorySnapshot) -> bool {
        if generation != self.generation {
            return false;
        }
        let Some(path) = normalize_tree_path(&snapshot.relative_directory) else {
            return false;
        };
        if path != snapshot.relative_directory || !self.loading.remove(&path) {
            return false;
        }
        self.errors.remove(&path);
        self.directories.insert(path, snapshot);
        true
    }

    pub fn apply_error(&mut self, generation: u64, path: &Path, error: impl Into<String>) -> bool {
        if generation != self.generation {
            return false;
        }
        let Some(path) = normalize_tree_path(path) else {
            return false;
        };
        if !self.loading.remove(&path) {
            return false;
        }
        self.errors.insert(path, error.into());
        true
    }

    pub fn refresh(&mut self) -> DirectoryLoadRequest {
        self.generation = self.generation.wrapping_add(1);
        self.directories.clear();
        self.loading.clear();
        self.errors.clear();
        self.expanded.insert(PathBuf::new());
        self.loading.insert(PathBuf::new());
        DirectoryLoadRequest {
            relative_directory: PathBuf::new(),
            generation: self.generation,
        }
    }

    pub fn refresh_expanded(&mut self) -> Vec<DirectoryLoadRequest> {
        self.generation = self.generation.wrapping_add(1);
        self.directories.clear();
        self.loading.clear();
        self.errors.clear();
        self.expanded.insert(PathBuf::new());
        self.expanded
            .iter()
            .cloned()
            .map(|relative_directory| {
                self.loading.insert(relative_directory.clone());
                DirectoryLoadRequest {
                    relative_directory,
                    generation: self.generation,
                }
            })
            .collect()
    }

    pub fn reset_root(&mut self, root: impl Into<PathBuf>) -> DirectoryLoadRequest {
        self.root = root.into();
        self.expanded.clear();
        self.expanded.insert(PathBuf::new());
        self.selected = None;
        self.refresh()
    }

    pub fn select(&mut self, path: Option<PathBuf>) {
        self.selected = path.and_then(|path| normalize_tree_path(&path));
    }

    pub fn selected(&self) -> Option<&Path> {
        self.selected.as_deref()
    }

    pub fn visible_rows(&self) -> Vec<ProjectTreeVisibleRow> {
        let mut rows = Vec::new();
        if let Some(root) = self.directories.get(Path::new("")) {
            self.append_visible_entries(&root.entries, 0, &mut rows);
        }
        rows
    }

    fn append_visible_entries(
        &self,
        entries: &[ProjectTreeEntry],
        depth: usize,
        rows: &mut Vec<ProjectTreeVisibleRow>,
    ) {
        for entry in entries {
            let expanded = entry.kind.is_traversable()
                && self.expanded.contains(entry.relative_path.as_path());
            rows.push(ProjectTreeVisibleRow {
                name: entry.name.clone(),
                relative_path: entry.relative_path.clone(),
                kind: entry.kind,
                depth,
                expanded,
                selected: self.selected.as_deref() == Some(entry.relative_path.as_path()),
                load_state: self.load_state(entry),
            });

            if expanded && let Some(snapshot) = self.directories.get(&entry.relative_path) {
                self.append_visible_entries(&snapshot.entries, depth + 1, rows);
            }
        }
    }

    fn load_state(&self, entry: &ProjectTreeEntry) -> ProjectTreeLoadState {
        if !entry.kind.is_traversable() {
            return ProjectTreeLoadState::NotApplicable;
        }
        if let Some(error) = self.errors.get(&entry.relative_path) {
            return ProjectTreeLoadState::Error(error.clone());
        }
        if self.loading.contains(&entry.relative_path) {
            return ProjectTreeLoadState::Loading;
        }
        if self.directories.contains_key(&entry.relative_path) {
            ProjectTreeLoadState::Loaded
        } else {
            ProjectTreeLoadState::Unloaded
        }
    }

    fn entry_for_path(&self, path: &Path) -> Option<&ProjectTreeEntry> {
        let parent = path.parent().unwrap_or_else(|| Path::new(""));
        self.directories
            .get(parent)?
            .entries
            .iter()
            .find(|entry| entry.relative_path == path)
    }
}

fn normalize_tree_path(path: &Path) -> Option<PathBuf> {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(component) => normalized.push(component),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    Some(normalized)
}
