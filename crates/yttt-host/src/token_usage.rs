use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::{self, BufRead as _, BufReader, Write as _},
    path::{Path, PathBuf},
};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use yttt_agent_core::{AgentTokenUsage, ProjectTokenUsage, ProviderId};

const SNAPSHOT_FILE: &str = "project-token-usage.json";
const JOURNAL_FILE: &str = "project-token-usage.journal";
const SCHEMA_VERSION: u16 = 1;
const MAX_SNAPSHOT_BYTES: usize = 32 * 1024 * 1024;
const MAX_JOURNAL_BYTES: u64 = 64 * 1024 * 1024;
const MAX_JOURNAL_LINE_BYTES: usize = 16 * 1024;
const COMPACT_AFTER_JOURNAL_BYTES: u64 = 4 * 1024 * 1024;
const MAX_ID_BYTES: usize = 1024;

/// Durable, Host-owned project token accounting.
///
/// Every durable mutation is appended and fsynced before the in-memory state is
/// published. A compacted snapshot is only a replay acceleration; the journal
/// remains the acknowledgement path so progress updates never rewrite the
/// growing ledger.
pub(crate) struct TokenUsageLedger {
    root: Option<PathBuf>,
    state: Mutex<TokenUsageState>,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum TokenUsageLedgerError {
    #[error("invalid token usage: {0}")]
    Invalid(String),
    #[error("token usage persistence failed: {0}")]
    Io(#[from] io::Error),
}

#[derive(Clone, Default)]
struct TokenUsageState {
    projects: BTreeMap<String, ProjectLedger>,
    // Only rebuilt at load or after a durable mutation, never on each client poll.
    summaries: BTreeMap<String, ProjectTokenUsage>,
}

#[derive(Clone, Default)]
struct ProjectLedger {
    sources: BTreeMap<ProviderSourceKey, SourceObservation>,
    subjects: BTreeMap<ProviderSubjectKey, SubjectObservation>,
    updated_at: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
struct ProviderSourceKey {
    provider_id: String,
    source_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
struct ProviderSubjectKey {
    provider_id: String,
    subject_id: String,
}

#[derive(Clone, Default, Serialize, Deserialize)]
struct SourceObservation {
    reported: bool,
}

#[derive(Clone, Serialize, Deserialize)]
struct SubjectObservation {
    parent_id: Option<String>,
    tokens: u64,
    includes_children: bool,
    partial: bool,
}

#[derive(Serialize, Deserialize)]
struct PersistedLedger {
    schema_version: u16,
    state: PersistedTokenUsageState,
}

#[derive(Serialize, Deserialize)]
struct PersistedTokenUsageState {
    projects: BTreeMap<String, PersistedProjectLedger>,
}

#[derive(Serialize, Deserialize)]
struct PersistedProjectLedger {
    sources: Vec<(ProviderSourceKey, SourceObservation)>,
    subjects: Vec<(ProviderSubjectKey, SubjectObservation)>,
    updated_at: u64,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum JournalEntry {
    Usage {
        project_id: String,
        provider_id: String,
        source_id: String,
        usage: AgentTokenUsage,
        updated_at: u64,
    },
    Gap {
        project_id: String,
        provider_id: String,
        source_id: String,
        updated_at: u64,
    },
}

fn encode_state(state: &TokenUsageState) -> PersistedTokenUsageState {
    PersistedTokenUsageState {
        projects: state
            .projects
            .iter()
            .map(|(project_id, project)| {
                (
                    project_id.clone(),
                    PersistedProjectLedger {
                        sources: project
                            .sources
                            .iter()
                            .map(|(key, source)| (key.clone(), source.clone()))
                            .collect(),
                        subjects: project
                            .subjects
                            .iter()
                            .map(|(key, subject)| (key.clone(), subject.clone()))
                            .collect(),
                        updated_at: project.updated_at,
                    },
                )
            })
            .collect(),
    }
}

fn decode_state(
    persisted: PersistedTokenUsageState,
) -> Result<TokenUsageState, TokenUsageLedgerError> {
    let mut projects = BTreeMap::new();
    for (project_id, project) in persisted.projects {
        let mut sources = BTreeMap::new();
        for (key, source) in project.sources {
            if sources.insert(key, source).is_some() {
                return Err(invalid("duplicate token usage source in snapshot"));
            }
        }
        let mut subjects = BTreeMap::new();
        for (key, subject) in project.subjects {
            if subjects.insert(key, subject).is_some() {
                return Err(invalid("duplicate token usage subject in snapshot"));
            }
        }
        projects.insert(
            project_id,
            ProjectLedger {
                sources,
                subjects,
                updated_at: project.updated_at,
            },
        );
    }
    Ok(TokenUsageState {
        projects,
        summaries: BTreeMap::new(),
    })
}

impl TokenUsageLedger {
    pub(crate) fn open(root: impl AsRef<Path>) -> Result<Self, TokenUsageLedgerError> {
        let root = root.as_ref().to_path_buf();
        fs::create_dir_all(&root)?;
        let snapshot_path = root.join(SNAPSHOT_FILE);
        let mut state =
            match crate::workspace::read_bounded_file(&snapshot_path, MAX_SNAPSHOT_BYTES) {
                Ok(bytes) => {
                    let persisted: PersistedLedger =
                        serde_json::from_slice(&bytes).map_err(|error| {
                            invalid(format!("invalid token usage snapshot: {error}"))
                        })?;
                    if persisted.schema_version != SCHEMA_VERSION {
                        return Err(invalid("unsupported token usage snapshot schema"));
                    }
                    decode_state(persisted.state)?
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => TokenUsageState::default(),
                Err(error) => return Err(error.into()),
            };
        let journal_path = root.join(JOURNAL_FILE);
        match fs::metadata(&journal_path) {
            Ok(metadata) if metadata.len() > MAX_JOURNAL_BYTES => {
                return Err(invalid("token usage journal exceeds its size limit"));
            }
            Ok(_) => {
                if let Some(committed_bytes) = replay_journal(&journal_path, &mut state)? {
                    truncate_journal_tail(&journal_path, committed_bytes)?;
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        validate_state(&state)?;
        state.summaries = state
            .projects
            .keys()
            .filter_map(|id| summary_for(&state, id).map(|summary| (id.clone(), summary)))
            .collect();
        Ok(Self {
            root: Some(root),
            state: Mutex::new(state),
        })
    }

    #[cfg(test)]
    pub(crate) fn memory() -> Self {
        Self {
            root: None,
            state: Mutex::new(TokenUsageState::default()),
        }
    }

    pub(crate) fn observe(
        &self,
        project_id: &str,
        provider_id: &ProviderId,
        source_id: &str,
        usage: &AgentTokenUsage,
        updated_at: u64,
    ) -> Result<Option<ProjectTokenUsage>, TokenUsageLedgerError> {
        validate_usage(project_id, provider_id.as_str(), source_id, usage)?;
        let mut state = self.state.lock();
        if !usage_would_change(&state, project_id, provider_id.as_str(), source_id, usage)? {
            return Ok(None);
        }
        self.append(&JournalEntry::Usage {
            project_id: project_id.to_string(),
            provider_id: provider_id.to_string(),
            source_id: source_id.to_string(),
            usage: usage.clone(),
            updated_at,
        })?;
        let applied = apply_usage(
            &mut state,
            project_id,
            provider_id.as_str(),
            source_id,
            usage,
            updated_at,
        )?;
        debug_assert!(applied, "prevalidated token usage must change the ledger");
        let summary = summary_for(&state, project_id).expect("changed project has a summary");
        state
            .summaries
            .insert(project_id.to_string(), summary.clone());
        self.maybe_compact(&state);
        Ok(Some(summary))
    }

    pub(crate) fn observe_gap(
        &self,
        project_id: &str,
        provider_id: &ProviderId,
        source_id: &str,
        updated_at: u64,
    ) -> Result<Option<ProjectTokenUsage>, TokenUsageLedgerError> {
        validate_id("project id", project_id)?;
        validate_id("provider id", provider_id.as_str())?;
        validate_id("source id", source_id)?;
        let mut state = self.state.lock();
        if !gap_would_change(&state, project_id, provider_id.as_str(), source_id) {
            return Ok(None);
        }
        self.append(&JournalEntry::Gap {
            project_id: project_id.to_string(),
            provider_id: provider_id.to_string(),
            source_id: source_id.to_string(),
            updated_at,
        })?;
        let applied = apply_gap(
            &mut state,
            project_id,
            provider_id.as_str(),
            source_id,
            updated_at,
        );
        debug_assert!(
            applied,
            "prevalidated token source gap must change the ledger"
        );
        let summary = summary_for(&state, project_id).expect("changed project has a summary");
        state
            .summaries
            .insert(project_id.to_string(), summary.clone());
        self.maybe_compact(&state);
        Ok(Some(summary))
    }

    pub(crate) fn summaries(&self) -> Vec<ProjectTokenUsage> {
        self.state.lock().summaries.values().cloned().collect()
    }

    #[cfg(test)]
    pub(crate) fn summary(&self, project_id: &str) -> Option<ProjectTokenUsage> {
        self.state.lock().summaries.get(project_id).cloned()
    }

    fn append(&self, entry: &JournalEntry) -> Result<(), TokenUsageLedgerError> {
        let Some(root) = &self.root else {
            return Ok(());
        };
        let bytes = serde_json::to_vec(entry).map_err(io::Error::other)?;
        if bytes.len() > MAX_JOURNAL_LINE_BYTES {
            return Err(invalid("token usage journal entry exceeds its size limit"));
        }
        let path = root.join(JOURNAL_FILE);
        let new_file = !path.exists();
        let mut options = OpenOptions::new();
        options.create(true).append(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        let mut journal = options.open(&path)?;
        journal.write_all(&bytes)?;
        journal.write_all(b"\n")?;
        journal.sync_all()?;
        drop(journal);
        if new_file {
            sync_directory(root)?;
        }
        Ok(())
    }

    fn maybe_compact(&self, state: &TokenUsageState) {
        let Some(root) = &self.root else {
            return;
        };
        let journal_path = root.join(JOURNAL_FILE);
        let Ok(metadata) = fs::metadata(&journal_path) else {
            return;
        };
        if metadata.len() < COMPACT_AFTER_JOURNAL_BYTES {
            return;
        }
        let _ = self.compact(state);
    }

    fn compact(&self, state: &TokenUsageState) -> Result<(), TokenUsageLedgerError> {
        let Some(root) = &self.root else {
            return Ok(());
        };
        let persisted = PersistedLedger {
            schema_version: SCHEMA_VERSION,
            state: encode_state(state),
        };
        let bytes = serde_json::to_vec(&persisted).map_err(io::Error::other)?;
        if bytes.len() > MAX_SNAPSHOT_BYTES {
            return Err(invalid("token usage snapshot exceeds its size limit"));
        }
        crate::workspace::atomic_write(
            &root.join(SNAPSHOT_FILE),
            &bytes,
            updated_temporary_file(),
        )?;
        let journal_path = root.join(JOURNAL_FILE);
        let mut options = OpenOptions::new();
        options.write(true).truncate(true);
        let journal = options.open(&journal_path)?;
        journal.sync_all()?;
        drop(journal);
        sync_directory(root)?;
        Ok(())
    }
}

fn replay_journal(
    path: &Path,
    state: &mut TokenUsageState,
) -> Result<Option<u64>, TokenUsageLedgerError> {
    let mut reader = BufReader::new(File::open(path)?);
    let mut line = Vec::with_capacity(512);
    let mut committed_bytes = 0_u64;
    loop {
        line.clear();
        let read = reader.read_until(b'\n', &mut line)?;
        if read == 0 {
            return Ok(None);
        }
        if line.len() > MAX_JOURNAL_LINE_BYTES + 1 {
            return Err(invalid("token usage journal line exceeds its size limit"));
        }
        // A crash can leave an un-fsynced final append without its delimiter.
        // Every acknowledged journal entry ends in a synced newline.
        if line.last() != Some(&b'\n') {
            return Ok(Some(committed_bytes));
        }
        committed_bytes = committed_bytes.saturating_add(read as u64);
        line.pop();
        if line.iter().all(|byte| byte.is_ascii_whitespace()) {
            continue;
        }
        let entry = serde_json::from_slice::<JournalEntry>(&line)
            .map_err(|error| invalid(format!("invalid token usage journal: {error}")))?;
        match entry {
            JournalEntry::Usage {
                project_id,
                provider_id,
                source_id,
                usage,
                updated_at,
            } => {
                validate_usage(&project_id, &provider_id, &source_id, &usage)?;
                apply_usage(
                    state,
                    &project_id,
                    &provider_id,
                    &source_id,
                    &usage,
                    updated_at,
                )?;
            }
            JournalEntry::Gap {
                project_id,
                provider_id,
                source_id,
                updated_at,
            } => {
                validate_id("project id", &project_id)?;
                validate_id("provider id", &provider_id)?;
                validate_id("source id", &source_id)?;
                apply_gap(state, &project_id, &provider_id, &source_id, updated_at);
            }
        }
    }
}

fn truncate_journal_tail(path: &Path, committed_bytes: u64) -> io::Result<()> {
    let journal = OpenOptions::new().write(true).open(path)?;
    journal.set_len(committed_bytes)?;
    journal.sync_all()?;
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "token usage journal does not have a parent directory",
        )
    })?;
    sync_directory(parent)
}

fn usage_would_change(
    state: &TokenUsageState,
    project_id: &str,
    provider_id: &str,
    source_id: &str,
    usage: &AgentTokenUsage,
) -> Result<bool, TokenUsageLedgerError> {
    let Some(project) = state.projects.get(project_id) else {
        return Ok(true);
    };
    let source_changed = project
        .sources
        .get(&ProviderSourceKey {
            provider_id: provider_id.to_string(),
            source_id: source_id.to_string(),
        })
        .is_none_or(|source| !source.reported);
    let key = ProviderSubjectKey {
        provider_id: provider_id.to_string(),
        subject_id: usage.id.clone(),
    };
    let subject_changed = match project.subjects.get(&key) {
        Some(existing) => {
            if existing.parent_id != usage.parent_id {
                return Err(invalid("a token subject changed parent identity"));
            }
            if existing.includes_children != usage.includes_children {
                return Err(invalid("a token subject changed counter scope"));
            }
            if usage.tokens > existing.tokens {
                true
            } else {
                usage.tokens == existing.tokens && existing.partial && !usage.partial
            }
        }
        None => {
            if parent_introduces_cycle(project, &key, usage.parent_id.as_deref()) {
                return Err(invalid("token subject parent relation forms a cycle"));
            }
            true
        }
    };
    Ok(source_changed || subject_changed)
}

fn gap_would_change(
    state: &TokenUsageState,
    project_id: &str,
    provider_id: &str,
    source_id: &str,
) -> bool {
    state.projects.get(project_id).is_none_or(|project| {
        !project.sources.contains_key(&ProviderSourceKey {
            provider_id: provider_id.to_string(),
            source_id: source_id.to_string(),
        })
    })
}

fn apply_usage(
    state: &mut TokenUsageState,
    project_id: &str,
    provider_id: &str,
    source_id: &str,
    usage: &AgentTokenUsage,
    updated_at: u64,
) -> Result<bool, TokenUsageLedgerError> {
    let project = state.projects.entry(project_id.to_string()).or_default();
    let source = project
        .sources
        .entry(ProviderSourceKey {
            provider_id: provider_id.to_string(),
            source_id: source_id.to_string(),
        })
        .or_default();
    let mut changed = !source.reported;
    source.reported = true;

    let key = ProviderSubjectKey {
        provider_id: provider_id.to_string(),
        subject_id: usage.id.clone(),
    };
    let subject_changed = match project.subjects.get_mut(&key) {
        Some(existing) => {
            if existing.parent_id != usage.parent_id {
                return Err(invalid("a token subject changed parent identity"));
            }
            if existing.includes_children != usage.includes_children {
                return Err(invalid("a token subject changed counter scope"));
            }
            if usage.tokens > existing.tokens {
                existing.tokens = usage.tokens;
                existing.partial = usage.partial;
                true
            } else if usage.tokens == existing.tokens && existing.partial && !usage.partial {
                existing.partial = false;
                true
            } else {
                false
            }
        }
        None => {
            if parent_introduces_cycle(project, &key, usage.parent_id.as_deref()) {
                return Err(invalid("token subject parent relation forms a cycle"));
            }
            project.subjects.insert(
                key,
                SubjectObservation {
                    parent_id: usage.parent_id.clone(),
                    tokens: usage.tokens,
                    includes_children: usage.includes_children,
                    partial: usage.partial,
                },
            );
            true
        }
    };
    changed |= subject_changed;
    if changed {
        project.updated_at = next_updated_at(project.updated_at, updated_at);
    }
    Ok(changed)
}

fn apply_gap(
    state: &mut TokenUsageState,
    project_id: &str,
    provider_id: &str,
    source_id: &str,
    updated_at: u64,
) -> bool {
    let project = state.projects.entry(project_id.to_string()).or_default();
    let key = ProviderSourceKey {
        provider_id: provider_id.to_string(),
        source_id: source_id.to_string(),
    };
    if project.sources.contains_key(&key) {
        return false;
    }
    project.sources.insert(key, SourceObservation::default());
    project.updated_at = next_updated_at(project.updated_at, updated_at);
    true
}

fn parent_introduces_cycle(
    project: &ProjectLedger,
    key: &ProviderSubjectKey,
    parent_id: Option<&str>,
) -> bool {
    let mut current = parent_id.map(|subject_id| ProviderSubjectKey {
        provider_id: key.provider_id.clone(),
        subject_id: subject_id.to_string(),
    });
    let mut visited = BTreeSet::new();
    while let Some(candidate) = current {
        if candidate == *key || !visited.insert(candidate.clone()) {
            return true;
        }
        current = project.subjects.get(&candidate).and_then(|subject| {
            subject
                .parent_id
                .as_ref()
                .map(|subject_id| ProviderSubjectKey {
                    provider_id: candidate.provider_id.clone(),
                    subject_id: subject_id.clone(),
                })
        });
    }
    false
}

fn summary_for(state: &TokenUsageState, project_id: &str) -> Option<ProjectTokenUsage> {
    let project = state.projects.get(project_id)?;
    let subjects_by_identity = project
        .subjects
        .keys()
        .map(|key| ((key.provider_id.as_str(), key.subject_id.as_str()), key))
        .collect::<BTreeMap<_, _>>();
    let mut children = project
        .subjects
        .keys()
        .map(|key| (key, Vec::new()))
        .collect::<BTreeMap<&ProviderSubjectKey, Vec<&ProviderSubjectKey>>>();
    for (key, subject) in &project.subjects {
        let Some(parent_id) = &subject.parent_id else {
            continue;
        };
        if let Some(parent) =
            subjects_by_identity.get(&(key.provider_id.as_str(), parent_id.as_str()))
        {
            children.entry(*parent).or_default().push(key);
        }
    }
    let attached = children
        .values()
        .flat_map(|keys| keys.iter().copied())
        .collect::<BTreeSet<_>>();
    let mut total = 0_u64;
    let mut visited = BTreeSet::new();
    for root in project.subjects.keys() {
        if !attached.contains(&root) {
            total = total.saturating_add(subject_total(root, project, &children, &mut visited));
        }
    }
    let partial = project.sources.values().any(|source| !source.reported)
        || project.subjects.values().any(|subject| subject.partial);
    let reported_sources = project
        .sources
        .values()
        .filter(|source| source.reported)
        .count() as u64;
    Some(ProjectTokenUsage {
        project_id: project_id.to_string(),
        tokens: (!project.subjects.is_empty()).then_some(total),
        partial,
        reported_sources,
        observed_sources: project.sources.len() as u64,
        updated_at: project.updated_at,
    })
}

fn subject_total<'a>(
    key: &'a ProviderSubjectKey,
    project: &ProjectLedger,
    children: &BTreeMap<&'a ProviderSubjectKey, Vec<&'a ProviderSubjectKey>>,
    visited: &mut BTreeSet<&'a ProviderSubjectKey>,
) -> u64 {
    if !visited.insert(key) {
        return 0;
    }
    let Some(subject) = project.subjects.get(key) else {
        return 0;
    };
    let descendants = children
        .get(&key)
        .into_iter()
        .flatten()
        .fold(0_u64, |total, child| {
            total.saturating_add(subject_total(child, project, children, visited))
        });
    if subject.includes_children {
        subject.tokens.max(descendants)
    } else {
        subject.tokens.saturating_add(descendants)
    }
}

fn next_updated_at(current: u64, observed_at: u64) -> u64 {
    observed_at.max(current.saturating_add(1))
}

fn validate_state(state: &TokenUsageState) -> Result<(), TokenUsageLedgerError> {
    for (project_id, project) in &state.projects {
        validate_id("project id", project_id)?;
        for source in project.sources.keys() {
            validate_id("provider id", &source.provider_id)?;
            validate_id("source id", &source.source_id)?;
        }
        for (key, subject) in &project.subjects {
            validate_id("provider id", &key.provider_id)?;
            validate_id("token subject id", &key.subject_id)?;
            if let Some(parent_id) = &subject.parent_id {
                validate_id("token parent id", parent_id)?;
                if parent_introduces_cycle(project, key, Some(parent_id)) {
                    return Err(invalid("token subject parent relation forms a cycle"));
                }
            }
        }
    }
    Ok(())
}

fn validate_usage(
    project_id: &str,
    provider_id: &str,
    source_id: &str,
    usage: &AgentTokenUsage,
) -> Result<(), TokenUsageLedgerError> {
    validate_id("project id", project_id)?;
    validate_id("provider id", provider_id)?;
    validate_id("source id", source_id)?;
    validate_id("token subject id", &usage.id)?;
    if let Some(parent_id) = &usage.parent_id {
        validate_id("token parent id", parent_id)?;
        if parent_id == &usage.id {
            return Err(invalid("token subject cannot parent itself"));
        }
    }
    Ok(())
}

fn validate_id(name: &str, value: &str) -> Result<(), TokenUsageLedgerError> {
    if value.trim().is_empty()
        || value.len() > MAX_ID_BYTES
        || value.contains('\n')
        || value.contains('\r')
    {
        return Err(invalid(format!("{name} is empty or too long")));
    }
    Ok(())
}

fn invalid(message: impl Into<String>) -> TokenUsageLedgerError {
    TokenUsageLedgerError::Invalid(message.into())
}

fn sync_directory(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        File::open(path)?.sync_all()
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Ok(())
    }
}

fn updated_temporary_file() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usage(
        id: &str,
        parent_id: Option<&str>,
        tokens: u64,
        includes_children: bool,
    ) -> AgentTokenUsage {
        AgentTokenUsage {
            id: id.to_string(),
            parent_id: parent_id.map(str::to_string),
            tokens,
            includes_children,
            partial: false,
        }
    }

    fn provider() -> ProviderId {
        ProviderId::from_static("omp")
    }

    #[test]
    fn duplicate_lower_and_distinct_sources_keep_monotonic_total() {
        let root = tempfile::tempdir().unwrap();
        let ledger = TokenUsageLedger::open(root.path()).unwrap();
        assert_eq!(
            ledger
                .observe(
                    "project",
                    &provider(),
                    "session-a",
                    &usage("request-a", None, 12, false),
                    1
                )
                .unwrap()
                .unwrap()
                .tokens,
            Some(12)
        );
        assert!(
            ledger
                .observe(
                    "project",
                    &provider(),
                    "session-a",
                    &usage("request-a", None, 12, false),
                    2
                )
                .unwrap()
                .is_none()
        );
        assert!(
            ledger
                .observe(
                    "project",
                    &provider(),
                    "session-a",
                    &usage("request-a", None, 7, false),
                    3
                )
                .unwrap()
                .is_none()
        );
        let summary = ledger
            .observe(
                "project",
                &provider(),
                "session-b",
                &usage("request-b", None, 9, false),
                4,
            )
            .unwrap()
            .unwrap();
        assert_eq!(summary.tokens, Some(21));
        assert_eq!(summary.reported_sources, 2);
        assert_eq!(summary.observed_sources, 2);
    }

    #[test]
    fn zero_is_reported_but_missing_source_remains_partial() {
        let root = tempfile::tempdir().unwrap();
        let ledger = TokenUsageLedger::open(root.path()).unwrap();
        let gap = ledger
            .observe_gap("project", &provider(), "unavailable-session", 1)
            .unwrap()
            .unwrap();
        assert_eq!(gap.tokens, None);
        assert!(gap.partial);
        let zero = ledger
            .observe(
                "project",
                &provider(),
                "zero-session",
                &usage("request-zero", None, 0, false),
                2,
            )
            .unwrap()
            .unwrap();
        assert_eq!(zero.tokens, Some(0));
        assert!(zero.partial);
        assert_eq!(zero.reported_sources, 1);
        assert_eq!(zero.observed_sources, 2);
    }

    #[test]
    fn newer_partial_counter_does_not_fabricate_complete_coverage() {
        let root = tempfile::tempdir().unwrap();
        let ledger = TokenUsageLedger::open(root.path()).unwrap();
        ledger
            .observe(
                "project",
                &provider(),
                "request",
                &usage("request", None, 10, false),
                1,
            )
            .unwrap();
        let mut partial = usage("request", None, 12, false);
        partial.partial = true;
        let summary = ledger
            .observe("project", &provider(), "request", &partial, 2)
            .unwrap()
            .unwrap();
        assert_eq!(summary.tokens, Some(12));
        assert!(summary.partial);
        assert!(
            ledger
                .observe(
                    "project",
                    &provider(),
                    "request",
                    &usage("request", None, 11, false),
                    3
                )
                .unwrap()
                .is_none()
        );
        assert!(ledger.summary("project").unwrap().partial);
        let summary = ledger
            .observe(
                "project",
                &provider(),
                "request",
                &usage("request", None, 12, false),
                4,
            )
            .unwrap()
            .unwrap();
        assert!(!summary.partial);
    }

    #[test]
    fn inclusive_parent_covers_child_in_either_arrival_order() {
        for parent_first in [true, false] {
            let root = tempfile::tempdir().unwrap();
            let ledger = TokenUsageLedger::open(root.path()).unwrap();
            let parent = usage("parent", None, 100, true);
            let child = usage("child", Some("parent"), 140, false);
            if parent_first {
                ledger
                    .observe("project", &provider(), "parent-source", &parent, 1)
                    .unwrap();
                ledger
                    .observe("project", &provider(), "child-source", &child, 2)
                    .unwrap();
            } else {
                ledger
                    .observe("project", &provider(), "child-source", &child, 1)
                    .unwrap();
                ledger
                    .observe("project", &provider(), "parent-source", &parent, 2)
                    .unwrap();
            }
            assert_eq!(ledger.summary("project").unwrap().tokens, Some(140));
        }
    }

    #[test]
    fn durable_journal_outlives_reopen_and_source_closure() {
        let root = tempfile::tempdir().unwrap();
        let ledger = TokenUsageLedger::open(root.path()).unwrap();
        ledger
            .observe(
                "project",
                &provider(),
                "closed-session",
                &usage("request", None, 33, false),
                7,
            )
            .unwrap();
        drop(ledger);
        let reopened = TokenUsageLedger::open(root.path()).unwrap();
        let summary = reopened.summary("project").unwrap();
        assert_eq!(summary.tokens, Some(33));
        assert_eq!(summary.reported_sources, 1);
    }

    #[test]
    fn compacted_snapshot_reopens_with_dedup_state() {
        let root = tempfile::tempdir().unwrap();
        let ledger = TokenUsageLedger::open(root.path()).unwrap();
        ledger
            .observe(
                "project",
                &provider(),
                "request-a",
                &usage("request-a", None, 33, false),
                1,
            )
            .unwrap();
        ledger
            .observe(
                "project",
                &provider(),
                "request-b",
                &usage("request-b", None, 7, false),
                2,
            )
            .unwrap();
        let expected = ledger.summary("project").unwrap();
        {
            let state = ledger.state.lock();
            ledger.compact(&state).unwrap();
        }
        assert_eq!(
            fs::metadata(root.path().join(JOURNAL_FILE)).unwrap().len(),
            0
        );
        drop(ledger);

        let reopened = TokenUsageLedger::open(root.path()).unwrap();
        assert_eq!(reopened.summary("project"), Some(expected));
        assert!(
            reopened
                .observe(
                    "project",
                    &provider(),
                    "request-a",
                    &usage("request-a", None, 2, false),
                    3
                )
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn torn_unacknowledged_journal_tail_does_not_hide_durable_usage() {
        let root = tempfile::tempdir().unwrap();
        let ledger = TokenUsageLedger::open(root.path()).unwrap();
        ledger
            .observe(
                "project",
                &provider(),
                "request",
                &usage("request", None, 33, false),
                1,
            )
            .unwrap();
        drop(ledger);
        let mut journal = OpenOptions::new()
            .append(true)
            .open(root.path().join(JOURNAL_FILE))
            .unwrap();
        std::io::Write::write_all(&mut journal, br#"{"kind":"usage""#).unwrap();
        journal.sync_all().unwrap();
        drop(journal);

        let reopened = TokenUsageLedger::open(root.path()).unwrap();
        assert_eq!(reopened.summary("project").unwrap().tokens, Some(33));
        reopened
            .observe(
                "project",
                &provider(),
                "request-two",
                &usage("request-two", None, 4, false),
                2,
            )
            .unwrap();
        drop(reopened);

        let replayed = TokenUsageLedger::open(root.path()).unwrap();
        assert_eq!(replayed.summary("project").unwrap().tokens, Some(37));
    }

    #[test]
    fn invalid_parent_cycles_and_persistence_failures_are_not_acknowledged() {
        let root = tempfile::tempdir().unwrap();
        let ledger = TokenUsageLedger::open(root.path()).unwrap();
        assert!(
            ledger
                .observe(
                    "project",
                    &provider(),
                    "source",
                    &usage("parent", Some("child"), 1, false),
                    1
                )
                .is_ok()
        );
        assert!(
            ledger
                .observe(
                    "project",
                    &provider(),
                    "source",
                    &usage("child", Some("parent"), 1, false),
                    2
                )
                .is_err()
        );

        let failing_root = tempfile::tempdir().unwrap();
        let failing = TokenUsageLedger::open(failing_root.path()).unwrap();
        fs::remove_dir_all(failing_root.path()).unwrap();
        assert!(
            failing
                .observe(
                    "project",
                    &provider(),
                    "source",
                    &usage("request", None, 1, false),
                    1
                )
                .is_err()
        );
        assert!(failing.summary("project").is_none());
    }
}
