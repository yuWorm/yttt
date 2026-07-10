use std::{fs, path::Path};

use tempfile::tempdir;
use yttt::ui::editor::{
    CurrentDiskState, MAX_PROJECT_FILE_BYTES, ProjectFileIoError, SaveMode, SaveProjectFileOutcome,
    read_project_file, save_project_file,
};

#[test]
fn read_project_file_returns_text_paths_and_fingerprint() {
    let root = tempdir().unwrap();
    fs::create_dir_all(root.path().join("src")).unwrap();
    fs::write(root.path().join("src/main.rs"), "fn main() {}\n").unwrap();

    let loaded = read_project_file(root.path(), Path::new("src/main.rs")).unwrap();

    assert_eq!(
        loaded.canonical_path,
        root.path().join("src/main.rs").canonicalize().unwrap()
    );
    assert_eq!(loaded.relative_path, Path::new("src/main.rs"));
    assert_eq!(loaded.text, "fn main() {}\n");
    assert!(loaded.fingerprint.exists);
    assert_eq!(loaded.fingerprint.byte_len, 13);
    assert!(loaded.fingerprint.modified.is_some());
}

#[test]
fn read_project_file_rejects_paths_outside_the_project() {
    let root = tempdir().unwrap();
    let absolute = root.path().join("inside.txt");
    fs::write(&absolute, "inside").unwrap();

    for path in [absolute.as_path(), Path::new("../outside.txt")] {
        assert!(matches!(
            read_project_file(root.path(), path),
            Err(ProjectFileIoError::PathOutsideProject { .. })
        ));
    }
}

#[cfg(unix)]
#[test]
fn read_project_file_rejects_symlinks_escaping_the_project() {
    use std::os::unix::fs::symlink;

    let root = tempdir().unwrap();
    let outside = tempdir().unwrap();
    fs::write(outside.path().join("secret.txt"), "secret").unwrap();
    symlink(
        outside.path().join("secret.txt"),
        root.path().join("escape.txt"),
    )
    .unwrap();

    assert!(matches!(
        read_project_file(root.path(), Path::new("escape.txt")),
        Err(ProjectFileIoError::PathOutsideProject { .. })
    ));
}

#[test]
fn read_project_file_rejects_binary_and_non_utf8_content() {
    let root = tempdir().unwrap();
    fs::write(root.path().join("nul.bin"), b"hello\0world").unwrap();
    fs::write(root.path().join("invalid.txt"), [0xff, 0xfe]).unwrap();

    assert!(matches!(
        read_project_file(root.path(), Path::new("nul.bin")),
        Err(ProjectFileIoError::BinaryContent { .. })
    ));
    assert!(matches!(
        read_project_file(root.path(), Path::new("invalid.txt")),
        Err(ProjectFileIoError::InvalidUtf8 { .. })
    ));
}

#[test]
fn read_project_file_rejects_files_over_the_size_limit() {
    let root = tempdir().unwrap();
    let path = root.path().join("large.txt");
    fs::File::create(&path)
        .unwrap()
        .set_len(MAX_PROJECT_FILE_BYTES + 1)
        .unwrap();

    assert!(matches!(
        read_project_file(root.path(), Path::new("large.txt")),
        Err(ProjectFileIoError::FileTooLarge { size, limit, .. })
            if size == MAX_PROJECT_FILE_BYTES + 1 && limit == MAX_PROJECT_FILE_BYTES
    ));
}

#[test]
fn save_project_file_writes_text_and_returns_the_new_fingerprint() {
    let root = tempdir().unwrap();
    fs::write(root.path().join("notes.txt"), "old").unwrap();
    let loaded = read_project_file(root.path(), Path::new("notes.txt")).unwrap();

    let outcome = save_project_file(
        root.path(),
        Path::new("notes.txt"),
        "new text",
        SaveMode::Check(&loaded.fingerprint),
    )
    .unwrap();

    let SaveProjectFileOutcome::Saved(fingerprint) = outcome else {
        panic!("expected a successful save");
    };
    assert_eq!(
        fs::read_to_string(root.path().join("notes.txt")).unwrap(),
        "new text"
    );
    assert_eq!(fingerprint.byte_len, 8);
    assert_ne!(fingerprint.content_hash, loaded.fingerprint.content_hash);
}

#[test]
fn save_project_file_reports_external_conflict() {
    let root = tempdir().unwrap();
    fs::write(root.path().join("notes.txt"), "aaaa").unwrap();
    let loaded = read_project_file(root.path(), Path::new("notes.txt")).unwrap();
    fs::write(root.path().join("notes.txt"), "bbbb").unwrap();

    let outcome = save_project_file(
        root.path(),
        Path::new("notes.txt"),
        "mine",
        SaveMode::Check(&loaded.fingerprint),
    )
    .unwrap();

    assert!(matches!(
        outcome,
        SaveProjectFileOutcome::Conflict(CurrentDiskState::Present(current))
            if current.byte_len == loaded.fingerprint.byte_len
                && current.content_hash != loaded.fingerprint.content_hash
    ));
    assert_eq!(
        fs::read_to_string(root.path().join("notes.txt")).unwrap(),
        "bbbb"
    );
}

#[test]
fn save_project_file_reports_deletion_and_force_can_recreate() {
    let root = tempdir().unwrap();
    fs::write(root.path().join("notes.txt"), "old").unwrap();
    let loaded = read_project_file(root.path(), Path::new("notes.txt")).unwrap();
    fs::remove_file(root.path().join("notes.txt")).unwrap();

    let conflict = save_project_file(
        root.path(),
        Path::new("notes.txt"),
        "mine",
        SaveMode::Check(&loaded.fingerprint),
    )
    .unwrap();
    assert_eq!(
        conflict,
        SaveProjectFileOutcome::Conflict(CurrentDiskState::Missing)
    );
    assert!(!root.path().join("notes.txt").exists());

    let forced =
        save_project_file(root.path(), Path::new("notes.txt"), "mine", SaveMode::Force).unwrap();
    assert!(matches!(forced, SaveProjectFileOutcome::Saved(_)));
    assert_eq!(
        fs::read_to_string(root.path().join("notes.txt")).unwrap(),
        "mine"
    );
}

#[cfg(unix)]
#[test]
fn save_project_file_preserves_unix_permissions() {
    use std::os::unix::fs::PermissionsExt;

    let root = tempdir().unwrap();
    let path = root.path().join("script.sh");
    fs::write(&path, "old").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
    let loaded = read_project_file(root.path(), Path::new("script.sh")).unwrap();

    save_project_file(
        root.path(),
        Path::new("script.sh"),
        "new",
        SaveMode::Check(&loaded.fingerprint),
    )
    .unwrap();

    assert_eq!(
        fs::metadata(path).unwrap().permissions().mode() & 0o777,
        0o640
    );
}
