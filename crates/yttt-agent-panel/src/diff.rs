//! Tools report an edit as old and new text, while the diff view reads a
//! unified patch. This bridges the two.
use similar::{ChangeTag, TextDiff};

const CONTEXT_LINES: usize = 3;

/// A unified patch for one file, empty when nothing changed. A file without
/// `old` is a new file and diffs against `/dev/null`.
pub fn unified_patch(path: &str, old: Option<&str>, new: &str) -> String {
    let diff = TextDiff::from_lines(old.unwrap_or(""), new);
    let old_header = match old {
        Some(_) => format!("a/{path}"),
        None => "/dev/null".to_owned(),
    };
    diff.unified_diff()
        .context_radius(CONTEXT_LINES)
        .header(&old_header, &format!("b/{path}"))
        .to_string()
}

/// Added and removed line counts.
pub fn line_stats(old: Option<&str>, new: &str) -> (usize, usize) {
    TextDiff::from_lines(old.unwrap_or(""), new)
        .iter_all_changes()
        .fold((0, 0), |(added, removed), change| match change.tag() {
            ChangeTag::Insert => (added + 1, removed),
            ChangeTag::Delete => (added, removed + 1),
            ChangeTag::Equal => (added, removed),
        })
}

#[cfg(test)]
mod tests {
    use gpui_component::diff::DiffFile;

    use super::*;

    #[test]
    fn a_modification_parses_as_one_file() {
        let patch = unified_patch("src/main.rs", Some("a\nb\nc\n"), "a\nB\nc\n");
        assert!(patch.starts_with("--- a/src/main.rs\n+++ b/src/main.rs\n"));
        let files = DiffFile::parse(&patch).expect("patch parses");
        assert_eq!(files.len(), 1);
        assert_eq!(line_stats(Some("a\nb\nc\n"), "a\nB\nc\n"), (1, 1));
    }

    #[test]
    fn a_new_file_diffs_against_dev_null() {
        let patch = unified_patch("notes.md", None, "one\ntwo\n");
        assert!(patch.starts_with("--- /dev/null\n+++ b/notes.md\n"));
        assert_eq!(DiffFile::parse(&patch).expect("patch parses").len(), 1);
        assert_eq!(line_stats(None, "one\ntwo\n"), (2, 0));
    }

    #[test]
    fn a_missing_trailing_newline_still_parses() {
        let patch = unified_patch("a.txt", Some("x\ny"), "x\nz");
        assert_eq!(DiffFile::parse(&patch).expect("patch parses").len(), 1);
    }

    #[test]
    fn identical_text_has_no_patch() {
        assert_eq!(unified_patch("a.txt", Some("same\n"), "same\n"), "");
        assert_eq!(line_stats(Some("same\n"), "same\n"), (0, 0));
    }
}
