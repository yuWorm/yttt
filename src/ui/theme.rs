use gpui::{Edges, Pixels, Rgba, px, rgb};
use gpui_component::{
    ThemeConfig, ThemeConfigColors, ThemeMode,
    highlighter::{HighlightThemeStyle, SyntaxColors, ThemeStyle},
};
use yttt_terminal::{ColorPalette, TerminalConfig};

use crate::config::{
    settings::{AppSettings, TerminalSettings},
    theme::ThemeStore,
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WorkbenchTheme {
    pub app_background: Rgba,
    pub surface: Rgba,
    pub surface_elevated: Rgba,
    pub titlebar_background: Rgba,
    pub sidebar_background: Rgba,
    pub tabbar_background: Rgba,
    pub terminal_background: Rgba,
    pub border: Rgba,
    pub border_strong: Rgba,
    pub split_line: Rgba,
    pub split_line_active: Rgba,
    pub text: Rgba,
    pub text_muted: Rgba,
    pub text_subtle: Rgba,
    pub accent: Rgba,
    pub active_surface: Rgba,
    pub hover_surface: Rgba,
    pub danger: Rgba,
    pub success: Rgba,
    pub warning: Rgba,
    pub focus_ring: Rgba,
    pub focused_pane_border: Rgba,
    pub split_line_width: Pixels,
    pub split_hit_area_width: Pixels,
}

impl WorkbenchTheme {
    pub fn dark() -> Self {
        Self {
            app_background: rgb(0x1f2329),
            surface: rgb(0x23272e),
            surface_elevated: rgb(0x23272e),
            titlebar_background: rgb(0x1f2329),
            sidebar_background: rgb(0x1f2329),
            tabbar_background: rgb(0x1f2329),
            terminal_background: rgb(0x1b1e23),
            border: rgb(0x343a43),
            border_strong: rgb(0x414852),
            split_line: rgb(0x343a43),
            split_line_active: rgb(0x414852),
            text: rgb(0xe6e8eb),
            text_muted: rgb(0xb8bcc2),
            text_subtle: rgb(0x7d8794),
            accent: rgb(0x7aa2f7),
            active_surface: rgb(0x2e333b),
            hover_surface: rgb(0x292e36),
            danger: rgb(0xef4444),
            success: rgb(0x22c55e),
            warning: rgb(0xf59e0b),
            focus_ring: rgb(0x7aa2f7),
            focused_pane_border: rgb(0x6f7785),
            split_line_width: px(1.0),
            split_hit_area_width: px(7.0),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct AppTheme {
    pub name: String,
    pub mode: ThemeMode,
    pub ui: WorkbenchTheme,
    pub editor: EditorTheme,
    pub terminal: TerminalTheme,
}

impl AppTheme {
    pub fn builtin_dark() -> Self {
        let ui = WorkbenchTheme::dark();
        Self {
            name: "yttt-dark".to_string(),
            mode: ThemeMode::Dark,
            editor: EditorTheme::from_workbench_theme(ui),
            terminal: TerminalTheme::from_workbench_theme(ui),
            ui,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ThemeRuntime {
    pub theme_name: String,
    pub mode: ThemeMode,
    pub ui: WorkbenchTheme,
    pub editor: EditorTheme,
    pub terminal: TerminalTheme,
    pub terminal_settings: TerminalSettings,
}

impl ThemeRuntime {
    pub fn resolve(settings: &AppSettings, store: &ThemeStore) -> Self {
        let selected = store
            .theme(&settings.theme.name)
            .or_else(|| store.theme("yttt-dark"))
            .cloned()
            .unwrap_or_else(AppTheme::builtin_dark);
        let terminal = settings
            .theme
            .terminal
            .as_deref()
            .and_then(|terminal_theme| store.theme(terminal_theme))
            .map(|theme| theme.terminal.clone())
            .unwrap_or_else(|| selected.terminal.clone());

        Self {
            theme_name: selected.name,
            mode: selected.mode,
            ui: selected.ui,
            editor: selected.editor,
            terminal,
            terminal_settings: settings.terminal.clone(),
        }
    }

    pub fn to_gpui_component_theme_config(&self) -> ThemeConfig {
        let theme = self.ui;
        let mut colors = ThemeConfigColors::default();
        colors.background = Some(color_hex(theme.app_background).into());
        colors.foreground = Some(color_hex(theme.text).into());
        colors.border = Some(color_hex(theme.border).into());
        colors.input = Some(color_hex(theme.border).into());
        colors.muted = Some(color_hex(theme.surface).into());
        colors.muted_foreground = Some(color_hex(theme.text_subtle).into());
        colors.primary = Some(color_hex(theme.active_surface).into());
        colors.primary_foreground = Some(color_hex(theme.text).into());
        colors.primary_hover = Some(color_hex(theme.hover_surface).into());
        colors.primary_active = Some(color_hex(theme.active_surface).into());
        colors.secondary = Some(color_hex(theme.surface_elevated).into());
        colors.secondary_foreground = Some(color_hex(theme.text_muted).into());
        colors.secondary_hover = Some(color_hex(theme.hover_surface).into());
        colors.secondary_active = Some(color_hex(theme.active_surface).into());
        colors.switch = Some(color_hex(theme.surface_elevated).into());
        colors.switch_thumb = Some(color_hex(theme.text).into());
        colors.accent = Some(color_hex(theme.hover_surface).into());
        colors.caret = Some(color_hex(theme.accent).into());
        colors.list = Some(color_hex(theme.surface_elevated).into());
        colors.list_active = Some(color_hex(theme.active_surface).into());
        colors.list_active_border = Some(color_hex(theme.active_surface).into());
        colors.list_hover = Some(color_hex(theme.hover_surface).into());
        colors.popover = Some(color_hex(theme.surface).into());
        colors.popover_foreground = Some(color_hex(theme.text).into());
        colors.selection = Some(color_hex(theme.active_surface).into());
        colors.sidebar = Some(color_hex(theme.sidebar_background).into());
        colors.sidebar_foreground = Some(color_hex(theme.text_muted).into());
        colors.sidebar_primary = Some(color_hex(theme.active_surface).into());
        colors.sidebar_primary_foreground = Some(color_hex(theme.text).into());
        colors.success = Some(color_hex(theme.success).into());
        colors.success_foreground = Some(color_hex(theme.text).into());
        colors.warning = Some(color_hex(theme.warning).into());
        colors.warning_foreground = Some(color_hex(theme.text).into());
        colors.danger = Some(color_hex(theme.danger).into());
        colors.danger_foreground = Some(color_hex(theme.text).into());
        colors.title_bar = Some(color_hex(theme.titlebar_background).into());
        colors.title_bar_border = Some(color_hex(theme.border).into());
        colors.window_border = Some(color_hex(theme.border).into());

        let mut config = ThemeConfig::default();
        config.name = self.theme_name.clone().into();
        config.mode = self.mode;
        config.radius = Some(6);
        config.radius_lg = Some(8);
        config.shadow = Some(false);
        config.colors = colors;
        config.highlight = Some(self.editor.to_highlight_theme_style());
        config
    }

    pub fn to_terminal_config(&self) -> TerminalConfig {
        let default_config = TerminalConfig::default();
        let font_family = if self.terminal_settings.font_family.trim().is_empty() {
            default_config.font_family
        } else {
            self.terminal_settings.font_family.clone()
        };

        TerminalConfig {
            cols: 80,
            rows: 24,
            font_family,
            font_size: px(self.terminal_settings.font_size),
            scrollback: self.terminal_settings.scrollback,
            line_height_multiplier: self.terminal_settings.line_height,
            padding: Edges::all(px(self.terminal_settings.padding)),
            show_scrollbar: self.terminal_settings.show_scrollbar,
            colors: self.terminal.to_color_palette(),
        }
    }
}

impl Default for ThemeRuntime {
    fn default() -> Self {
        Self::resolve(&AppSettings::default(), &ThemeStore::builtin())
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EditorTheme {
    pub background: Rgba,
    pub foreground: Rgba,
    pub active_line: Rgba,
    pub line_number: Rgba,
    pub active_line_number: Rgba,
    pub syntax: EditorSyntaxTheme,
}

impl EditorTheme {
    pub fn from_workbench_theme(theme: WorkbenchTheme) -> Self {
        Self {
            background: theme.terminal_background,
            foreground: theme.text,
            active_line: theme.surface,
            line_number: theme.text_subtle,
            active_line_number: theme.text_muted,
            syntax: EditorSyntaxTheme::dark(),
        }
    }

    pub fn to_highlight_theme_style(self) -> HighlightThemeStyle {
        let mut syntax = SyntaxColors::default();
        syntax.boolean = Some(theme_style(self.syntax.boolean));
        syntax.comment = Some(theme_style(self.syntax.comment));
        syntax.comment_doc = Some(theme_style(self.syntax.comment_doc));
        syntax.constant = Some(theme_style(self.syntax.constant));
        syntax.constructor = Some(theme_style(self.syntax.constructor));
        syntax.function = Some(theme_style(self.syntax.function));
        syntax.keyword = Some(theme_style(self.syntax.keyword));
        syntax.number = Some(theme_style(self.syntax.number));
        syntax.operator = Some(theme_style(self.syntax.operator));
        syntax.property = Some(theme_style(self.syntax.property));
        syntax.punctuation = Some(theme_style(self.syntax.punctuation));
        syntax.punctuation_bracket = Some(theme_style(self.syntax.punctuation));
        syntax.punctuation_delimiter = Some(theme_style(self.syntax.punctuation));
        syntax.string = Some(theme_style(self.syntax.string));
        syntax.string_escape = Some(theme_style(self.syntax.string_escape));
        syntax.type_ = Some(theme_style(self.syntax.type_));
        syntax.variable = Some(theme_style(self.syntax.variable));
        syntax.variable_special = Some(theme_style(self.syntax.variable_special));

        HighlightThemeStyle {
            editor_background: Some(self.background.into()),
            editor_foreground: Some(self.foreground.into()),
            editor_active_line: Some(self.active_line.into()),
            editor_line_number: Some(self.line_number.into()),
            editor_active_line_number: Some(self.active_line_number.into()),
            status: Default::default(),
            syntax,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EditorSyntaxTheme {
    pub boolean: Rgba,
    pub comment: Rgba,
    pub comment_doc: Rgba,
    pub constant: Rgba,
    pub constructor: Rgba,
    pub function: Rgba,
    pub keyword: Rgba,
    pub number: Rgba,
    pub operator: Rgba,
    pub property: Rgba,
    pub punctuation: Rgba,
    pub string: Rgba,
    pub string_escape: Rgba,
    pub type_: Rgba,
    pub variable: Rgba,
    pub variable_special: Rgba,
}

impl EditorSyntaxTheme {
    pub fn dark() -> Self {
        Self {
            boolean: rgb(0xff9e64),
            comment: rgb(0x697386),
            comment_doc: rgb(0x7d8794),
            constant: rgb(0xffcb6b),
            constructor: rgb(0xffcb6b),
            function: rgb(0x82aaff),
            keyword: rgb(0xc792ea),
            number: rgb(0xf78c6c),
            operator: rgb(0x89ddff),
            property: rgb(0x82aaff),
            punctuation: rgb(0x89ddff),
            string: rgb(0xecc48d),
            string_escape: rgb(0x89ddff),
            type_: rgb(0xffcb6b),
            variable: rgb(0xe6e8eb),
            variable_special: rgb(0xffcb6b),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TerminalTheme {
    pub background: Rgba,
    pub foreground: Rgba,
    pub cursor: Option<Rgba>,
    pub selection_background: Option<Rgba>,
    pub normal: AnsiColors,
    pub bright: AnsiColors,
    pub indexed_colors: Vec<IndexedColor>,
}

impl TerminalTheme {
    pub fn from_workbench_theme(theme: WorkbenchTheme) -> Self {
        Self {
            background: theme.terminal_background,
            foreground: theme.text,
            cursor: Some(theme.text),
            selection_background: Some(theme.active_surface),
            normal: AnsiColors::default_normal(),
            bright: AnsiColors::default_bright(),
            indexed_colors: Vec::new(),
        }
    }

    pub fn to_color_palette(&self) -> ColorPalette {
        let (background_r, background_g, background_b) = color_bytes(self.background);
        let (foreground_r, foreground_g, foreground_b) = color_bytes(self.foreground);
        let cursor = self.cursor.unwrap_or(self.foreground);
        let (cursor_r, cursor_g, cursor_b) = color_bytes(cursor);
        let selection_background = self.selection_background.unwrap_or(self.background);
        let (selection_r, selection_g, selection_b) = color_bytes(selection_background);

        let mut builder = ColorPalette::builder()
            .background(background_r, background_g, background_b)
            .foreground(foreground_r, foreground_g, foreground_b)
            .cursor(cursor_r, cursor_g, cursor_b)
            .selection_background(selection_r, selection_g, selection_b);

        let (r, g, b) = color_bytes(self.normal.black);
        builder = builder.black(r, g, b);
        let (r, g, b) = color_bytes(self.normal.red);
        builder = builder.red(r, g, b);
        let (r, g, b) = color_bytes(self.normal.green);
        builder = builder.green(r, g, b);
        let (r, g, b) = color_bytes(self.normal.yellow);
        builder = builder.yellow(r, g, b);
        let (r, g, b) = color_bytes(self.normal.blue);
        builder = builder.blue(r, g, b);
        let (r, g, b) = color_bytes(self.normal.magenta);
        builder = builder.magenta(r, g, b);
        let (r, g, b) = color_bytes(self.normal.cyan);
        builder = builder.cyan(r, g, b);
        let (r, g, b) = color_bytes(self.normal.white);
        builder = builder.white(r, g, b);

        let (r, g, b) = color_bytes(self.bright.black);
        builder = builder.bright_black(r, g, b);
        let (r, g, b) = color_bytes(self.bright.red);
        builder = builder.bright_red(r, g, b);
        let (r, g, b) = color_bytes(self.bright.green);
        builder = builder.bright_green(r, g, b);
        let (r, g, b) = color_bytes(self.bright.yellow);
        builder = builder.bright_yellow(r, g, b);
        let (r, g, b) = color_bytes(self.bright.blue);
        builder = builder.bright_blue(r, g, b);
        let (r, g, b) = color_bytes(self.bright.magenta);
        builder = builder.bright_magenta(r, g, b);
        let (r, g, b) = color_bytes(self.bright.cyan);
        builder = builder.bright_cyan(r, g, b);
        let (r, g, b) = color_bytes(self.bright.white);
        builder = builder.bright_white(r, g, b);

        builder.build()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AnsiColors {
    pub black: Rgba,
    pub red: Rgba,
    pub green: Rgba,
    pub yellow: Rgba,
    pub blue: Rgba,
    pub magenta: Rgba,
    pub cyan: Rgba,
    pub white: Rgba,
}

impl AnsiColors {
    pub fn default_normal() -> Self {
        Self {
            black: rgb(0x000000),
            red: rgb(0xcc0000),
            green: rgb(0x4e9a06),
            yellow: rgb(0xc4a000),
            blue: rgb(0x3465a4),
            magenta: rgb(0x75507b),
            cyan: rgb(0x06989a),
            white: rgb(0xd3d7cf),
        }
    }

    pub fn default_bright() -> Self {
        Self {
            black: rgb(0x555753),
            red: rgb(0xef2929),
            green: rgb(0x8ae234),
            yellow: rgb(0xfce94f),
            blue: rgb(0x729fcf),
            magenta: rgb(0xad7fa8),
            cyan: rgb(0x34e2e2),
            white: rgb(0xeeeeec),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct IndexedColor {
    pub index: u8,
    pub color: Rgba,
}

fn color_hex(color: Rgba) -> String {
    let r = (color.r.clamp(0.0, 1.0) * 255.0).round() as u8;
    let g = (color.g.clamp(0.0, 1.0) * 255.0).round() as u8;
    let b = (color.b.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#{r:02x}{g:02x}{b:02x}")
}

fn color_bytes(color: Rgba) -> (u8, u8, u8) {
    (
        (color.r.clamp(0.0, 1.0) * 255.0).round() as u8,
        (color.g.clamp(0.0, 1.0) * 255.0).round() as u8,
        (color.b.clamp(0.0, 1.0) * 255.0).round() as u8,
    )
}

fn theme_style(color: Rgba) -> ThemeStyle {
    toml::from_str(&format!("color = \"{}\"", color_hex(color)))
        .expect("valid editor syntax theme style")
}
