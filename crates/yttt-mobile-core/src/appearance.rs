//! The color themes both apps offer for the terminal and the editor, kept
//! here so the iOS and Android tables cannot drift apart.
//!
//! Colors are `0xRRGGBB`. "Follow the system" is a choice, not a palette: it
//! resolves to the dark or the light theme.

/// The choice a fresh install starts with.
pub const DEFAULT_THEME: &str = "system";

/// Colors for syntax highlighting in the editor.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Record)]
pub struct SyntaxColors {
    pub comment: u32,
    pub keyword: u32,
    pub string: u32,
    pub number: u32,
    pub function: u32,
    pub type_name: u32,
    pub constant: u32,
}

/// One palette: the terminal's 16 ANSI colors, its defaults, and the editor's.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct Theme {
    pub id: String,
    pub name: String,
    pub dark: bool,
    pub background: u32,
    pub foreground: u32,
    pub cursor: u32,
    pub selection: u32,
    /// Black, red, green, yellow, blue, magenta, cyan, white, then the bright ones.
    pub ansi: Vec<u32>,
    pub syntax: SyntaxColors,
}

/// An entry in the theme picker.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct ThemeChoice {
    pub id: String,
    pub name: String,
}

struct Palette {
    id: &'static str,
    name: &'static str,
    dark: bool,
    background: u32,
    foreground: u32,
    cursor: u32,
    selection: u32,
    ansi: [u32; 16],
    syntax: SyntaxColors,
}

const SOLARIZED_ANSI: [u32; 16] = [
    0x073642, 0xDC322F, 0x859900, 0xB58900, 0x268BD2, 0xD33682, 0x2AA198, 0xEEE8D5, 0x002B36,
    0xCB4B16, 0x586E75, 0x657B83, 0x839496, 0x6C71C4, 0x93A1A1, 0xFDF6E3,
];

const PALETTES: [Palette; 6] = [
    // Close to what the apps showed before themes: VS Code's dark terminal
    // colors on black, and Dark+ for code.
    Palette {
        id: "dark",
        name: "暗色",
        dark: true,
        background: 0x000000,
        foreground: 0xE5E5E5,
        cursor: 0xFFFFFF,
        selection: 0x264F78,
        ansi: [
            0x000000, 0xCD3131, 0x0DBC79, 0xE5E510, 0x2472C8, 0xBC3FBC, 0x11A8CD, 0xE5E5E5,
            0x666666, 0xF14C4C, 0x23D18B, 0xF5F543, 0x3B8EEA, 0xD670D6, 0x29B8DB, 0xFFFFFF,
        ],
        syntax: SyntaxColors {
            comment: 0x6A9955,
            keyword: 0xC586C0,
            string: 0xCE9178,
            number: 0xB5CEA8,
            function: 0xDCDCAA,
            type_name: 0x4EC9B0,
            constant: 0x4FC1FF,
        },
    },
    Palette {
        id: "light",
        name: "亮色",
        dark: false,
        background: 0xFFFFFF,
        foreground: 0x333333,
        cursor: 0x000000,
        selection: 0xADD6FF,
        ansi: [
            0x000000, 0xCD3131, 0x00A000, 0x949800, 0x0451A5, 0xBC05BC, 0x0598BC, 0x555555,
            0x666666, 0xCD3131, 0x14A014, 0x8A8E00, 0x0451A5, 0xBC05BC, 0x0598BC, 0xA5A5A5,
        ],
        syntax: SyntaxColors {
            comment: 0x008000,
            keyword: 0xAF00DB,
            string: 0xA31515,
            number: 0x098658,
            function: 0x795E26,
            type_name: 0x267F99,
            constant: 0x0070C1,
        },
    },
    Palette {
        id: "solarized-dark",
        name: "Solarized 暗",
        dark: true,
        background: 0x002B36,
        foreground: 0x93A1A1,
        cursor: 0x93A1A1,
        selection: 0x274642,
        ansi: SOLARIZED_ANSI,
        syntax: SyntaxColors {
            comment: 0x657B83,
            keyword: 0x859900,
            string: 0x2AA198,
            number: 0xD33682,
            function: 0x268BD2,
            type_name: 0xB58900,
            constant: 0xCB4B16,
        },
    },
    Palette {
        id: "solarized-light",
        name: "Solarized 亮",
        dark: false,
        background: 0xFDF6E3,
        foreground: 0x586E75,
        cursor: 0x586E75,
        selection: 0xEEE8D5,
        ansi: SOLARIZED_ANSI,
        // Solarized's accents, a little darker: on its light background the
        // originals are about 3:1, faint on a phone in daylight.
        syntax: SyntaxColors {
            comment: 0x839496,
            keyword: 0x6F8500,
            string: 0x1F857E,
            number: 0xD33682,
            function: 0x268BD2,
            type_name: 0x977300,
            constant: 0xCB4B16,
        },
    },
    Palette {
        id: "one-dark",
        name: "One Dark",
        dark: true,
        background: 0x282C34,
        foreground: 0xABB2BF,
        cursor: 0x528BFF,
        selection: 0x3E4451,
        ansi: [
            0x282C34, 0xE06C75, 0x98C379, 0xE5C07B, 0x61AFEF, 0xC678DD, 0x56B6C2, 0xABB2BF,
            0x5C6370, 0xE06C75, 0x98C379, 0xE5C07B, 0x61AFEF, 0xC678DD, 0x56B6C2, 0xFFFFFF,
        ],
        syntax: SyntaxColors {
            comment: 0x7F848E,
            keyword: 0xC678DD,
            string: 0x98C379,
            number: 0xD19A66,
            function: 0x61AFEF,
            type_name: 0xE5C07B,
            constant: 0x56B6C2,
        },
    },
    Palette {
        id: "nord",
        name: "Nord",
        dark: true,
        background: 0x2E3440,
        foreground: 0xD8DEE9,
        cursor: 0xD8DEE9,
        selection: 0x434C5E,
        ansi: [
            0x3B4252, 0xBF616A, 0xA3BE8C, 0xEBCB8B, 0x81A1C1, 0xB48EAD, 0x88C0D0, 0xE5E9F0,
            0x4C566A, 0xBF616A, 0xA3BE8C, 0xEBCB8B, 0x81A1C1, 0xB48EAD, 0x8FBCBB, 0xECEFF4,
        ],
        syntax: SyntaxColors {
            comment: 0x7B88A1,
            keyword: 0x81A1C1,
            string: 0xA3BE8C,
            number: 0xB48EAD,
            function: 0x88C0D0,
            type_name: 0x8FBCBB,
            constant: 0xEBCB8B,
        },
    },
];

impl Palette {
    fn theme(&self) -> Theme {
        Theme {
            id: self.id.into(),
            name: self.name.into(),
            dark: self.dark,
            background: self.background,
            foreground: self.foreground,
            cursor: self.cursor,
            selection: self.selection,
            ansi: self.ansi.to_vec(),
            syntax: self.syntax,
        }
    }
}

/// The picker's entries, "follow the system" first.
#[uniffi::export]
pub fn theme_choices() -> Vec<ThemeChoice> {
    std::iter::once(ThemeChoice {
        id: DEFAULT_THEME.into(),
        name: "跟随系统".into(),
    })
    .chain(PALETTES.iter().map(|palette| ThemeChoice {
        id: palette.id.into(),
        name: palette.name.into(),
    }))
    .collect()
}

/// The palette for a choice; "follow the system", or an id this build does
/// not know (a setting from a later version), picks dark or light.
#[uniffi::export]
pub fn theme(id: String, system_dark: bool) -> Theme {
    let fallback = if system_dark { "dark" } else { "light" };
    PALETTES
        .iter()
        .find(|palette| palette.id == id)
        .or_else(|| PALETTES.iter().find(|palette| palette.id == fallback))
        .expect("the dark and light palettes exist")
        .theme()
}

/// The language the editor highlights a file as, from its name: one id both
/// apps map to their grammars, or `None` for plain text.
#[uniffi::export]
pub fn editor_language(path: String) -> Option<String> {
    let name = path
        .rsplit('/')
        .next()
        .unwrap_or(&path)
        .to_ascii_lowercase();
    let by_name = match name.as_str() {
        "dockerfile" | "containerfile" => Some("dockerfile"),
        "makefile" | "gnumakefile" => Some("makefile"),
        ".bashrc" | ".zshrc" | ".profile" | ".bash_profile" | ".zprofile" => Some("shell"),
        "cargo.lock" | "poetry.lock" => Some("toml"),
        _ => None,
    };
    if let Some(language) = by_name {
        return Some(language.into());
    }
    let extension = name.rsplit_once('.')?.1;
    let language = match extension {
        "rs" => "rust",
        "js" | "mjs" | "cjs" | "jsx" => "javascript",
        "ts" | "mts" | "cts" => "typescript",
        "tsx" => "tsx",
        "json" | "jsonc" | "json5" => "json",
        "py" | "pyi" => "python",
        "go" => "go",
        "swift" => "swift",
        "kt" | "kts" => "kotlin",
        "java" => "java",
        "c" | "h" => "c",
        "cc" | "cpp" | "cxx" | "hpp" | "hh" | "hxx" | "m" | "mm" => "cpp",
        "cs" => "csharp",
        "rb" => "ruby",
        "php" => "php",
        "lua" => "lua",
        "sh" | "bash" | "zsh" | "fish" => "shell",
        "md" | "markdown" => "markdown",
        "yaml" | "yml" => "yaml",
        "toml" => "toml",
        "html" | "htm" | "vue" | "svelte" => "html",
        "css" | "scss" | "less" => "css",
        "xml" | "plist" | "svg" | "xib" | "storyboard" => "xml",
        "sql" => "sql",
        _ => return None,
    };
    Some(language.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// WCAG relative luminance and contrast ratio.
    fn luminance(color: u32) -> f64 {
        let channel = |shift: u32| {
            let value = f64::from((color >> shift) & 0xFF) / 255.0;
            if value <= 0.03928 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(16) + 0.7152 * channel(8) + 0.0722 * channel(0)
    }

    fn contrast(a: u32, b: u32) -> f64 {
        let (light, dark) = {
            let (a, b) = (luminance(a), luminance(b));
            if a > b { (a, b) } else { (b, a) }
        };
        (light + 0.05) / (dark + 0.05)
    }

    #[test]
    fn files_are_highlighted_by_their_name() {
        let language = |path: &str| editor_language(path.into());
        assert_eq!(language("src/main.rs").as_deref(), Some("rust"));
        assert_eq!(language("web/App.TSX").as_deref(), Some("tsx"));
        assert_eq!(language("a/b/Dockerfile").as_deref(), Some("dockerfile"));
        assert_eq!(language("Cargo.lock").as_deref(), Some("toml"));
        assert_eq!(language("~/.zshrc").as_deref(), Some("shell"));
        assert_eq!(language("notes.txt"), None);
        assert_eq!(language("LICENSE"), None);
        assert_eq!(language("archive.tar.gz"), None);
    }

    #[test]
    fn every_choice_is_unique_and_resolves() {
        let choices = theme_choices();
        assert_eq!(choices[0].id, DEFAULT_THEME);
        let mut ids: Vec<_> = choices.iter().map(|choice| choice.id.clone()).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), choices.len());
        for choice in &choices[1..] {
            assert_eq!(theme(choice.id.clone(), true).id, choice.id);
            assert_eq!(theme(choice.id.clone(), false).id, choice.id);
        }
    }

    #[test]
    fn following_the_system_and_unknown_ids_pick_dark_or_light() {
        assert_eq!(theme(DEFAULT_THEME.into(), true).id, "dark");
        assert_eq!(theme(DEFAULT_THEME.into(), false).id, "light");
        assert_eq!(theme("from-a-later-version".into(), true).id, "dark");
        assert_eq!(theme("from-a-later-version".into(), false).id, "light");
    }

    #[test]
    fn every_palette_is_complete_and_readable() {
        for palette in &PALETTES {
            let theme = palette.theme();
            assert_eq!(theme.ansi.len(), 16, "{}", theme.id);
            assert!(
                theme
                    .ansi
                    .iter()
                    .chain([&theme.background, &theme.foreground])
                    .all(|color| *color <= 0xFF_FFFF)
            );
            assert_eq!(
                theme.dark,
                luminance(theme.background) < 0.2,
                "{}",
                theme.id
            );
            // Text and code must read on the background; comments may be quieter.
            assert!(
                contrast(theme.foreground, theme.background) >= 4.5,
                "{} text",
                theme.id
            );
            let syntax = theme.syntax;
            for (role, color) in [
                ("keyword", syntax.keyword),
                ("string", syntax.string),
                ("function", syntax.function),
                ("type", syntax.type_name),
                ("constant", syntax.constant),
                ("number", syntax.number),
            ] {
                assert!(
                    contrast(color, theme.background) >= 3.0,
                    "{} {role}",
                    theme.id
                );
            }
            assert!(
                contrast(syntax.comment, theme.background) >= 2.5,
                "{} comment",
                theme.id
            );
            assert!(
                contrast(theme.cursor, theme.background) >= 3.0,
                "{} cursor",
                theme.id
            );
        }
    }
}
