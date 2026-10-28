//! Backups of persisted workspace records.
//!
//! Before a record is replaced, the current file becomes `backup-1.json`
//! (older copies shift to `backup-2` and `backup-3`). At most every 15 minutes
//! the current file is also kept as `history-<millis>.json`, up to 48 copies.
//! A record that cannot be read is moved aside as `corrupt-<millis>.json`, and
//! the newest readable copy replaces it.
//!
//! Backups are best effort: failing to write one never blocks persistence.

use std::{
    fs, io,
    path::{Path, PathBuf},
};

const ROTATED_BACKUPS: usize = 3;
const HISTORY_INTERVAL_MILLIS: u64 = 15 * 60 * 1000;
const HISTORY_COPIES: usize = 48;

pub(crate) struct WorkspaceBackups {
    root: PathBuf,
}

impl WorkspaceBackups {
    pub(crate) fn new(state_root: &Path) -> Self {
        Self {
            root: state_root.join("workspace-backups"),
        }
    }

    fn directory(&self, workspace_id: &str) -> PathBuf {
        self.root.join(workspace_id)
    }

    /// Keeps the current record at `current` before it is replaced.
    pub(crate) fn preserve(&self, workspace_id: &str, current: &Path, now_millis: u64) {
        if let Err(error) = self.try_preserve(workspace_id, current, now_millis) {
            eprintln!("workspace {workspace_id} backup failed: {error}");
        }
    }

    fn try_preserve(&self, workspace_id: &str, current: &Path, now_millis: u64) -> io::Result<()> {
        if !current.try_exists()? {
            return Ok(());
        }
        let directory = self.directory(workspace_id);
        fs::create_dir_all(&directory)?;
        for index in (1..ROTATED_BACKUPS).rev() {
            let from = directory.join(format!("backup-{index}.json"));
            if from.try_exists()? {
                fs::rename(&from, directory.join(format!("backup-{}.json", index + 1)))?;
            }
        }
        fs::copy(current, directory.join("backup-1.json"))?;

        let mut history = history_copies(&directory)?;
        if history
            .last()
            .is_none_or(|(saved, _)| now_millis.saturating_sub(*saved) >= HISTORY_INTERVAL_MILLIS)
        {
            let path = directory.join(format!("history-{now_millis}.json"));
            fs::copy(current, &path)?;
            history.push((now_millis, path));
        }
        let excess = history.len().saturating_sub(HISTORY_COPIES);
        for (_, path) in history.drain(..excess) {
            fs::remove_file(path)?;
        }
        Ok(())
    }

    /// Moves an unreadable record aside and returns the newest copy, rotated
    /// backups first, for which `accept` succeeds.
    pub(crate) fn recover<T>(
        &self,
        workspace_id: &str,
        unreadable: &Path,
        now_millis: u64,
        mut accept: impl FnMut(&[u8]) -> io::Result<T>,
    ) -> io::Result<Option<(Vec<u8>, T)>> {
        let directory = self.directory(workspace_id);
        fs::create_dir_all(&directory)?;
        fs::rename(
            unreadable,
            directory.join(format!("corrupt-{now_millis}.json")),
        )?;
        let mut candidates = (1..=ROTATED_BACKUPS)
            .map(|index| directory.join(format!("backup-{index}.json")))
            .collect::<Vec<_>>();
        candidates.extend(
            history_copies(&directory)?
                .into_iter()
                .rev()
                .map(|(_, path)| path),
        );
        for candidate in candidates {
            let bytes = match fs::read(&candidate) {
                Ok(bytes) => bytes,
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error),
            };
            match accept(&bytes) {
                Ok(value) => return Ok(Some((bytes, value))),
                Err(error) => {
                    eprintln!(
                        "workspace {workspace_id} backup {} is unreadable: {error}",
                        candidate.display()
                    );
                }
            }
        }
        Ok(None)
    }

    pub(crate) fn remove(&self, workspace_id: &str) {
        match fs::remove_dir_all(self.directory(workspace_id)) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => eprintln!("workspace {workspace_id} backups were not removed: {error}"),
        }
    }
}

/// History copies sorted oldest first.
fn history_copies(directory: &Path) -> io::Result<Vec<(u64, PathBuf)>> {
    let mut copies = Vec::new();
    for entry in fs::read_dir(directory)? {
        let path = entry?.path();
        if let Some(saved) = path
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| name.strip_prefix("history-"))
            .and_then(|name| name.strip_suffix(".json"))
            .and_then(|millis| millis.parse::<u64>().ok())
        {
            copies.push((saved, path));
        }
    }
    copies.sort_by_key(|(saved, _)| *saved);
    Ok(copies)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINUTE: u64 = 60 * 1000;

    fn saves(backups: &WorkspaceBackups, current: &Path, contents: &[(&str, u64)]) {
        for (content, now) in contents {
            backups.preserve("w", current, *now);
            fs::write(current, content).unwrap();
        }
    }

    #[test]
    fn rotation_keeps_the_three_previous_records() {
        let temp = tempfile::tempdir().unwrap();
        let backups = WorkspaceBackups::new(temp.path());
        let current = temp.path().join("w.json");
        saves(
            &backups,
            &current,
            &[("1", 0), ("2", 1), ("3", 2), ("4", 3), ("5", 4)],
        );
        let directory = temp.path().join("workspace-backups/w");
        let read = |name: &str| fs::read_to_string(directory.join(name)).unwrap();
        assert_eq!(read("backup-1.json"), "4");
        assert_eq!(read("backup-2.json"), "3");
        assert_eq!(read("backup-3.json"), "2");
        assert!(!directory.join("backup-4.json").exists());
    }

    #[test]
    fn history_is_kept_at_most_every_fifteen_minutes_and_bounded() {
        let temp = tempfile::tempdir().unwrap();
        let backups = WorkspaceBackups::new(temp.path());
        let current = temp.path().join("w.json");
        fs::write(&current, "0").unwrap();
        backups.preserve("w", &current, 0);
        backups.preserve("w", &current, 14 * MINUTE);
        backups.preserve("w", &current, 15 * MINUTE);
        let directory = temp.path().join("workspace-backups/w");
        let saved = history_copies(&directory)
            .unwrap()
            .into_iter()
            .map(|(saved, _)| saved)
            .collect::<Vec<_>>();
        assert_eq!(saved, [0, 15 * MINUTE]);

        for copy in 2..60 {
            backups.preserve("w", &current, copy * 15 * MINUTE);
        }
        let saved = history_copies(&directory).unwrap();
        assert_eq!(saved.len(), HISTORY_COPIES);
        assert_eq!(saved.last().unwrap().0, 59 * 15 * MINUTE);
    }

    #[test]
    fn recovery_moves_the_unreadable_record_aside_and_uses_the_newest_readable_copy() {
        let temp = tempfile::tempdir().unwrap();
        let backups = WorkspaceBackups::new(temp.path());
        let current = temp.path().join("w.json");
        saves(
            &backups,
            &current,
            &[("good-old", 0), ("good-new", 1), ("bad", 2)],
        );
        // backup-1 holds "good-new"; make it unreadable too.
        let directory = temp.path().join("workspace-backups/w");
        fs::write(directory.join("backup-1.json"), "bad").unwrap();

        let recovered = backups
            .recover("w", &current, 3, |bytes| {
                if bytes.starts_with(b"good") {
                    Ok(bytes.len())
                } else {
                    Err(io::Error::other("unreadable"))
                }
            })
            .unwrap()
            .unwrap();
        assert_eq!(recovered.0, b"good-old");
        assert!(!current.exists());
        assert_eq!(
            fs::read_to_string(directory.join("corrupt-3.json")).unwrap(),
            "bad"
        );
    }

    #[test]
    fn recovery_without_a_readable_copy_returns_none() {
        let temp = tempfile::tempdir().unwrap();
        let backups = WorkspaceBackups::new(temp.path());
        let current = temp.path().join("w.json");
        fs::write(&current, "bad").unwrap();
        let recovered = backups
            .recover("w", &current, 1, |_| Err::<(), _>(io::Error::other("bad")))
            .unwrap();
        assert!(recovered.is_none());
    }
}
