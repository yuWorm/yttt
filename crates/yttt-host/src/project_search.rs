//! Text search over a local project's files, skipping what Git ignores.

use std::{
    io::Read,
    path::Path,
    time::{Duration, Instant},
};

use regex::{Regex, RegexBuilder};
use yttt_protocol::path::ProjectRelativePath;
use yttt_protocol::project::{
    ProjectSearchFile, ProjectSearchLine, ProjectSearchQuery, ProjectSearchRange,
    ProjectSearchResults,
};

/// Longer searches answer with what they found so far, inside the Client's
/// request timeout.
const TIME_BUDGET: Duration = Duration::from_secs(5);
const MAX_MATCHES: u32 = 5_000;
const MAX_FILE_BYTES: u64 = 4 * 1024 * 1024;
/// A NUL byte in this prefix marks a file as binary.
const BINARY_PROBE_BYTES: usize = 8 * 1024;
/// Longer lines are cut to a window around their first match.
const MAX_LINE_BYTES: usize = 320;
const CONTEXT_BEFORE_MATCH: usize = 48;
const REGEX_SIZE_LIMIT: usize = 1 << 20;

pub(crate) fn search(
    root: &Path,
    query: &ProjectSearchQuery,
) -> Result<ProjectSearchResults, String> {
    search_with_budget(root, query, TIME_BUDGET)
}

fn search_with_budget(
    root: &Path,
    query: &ProjectSearchQuery,
    budget: Duration,
) -> Result<ProjectSearchResults, String> {
    if query.pattern.is_empty() {
        return Ok(ProjectSearchResults::default());
    }
    let matcher = build_regex(query)?;
    let max_matches = query.max_matches.clamp(1, MAX_MATCHES);
    let started = Instant::now();
    let mut results = ProjectSearchResults::default();
    let walker = ignore::WalkBuilder::new(root)
        // Dotfiles such as `.github/` are searched; Git's own store is not.
        .hidden(false)
        .filter_entry(|entry| entry.file_name() != ".git")
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .parents(true)
        .require_git(false)
        .sort_by_file_name(|left, right| left.cmp(right))
        .build();
    for entry in walker {
        if started.elapsed() >= budget {
            results.truncated = true;
            break;
        }
        let Ok(entry) = entry else {
            continue;
        };
        if !entry.file_type().is_some_and(|kind| kind.is_file()) {
            continue;
        }
        let Some(text) = read_text(entry.path()) else {
            continue;
        };
        let remaining = max_matches - results.match_count;
        let (lines, count, hit_limit) = search_text(&text, &matcher, remaining);
        if !lines.is_empty() {
            let Ok(relative) = entry.path().strip_prefix(root) else {
                continue;
            };
            let Ok(relative_path) = ProjectRelativePath::from_path(relative) else {
                continue;
            };
            results.files.push(ProjectSearchFile {
                relative_path,
                lines,
            });
            results.match_count += count;
        }
        if hit_limit {
            results.truncated = true;
            break;
        }
    }
    Ok(results)
}

fn build_regex(query: &ProjectSearchQuery) -> Result<Regex, String> {
    let pattern = if query.regex {
        query.pattern.clone()
    } else {
        regex::escape(&query.pattern)
    };
    let pattern = if query.whole_word {
        format!(r"\b(?:{pattern})\b")
    } else {
        pattern
    };
    RegexBuilder::new(&pattern)
        .case_insensitive(!query.case_sensitive)
        .size_limit(REGEX_SIZE_LIMIT)
        .build()
        .map_err(|error| error.to_string())
}

fn read_text(path: &Path) -> Option<String> {
    let file = std::fs::File::open(path).ok()?;
    if file.metadata().ok()?.len() > MAX_FILE_BYTES {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(MAX_FILE_BYTES).read_to_end(&mut bytes).ok()?;
    if bytes[..bytes.len().min(BINARY_PROBE_BYTES)].contains(&0) {
        return None;
    }
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

/// The matching lines of `text`, how many matches they hold, and whether the
/// search stopped at `limit` matches.
fn search_text(text: &str, matcher: &Regex, limit: u32) -> (Vec<ProjectSearchLine>, u32, bool) {
    let mut lines = Vec::new();
    let mut count = 0;
    for (index, line) in text.lines().enumerate() {
        let mut ranges = Vec::new();
        for found in matcher.find_iter(line) {
            if found.start() == found.end() {
                continue;
            }
            if count == limit {
                if !ranges.is_empty() {
                    lines.push(display_line(index, line, &ranges));
                }
                return (lines, count, true);
            }
            ranges.push(found.start()..found.end());
            count += 1;
        }
        if !ranges.is_empty() {
            lines.push(display_line(index, line, &ranges));
        }
    }
    (lines, count, false)
}

fn display_line(index: usize, line: &str, ranges: &[std::ops::Range<usize>]) -> ProjectSearchLine {
    let trimmed_start = line.len() - line.trim_start().len();
    let first = ranges[0].start;
    let mut start = if line.len() - trimmed_start > MAX_LINE_BYTES {
        first
            .saturating_sub(CONTEXT_BEFORE_MATCH)
            .max(trimmed_start)
    } else {
        trimmed_start
    };
    while !line.is_char_boundary(start) {
        start -= 1;
    }
    let mut end = (start + MAX_LINE_BYTES).min(line.len());
    while !line.is_char_boundary(end) {
        end -= 1;
    }
    let text = line[start..end].trim_end();
    let matches = ranges
        .iter()
        .filter(|range| range.start >= start && range.start < start + text.len())
        .map(|range| ProjectSearchRange {
            start: (range.start - start) as u32,
            end: (range.end.min(start + text.len()) - start) as u32,
        })
        .collect();
    ProjectSearchLine {
        line: index as u32,
        text: text.to_string(),
        column_offset: line[..start].encode_utf16().count() as u32,
        matches,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn query(pattern: &str) -> ProjectSearchQuery {
        ProjectSearchQuery {
            pattern: pattern.to_string(),
            case_sensitive: false,
            whole_word: false,
            regex: false,
            max_matches: 100,
        }
    }

    fn project() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::create_dir_all(root.join("target")).unwrap();
        std::fs::write(root.join(".gitignore"), "target/\n").unwrap();
        std::fs::write(
            root.join("src/main.rs"),
            "fn main() {\n    let port = 8080;\n    serve(port);\n}\n",
        )
        .unwrap();
        std::fs::write(root.join("src/lib.rs"), "pub const PORT: u16 = 3000;\n").unwrap();
        std::fs::write(root.join("target/out.rs"), "let port = 1;\n").unwrap();
        std::fs::write(root.join("image.bin"), b"port\0\0\0").unwrap();
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::write(root.join(".git/config"), "port = 1\n").unwrap();
        std::fs::create_dir_all(root.join(".github")).unwrap();
        std::fs::write(root.join(".github/ci.yml"), "port: 9\n").unwrap();
        dir
    }

    fn paths(results: &ProjectSearchResults) -> Vec<String> {
        results
            .files
            .iter()
            .map(|file| file.relative_path.to_utf8().unwrap())
            .collect()
    }

    #[test]
    fn finds_lines_in_order_and_skips_ignored_and_binary_files() {
        let dir = project();
        let results = search(dir.path(), &query("port")).unwrap();
        assert_eq!(
            paths(&results),
            vec![".github/ci.yml", "src/lib.rs", "src/main.rs"]
        );
        assert_eq!(results.match_count, 4);
        assert!(!results.truncated);
        let main = &results.files[2];
        assert_eq!(main.lines[0].line, 1);
        assert_eq!(main.lines[0].text, "let port = 8080;");
        assert_eq!(main.lines[0].column_offset, 4);
        assert_eq!(
            main.lines[0].matches,
            vec![ProjectSearchRange { start: 4, end: 8 }]
        );
    }

    #[test]
    fn case_whole_word_and_regex_options_apply() {
        let dir = project();
        let mut case = query("PORT");
        case.case_sensitive = true;
        assert_eq!(
            paths(&search(dir.path(), &case).unwrap()),
            vec!["src/lib.rs"]
        );

        let mut word = query("serve");
        word.whole_word = true;
        assert_eq!(search(dir.path(), &word).unwrap().match_count, 1);
        word.pattern = "serv".to_string();
        assert_eq!(search(dir.path(), &word).unwrap().match_count, 0);

        let mut pattern = query(r"\d{4}");
        pattern.regex = true;
        assert_eq!(search(dir.path(), &pattern).unwrap().match_count, 2);
        pattern.pattern = "(".to_string();
        assert!(search(dir.path(), &pattern).is_err());
    }

    #[test]
    fn stops_at_the_match_limit() {
        let dir = project();
        let mut limited = query("port");
        limited.max_matches = 2;
        let results = search(dir.path(), &limited).unwrap();
        assert_eq!(results.match_count, 2);
        assert!(results.truncated);
    }

    #[test]
    fn a_spent_time_budget_truncates() {
        let dir = project();
        let results = search_with_budget(dir.path(), &query("port"), Duration::ZERO).unwrap();
        assert!(results.truncated);
        assert!(results.files.is_empty());
    }

    #[test]
    fn a_long_line_is_cut_around_its_first_match() {
        let line = format!("{}needle{}", "a".repeat(1000), "b".repeat(1000));
        let matcher = build_regex(&query("needle")).unwrap();
        let (lines, count, _) = search_text(&line, &matcher, 10);
        assert_eq!(count, 1);
        let shown = &lines[0];
        assert!(shown.text.len() <= MAX_LINE_BYTES);
        let range = shown.matches[0];
        assert_eq!(
            &shown.text[range.start as usize..range.end as usize],
            "needle"
        );
        assert_eq!(shown.column_offset as usize + range.start as usize, 1000);
    }

    #[test]
    fn an_empty_pattern_finds_nothing() {
        let dir = project();
        assert_eq!(
            search(dir.path(), &query("")).unwrap(),
            ProjectSearchResults::default()
        );
    }
}
