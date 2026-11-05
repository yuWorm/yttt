//! Composer text rules that need no window: when `@` or `/` opens the
//! candidate popover, how a submitted draft splits into prompt segments, and
//! how candidates rank against a query.
use std::ops::Range;

use crate::event::PromptSegment;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TriggerKind {
    Mention,
    Command,
}

/// An open trigger: `range` covers the trigger character through the cursor,
/// which is what accepting a candidate replaces.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Trigger {
    pub kind: TriggerKind,
    pub range: Range<usize>,
    pub query: String,
}

/// A mention token as the draft holds it, in source bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TokenRange {
    pub range: Range<usize>,
    pub id: String,
    pub label: String,
}

/// `/` opens only at the very start of the draft; `@` opens at the start of
/// a line or after whitespace. Either closes once whitespace follows it, and
/// an `@` inside an existing token never reopens one.
pub fn detect_trigger(text: &str, cursor: usize, tokens: &[Range<usize>]) -> Option<Trigger> {
    let before = text.get(..cursor)?;
    if let Some(query) = before.strip_prefix('/')
        && !query.contains(char::is_whitespace)
    {
        return Some(Trigger {
            kind: TriggerKind::Command,
            range: 0..cursor,
            query: query.to_owned(),
        });
    }
    let at = before.rfind('@')?;
    let query = &before[at + 1..];
    let starts_word = before[..at]
        .chars()
        .next_back()
        .is_none_or(char::is_whitespace);
    let inside_token = tokens.iter().any(|token| token.contains(&at));
    if !starts_word || inside_token || query.contains(char::is_whitespace) {
        return None;
    }
    Some(Trigger {
        kind: TriggerKind::Mention,
        range: at..cursor,
        query: query.to_owned(),
    })
}

/// Splits a draft into text and mention segments, trimming the draft's outer
/// whitespace and dropping empty text between adjacent tokens.
pub fn prompt_segments(text: &str, tokens: &[TokenRange]) -> Vec<PromptSegment> {
    let mut tokens = tokens.to_vec();
    tokens.sort_by_key(|token| token.range.start);
    let mut segments = Vec::new();
    let mut offset = 0;
    for token in tokens {
        push_text(&mut segments, &text[offset..token.range.start]);
        segments.push(PromptSegment::Mention {
            id: token.id,
            label: token.label,
        });
        offset = token.range.end;
    }
    push_text(&mut segments, &text[offset..]);

    if let Some(PromptSegment::Text(first)) = segments.first_mut() {
        *first = first.trim_start().to_owned();
    }
    if let Some(PromptSegment::Text(last)) = segments.last_mut() {
        *last = last.trim_end().to_owned();
    }
    segments.retain(|segment| !matches!(segment, PromptSegment::Text(text) if text.is_empty()));
    segments
}

fn push_text(segments: &mut Vec<PromptSegment>, text: &str) {
    if !text.is_empty() {
        segments.push(PromptSegment::Text(text.to_owned()));
    }
}

/// Keeps the items whose key matches `query` case-insensitively, prefix
/// matches first, then substring, then subsequence; ties keep input order.
pub fn rank<'a, T>(query: &str, items: &'a [T], key: impl Fn(&T) -> &str) -> Vec<&'a T> {
    let query = query.to_lowercase();
    let mut scored = items
        .iter()
        .filter_map(|item| match_score(&query, &key(item).to_lowercase()).map(|s| (s, item)))
        .collect::<Vec<_>>();
    scored.sort_by_key(|(score, _)| *score);
    scored.into_iter().map(|(_, item)| item).collect()
}

fn match_score(query: &str, candidate: &str) -> Option<u8> {
    if candidate.starts_with(query) {
        return Some(0);
    }
    if candidate.contains(query) {
        return Some(1);
    }
    let mut rest = candidate.chars();
    query
        .chars()
        .all(|c| rest.any(|candidate_char| candidate_char == c))
        .then_some(2)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trigger(text: &str) -> Option<Trigger> {
        detect_trigger(text, text.len(), &[])
    }

    #[test]
    fn at_opens_a_mention_at_line_start_and_after_whitespace() {
        assert_eq!(
            trigger("@ma"),
            Some(Trigger {
                kind: TriggerKind::Mention,
                range: 0..3,
                query: "ma".to_owned(),
            })
        );
        assert_eq!(trigger("look at @src/m").map(|t| t.range), Some(8..14));
        assert_eq!(trigger("line\n@x").map(|t| t.query), Some("x".to_owned()));
        assert_eq!(trigger("@").map(|t| t.query), Some(String::new()));
    }

    #[test]
    fn at_inside_a_word_or_followed_by_whitespace_does_not_open() {
        assert_eq!(trigger("mail@host"), None);
        assert_eq!(trigger("@main.rs "), None);
    }

    #[test]
    fn at_inside_an_existing_token_does_not_reopen() {
        let text = "see @main.rs";
        let token = 4..12;
        assert_eq!(
            detect_trigger(text, text.len(), std::slice::from_ref(&token)),
            None
        );
    }

    #[test]
    fn slash_opens_only_at_the_start() {
        assert_eq!(
            trigger("/com"),
            Some(Trigger {
                kind: TriggerKind::Command,
                range: 0..4,
                query: "com".to_owned(),
            })
        );
        assert_eq!(trigger("run /com"), None);
        assert_eq!(trigger("/compact now"), None);
    }

    #[test]
    fn the_cursor_bounds_the_query() {
        assert_eq!(
            detect_trigger("@main.rs tail", 5, &[]).map(|t| t.query),
            Some("main".to_owned())
        );
    }

    #[test]
    fn multibyte_text_and_non_boundary_cursors_are_safe() {
        assert_eq!(
            trigger("看看 @文件").map(|t| t.query),
            Some("文件".to_owned())
        );
        assert_eq!(detect_trigger("看", 1, &[]), None);
    }

    fn token(range: Range<usize>, id: &str) -> TokenRange {
        TokenRange {
            range,
            id: id.to_owned(),
            label: id.to_owned(),
        }
    }

    fn mention(id: &str) -> PromptSegment {
        PromptSegment::Mention {
            id: id.to_owned(),
            label: id.to_owned(),
        }
    }

    #[test]
    fn segments_interleave_text_and_mentions() {
        let text = "  fix @a and @b  ";
        let segments = prompt_segments(text, &[token(13..15, "b"), token(6..8, "a")]);
        assert_eq!(
            segments,
            [
                PromptSegment::Text("fix ".to_owned()),
                mention("a"),
                PromptSegment::Text(" and ".to_owned()),
                mention("b"),
            ]
        );
    }

    #[test]
    fn adjacent_tokens_have_no_empty_text_between() {
        let segments = prompt_segments("@a@b", &[token(0..2, "a"), token(2..4, "b")]);
        assert_eq!(segments, [mention("a"), mention("b")]);
    }

    #[test]
    fn a_blank_draft_has_no_segments() {
        assert!(prompt_segments("  \n ", &[]).is_empty());
    }

    #[test]
    fn ranking_prefers_prefix_then_substring_then_subsequence() {
        let items = ["src/model.rs", "model.rs", "readme.md", "make.toml"];
        assert_eq!(
            rank("mo", &items, |item| item),
            [&"model.rs", &"src/model.rs", &"make.toml"]
        );
        assert_eq!(
            rank("MD", &items, |item| item),
            [&"readme.md", &"src/model.rs", &"model.rs"]
        );
        assert_eq!(rank("mkt", &items, |item| item), [&"make.toml"]);
        assert_eq!(rank("", &items, |item| item).len(), items.len());
    }
}
