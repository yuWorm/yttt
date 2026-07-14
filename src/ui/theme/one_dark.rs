use gpui::rgb;
use gpui_component::ThemeMode;

use super::{
    AnsiColors, AppTheme, DEFAULT_THEME_NAME, EditorSyntaxTheme, EditorTheme, TerminalTheme,
    ThemeMetadata, WorkbenchTheme,
};

pub(super) fn theme() -> AppTheme {
    AppTheme {
        name: DEFAULT_THEME_NAME.to_string(),
        mode: ThemeMode::Dark,
        metadata: ThemeMetadata::default(),
        ui: WorkbenchTheme::one_dark(),
        editor: EditorTheme {
            background: rgb(0x23272e),
            foreground: rgb(0xabb2bf),
            active_line: rgb(0x2c313c),
            line_number: rgb(0x495162),
            active_line_number: rgb(0xabb2bf),
            syntax: EditorSyntaxTheme {
                boolean: rgb(0xd19a66),
                comment: rgb(0x7f838c),
                comment_doc: rgb(0x7f848e),
                constant: rgb(0xd19a66),
                constructor: rgb(0xe06c75),
                function: rgb(0x61afef),
                keyword: rgb(0xc678dd),
                number: rgb(0xd19a66),
                operator: rgb(0xabb2bf),
                property: rgb(0xe06c75),
                punctuation: rgb(0xabb2bf),
                string: rgb(0x98c379),
                string_escape: rgb(0x56b6c2),
                type_: rgb(0xe5c07b),
                variable: rgb(0xe06c75),
                variable_special: rgb(0xe5c07b),
            },
        },
        terminal: TerminalTheme {
            background: rgb(0x23272e),
            foreground: rgb(0xabb2bf),
            cursor: Some(rgb(0xabb2bf)),
            // Zed's #67769640 selection composited over the RGB-only terminal background.
            selection_background: Some(rgb(0x343b48)),
            selection_foreground: None,
            cursor_text: None,
            search_foreground: rgb(0x181818),
            search_background: rgb(0xac4242),
            focused_search_foreground: rgb(0x181818),
            focused_search_background: rgb(0xf4bf75),
            hint_start_foreground: rgb(0x181818),
            hint_start_background: rgb(0xf4bf75),
            hint_end_foreground: rgb(0x181818),
            hint_end_background: rgb(0xac4242),
            normal: AnsiColors {
                black: rgb(0x3f4451),
                red: rgb(0xe05561),
                green: rgb(0x8cc265),
                yellow: rgb(0xd18f52),
                blue: rgb(0x4aa5f0),
                magenta: rgb(0xc162de),
                cyan: rgb(0x42b3c2),
                white: rgb(0xd7dae0),
            },
            bright: AnsiColors {
                black: rgb(0x4f5666),
                red: rgb(0xff616e),
                green: rgb(0xa5e075),
                yellow: rgb(0xf0a45d),
                blue: rgb(0x4dc4ff),
                magenta: rgb(0xde73ff),
                cyan: rgb(0x4cd1e0),
                white: rgb(0xe6e6e6),
            },
            indexed_colors: Vec::new(),
        },
    }
}
