//! Text shaping for tool output: terminal escapes, long-output truncation,
//! search-term highlighting and code fences. Pure, so it is tested without a
//! window.
use std::ops::Range;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnsiColor {
    Black,
    Red,
    Green,
    Yellow,
    Blue,
    Magenta,
    Cyan,
    White,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AnsiStyle {
    pub color: Option<AnsiColor>,
    pub bold: bool,
}

/// Plain text plus the styled byte ranges in it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StyledOutput {
    pub text: String,
    pub styles: Vec<(Range<usize>, AnsiStyle)>,
}

/// Renders raw terminal output as a terminal would show it: a carriage
/// return rewrites its line (progress bars keep only their last frame), SGR
/// colors and bold become styles, and every other escape sequence is dropped.
pub fn parse_ansi(raw: &str) -> StyledOutput {
    let mut output = StyledOutput::default();
    let mut style = AnsiStyle::default();
    let mut run_start = 0;
    for (ix, line) in raw.split('\n').enumerate() {
        if ix > 0 {
            output.text.push('\n');
        }
        let line = line.strip_suffix('\r').unwrap_or(line);
        let line = line.rsplit('\r').next().unwrap_or(line);
        let mut chars = line.chars().peekable();
        while let Some(c) = chars.next() {
            if c != '\x1b' {
                output.text.push(c);
                continue;
            }
            match chars.next() {
                Some('[') => {
                    let mut params = String::new();
                    let mut last = None;
                    for c in chars.by_ref() {
                        if ('\x40'..='\x7e').contains(&c) {
                            last = Some(c);
                            break;
                        }
                        params.push(c);
                    }
                    if last == Some('m') {
                        let next = apply_sgr(style, &params);
                        if next != style {
                            push_run(&mut output, run_start, style);
                            run_start = output.text.len();
                            style = next;
                        }
                    }
                }
                // OSC, like a title or a hyperlink: up to BEL or ESC \.
                Some(']') => {
                    while let Some(c) = chars.next() {
                        if c == '\x07' || (c == '\x1b' && chars.next_if_eq(&'\\').is_some()) {
                            break;
                        }
                    }
                }
                _ => {}
            }
        }
    }
    push_run(&mut output, run_start, style);
    output
}

fn push_run(output: &mut StyledOutput, start: usize, style: AnsiStyle) {
    if style != AnsiStyle::default() && start < output.text.len() {
        output.styles.push((start..output.text.len(), style));
    }
}

fn apply_sgr(mut style: AnsiStyle, params: &str) -> AnsiStyle {
    const COLORS: [AnsiColor; 8] = [
        AnsiColor::Black,
        AnsiColor::Red,
        AnsiColor::Green,
        AnsiColor::Yellow,
        AnsiColor::Blue,
        AnsiColor::Magenta,
        AnsiColor::Cyan,
        AnsiColor::White,
    ];
    let codes = params
        .split(';')
        .map(|code| code.parse::<u16>().unwrap_or(0))
        .collect::<Vec<_>>();
    let mut codes = codes.iter().copied();
    while let Some(code) = codes.next() {
        match code {
            0 => style = AnsiStyle::default(),
            1 => style.bold = true,
            22 => style.bold = false,
            30..=37 => style.color = Some(COLORS[usize::from(code - 30)]),
            90..=97 => style.color = Some(COLORS[usize::from(code - 90)]),
            39 => style.color = None,
            // 256-color and RGB foregrounds carry their own arguments.
            38 => match codes.next() {
                Some(5) => {
                    codes.next();
                }
                Some(2) => {
                    codes.nth(2);
                }
                _ => {}
            },
            _ => {}
        }
    }
    style
}

/// Keeps the last `max_lines` lines, shifting styles to match, and reports
/// how many lines it dropped.
pub fn tail_lines(output: &StyledOutput, max_lines: usize) -> (StyledOutput, usize) {
    let total = output.text.lines().count();
    if total <= max_lines {
        return (output.clone(), 0);
    }
    let hidden = total - max_lines;
    let start = output
        .text
        .match_indices('\n')
        .nth(hidden - 1)
        .map_or(0, |(ix, _)| ix + 1);
    let styles = output
        .styles
        .iter()
        .filter(|(range, _)| range.end > start)
        .map(|(range, style)| (range.start.max(start) - start..range.end - start, *style))
        .collect();
    (
        StyledOutput {
            text: output.text[start..].to_owned(),
            styles,
        },
        hidden,
    )
}

/// The first `max_lines` lines and how many more there are.
pub fn head_lines(text: &str, max_lines: usize) -> (&str, usize) {
    let total = text.lines().count();
    if total <= max_lines {
        return (text, 0);
    }
    let end = text
        .match_indices('\n')
        .nth(max_lines - 1)
        .map_or(text.len(), |(ix, _)| ix);
    (&text[..end], total - max_lines)
}

/// Every ASCII-case-insensitive occurrence of `query`, without overlaps.
pub fn find_matches(text: &str, query: &str) -> Vec<Range<usize>> {
    let mut matches = Vec::new();
    if query.is_empty() {
        return matches;
    }
    let mut ix = 0;
    while ix + query.len() <= text.len() {
        if text.is_char_boundary(ix)
            && text.is_char_boundary(ix + query.len())
            && text[ix..ix + query.len()].eq_ignore_ascii_case(query)
        {
            matches.push(ix..ix + query.len());
            ix += query.len();
        } else {
            ix += 1;
        }
    }
    matches
}

/// The highlighter's language name for a path's extension.
pub fn language_for_path(path: &str) -> Option<&'static str> {
    Some(match path.rsplit_once('.')?.1 {
        "rs" => "rust",
        "ts" | "tsx" => "typescript",
        "js" | "jsx" | "mjs" => "javascript",
        "py" => "python",
        "go" => "go",
        "md" => "markdown",
        "toml" => "toml",
        "json" => "json",
        "yaml" | "yml" => "yaml",
        "sh" | "bash" | "zsh" => "bash",
        "html" => "html",
        "css" => "css",
        "c" | "h" => "c",
        "cpp" | "cc" | "hpp" => "cpp",
        _ => return None,
    })
}

/// The styles that fall within `line`, clipped to it and made relative to
/// its start.
pub fn clip_styles<T: Copy>(
    styles: &[(Range<usize>, T)],
    line: Range<usize>,
) -> Vec<(Range<usize>, T)> {
    styles
        .iter()
        .filter(|(range, _)| range.start < line.end && range.end > line.start)
        .map(|(range, style)| {
            (
                range.start.max(line.start) - line.start..range.end.min(line.end) - line.start,
                *style,
            )
        })
        .collect()
}

/// `docs.rs` and `/reqwest/latest` for a URL; anything unparsable stays
/// whole in the host part.
pub fn split_url(url: &str) -> (&str, &str) {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    match rest.find('/') {
        Some(ix) => (&rest[..ix], &rest[ix..]),
        None => (rest, ""),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn red() -> AnsiStyle {
        AnsiStyle {
            color: Some(AnsiColor::Red),
            bold: false,
        }
    }

    #[test]
    fn sgr_colors_become_styles_and_escapes_disappear() {
        let output = parse_ansi("ok \x1b[31mFAIL\x1b[0m done \x1b[2K");
        assert_eq!(output.text, "ok FAIL done ");
        assert_eq!(output.styles, [(3..7, red())]);
    }

    #[test]
    fn bold_and_bright_colors_combine_and_reset() {
        let output = parse_ansi("\x1b[1;92mPASS\x1b[22m x\x1b[39m.");
        assert_eq!(output.text, "PASS x.");
        let green = Some(AnsiColor::Green);
        assert_eq!(
            output.styles,
            [
                (
                    0..4,
                    AnsiStyle {
                        color: green,
                        bold: true
                    }
                ),
                (
                    4..6,
                    AnsiStyle {
                        color: green,
                        bold: false
                    }
                ),
            ]
        );
    }

    #[test]
    fn extended_colors_do_not_leak_their_arguments() {
        let output = parse_ansi("\x1b[38;5;1ma\x1b[38;2;1;2;3;1mb\x1b[0m");
        assert_eq!(output.text, "ab");
        assert_eq!(
            output.styles,
            [(
                1..2,
                AnsiStyle {
                    color: None,
                    bold: true
                }
            )]
        );
    }

    #[test]
    fn a_carriage_return_rewrites_its_line() {
        let output = parse_ansi("Downloading 10%\rDownloading 100%\r\nDone");
        assert_eq!(output.text, "Downloading 100%\nDone");
    }

    #[test]
    fn osc_sequences_are_dropped() {
        let output = parse_ansi("\x1b]0;title\x07a\x1b]8;;http://x\x1b\\b");
        assert_eq!(output.text, "ab");
    }

    #[test]
    fn a_style_open_across_lines_spans_them() {
        let output = parse_ansi("\x1b[31ma\nb\x1b[0m");
        assert_eq!(output.styles, [(0..3, red())]);
    }

    #[test]
    fn tailing_shifts_and_clips_styles() {
        let output = parse_ansi("one\n\x1b[31mtwo\nthree\x1b[0m\nfour");
        let (tail, hidden) = tail_lines(&output, 2);
        assert_eq!(hidden, 2);
        assert_eq!(tail.text, "three\nfour");
        assert_eq!(tail.styles, [(0..5, red())]);
        assert_eq!(tail_lines(&output, 9).1, 0);
    }

    #[test]
    fn heading_keeps_the_first_lines() {
        assert_eq!(head_lines("a\nb\nc\n", 2), ("a\nb", 1));
        assert_eq!(head_lines("a\nb", 5), ("a\nb", 0));
    }

    #[test]
    fn matches_ignore_ascii_case_and_respect_char_boundaries() {
        assert_eq!(find_matches("Foo foo FOO", "foo"), [0..3, 4..7, 8..11]);
        assert_eq!(find_matches("文件foo", "FOO"), vec![6..9]);
        assert!(find_matches("abc", "").is_empty());
    }

    #[test]
    fn languages_come_from_extensions() {
        assert_eq!(language_for_path("src/a.rs"), Some("rust"));
        assert_eq!(language_for_path("Makefile"), None);
        assert_eq!(language_for_path("a.unknown"), None);
    }

    #[test]
    fn styles_clip_to_a_line_and_shift_to_its_start() {
        let styles = [(0..4, 'a'), (6..12, 'b'), (14..15, 'c')];
        assert_eq!(clip_styles(&styles, 5..10), [(1..5, 'b')]);
        assert_eq!(clip_styles(&styles, 2..8), [(0..2, 'a'), (4..6, 'b')]);
    }

    #[test]
    fn urls_split_into_host_and_path() {
        assert_eq!(
            split_url("https://docs.rs/reqwest/latest"),
            ("docs.rs", "/reqwest/latest")
        );
        assert_eq!(split_url("example.com"), ("example.com", ""));
    }
}
