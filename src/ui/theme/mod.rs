pub mod icons;
mod one_dark;
pub mod zed;

use gpui::{Edges, Rgba, px};
use gpui_component::{
    ThemeConfig, ThemeConfigColors, ThemeMode,
    highlighter::{HighlightThemeStyle, SyntaxColors, ThemeStyle},
};
use yttt_terminal::{ColorPalette, TerminalConfig};
pub use yttt_ui::theme::WorkbenchTheme;

use crate::config::{
    settings::{AppSettings, TerminalSettings},
    theme::ThemeStore,
};

pub const DEFAULT_THEME_NAME: &str = "one-dark-theme";

#[derive(Clone, Debug, Default, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(default)]
pub struct ThemeMetadata {
    pub authors: Vec<String>,
    pub description: Option<String>,
    pub repository: Option<String>,
    pub converted_from: Option<String>,
    pub source: Option<ThemeSourceMetadata>,
}

#[derive(Clone, Debug, Default, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(default)]
pub struct ThemeSourceMetadata {
    pub format: String,
    pub extension_id: Option<String>,
    pub extension_name: Option<String>,
    pub extension_version: Option<String>,
    pub theme_file: Option<String>,
    pub family_name: Option<String>,
    pub family_author: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AppTheme {
    pub name: String,
    pub mode: ThemeMode,
    pub metadata: ThemeMetadata,
    pub ui: WorkbenchTheme,
    pub editor: EditorTheme,
    pub terminal: TerminalTheme,
}

impl AppTheme {
    pub fn one_dark() -> Self {
        one_dark::theme()
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

fn cap_window_surface_opacity(color: &mut Rgba, opacity: f32) {
    color.a = color.a.min(opacity);
}

fn apply_window_surface_opacity(
    ui: &mut WorkbenchTheme,
    editor: &mut EditorTheme,
    terminal: &mut TerminalTheme,
    opacity: f32,
) {
    cap_window_surface_opacity(&mut ui.app_background, opacity);
    cap_window_surface_opacity(&mut ui.surface, opacity);
    cap_window_surface_opacity(&mut ui.surface_elevated, opacity);
    cap_window_surface_opacity(&mut ui.titlebar_background, opacity);
    cap_window_surface_opacity(&mut ui.sidebar_background, opacity);
    cap_window_surface_opacity(&mut ui.tabbar_background, opacity);
    cap_window_surface_opacity(&mut ui.terminal_background, opacity);
    cap_window_surface_opacity(&mut ui.active_surface, opacity);
    cap_window_surface_opacity(&mut ui.hover_surface, opacity);
    cap_window_surface_opacity(&mut editor.background, opacity);
    cap_window_surface_opacity(&mut editor.active_line, opacity);
    cap_window_surface_opacity(&mut terminal.background, opacity);
}

impl ThemeRuntime {
    pub fn resolve(settings: &AppSettings, store: &ThemeStore) -> Self {
        let selected = store
            .theme(&settings.theme.name)
            .or_else(|| store.theme(DEFAULT_THEME_NAME))
            .cloned()
            .unwrap_or_else(AppTheme::one_dark);
        let mut terminal = settings
            .theme
            .terminal
            .as_deref()
            .and_then(|terminal_theme| store.theme(terminal_theme))
            .map(|theme| theme.terminal.clone())
            .unwrap_or_else(|| selected.terminal.clone());
        let mut ui = selected.ui;
        let mut editor = selected.editor;
        apply_window_surface_opacity(
            &mut ui,
            &mut editor,
            &mut terminal,
            settings.window.resolved_opacity(),
        );

        Self {
            theme_name: selected.name,
            mode: selected.mode,
            ui,
            editor,
            terminal,
            terminal_settings: settings.terminal.clone(),
        }
    }

    pub fn to_gpui_component_theme_config(&self) -> ThemeConfig {
        let theme = self.ui;
        let mut colors = ThemeConfigColors::default();
        colors.background = Some(color_hex(theme.app_background.alpha(1.0)).into());
        colors.foreground = Some(color_hex(theme.text).into());
        colors.border = Some(color_hex(theme.border).into());
        colors.input = Some(color_hex(theme.border).into());
        colors.ring = Some(color_hex(theme.focus_ring).into());
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
        colors.selection = Some(color_hex(theme.selection).into());
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
            cursor_shape: self.terminal_settings.cursor_shape,
            cursor_blinking: self.terminal_settings.cursor_blinking,
            cursor_blink_interval_ms: self.terminal_settings.cursor_blink_interval_ms,
            cursor_blink_timeout_secs: self.terminal_settings.cursor_blink_timeout_secs as u8,
            cursor_unfocused_hollow: self.terminal_settings.cursor_unfocused_hollow,
            cursor_thickness: self.terminal_settings.cursor_thickness,
            hide_mouse_when_typing: self.terminal_settings.hide_mouse_when_typing,
            copy_on_select: self.terminal_settings.copy_on_select,
            semantic_escape_chars: self.terminal_settings.semantic_escape_chars.clone(),
            osc52_policy: self.terminal_settings.osc52_policy,
            kitty_keyboard: self.terminal_settings.kitty_keyboard,
            hint_alphabet: self.terminal_settings.hint_alphabet.clone(),
            hints: self.terminal_settings.hints.clone(),
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
            editor_invisible: Some(self.line_number.into()),
            editor_gutter_background: Some(self.background.into()),
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

#[derive(Clone, Debug, PartialEq)]
pub struct TerminalTheme {
    pub background: Rgba,
    pub foreground: Rgba,
    pub cursor: Option<Rgba>,
    pub selection_background: Option<Rgba>,
    pub selection_foreground: Option<Rgba>,
    pub cursor_text: Option<Rgba>,
    pub search_foreground: Rgba,
    pub search_background: Rgba,
    pub focused_search_foreground: Rgba,
    pub focused_search_background: Rgba,
    pub hint_start_foreground: Rgba,
    pub hint_start_background: Rgba,
    pub hint_end_foreground: Rgba,
    pub hint_end_background: Rgba,
    pub normal: AnsiColors,
    pub bright: AnsiColors,
    pub indexed_colors: Vec<IndexedColor>,
}

impl TerminalTheme {
    pub fn to_color_palette(&self) -> ColorPalette {
        let (background_r, background_g, background_b) = color_bytes(self.background);
        let (foreground_r, foreground_g, foreground_b) = color_bytes(self.foreground);
        let cursor = self.cursor.unwrap_or(self.foreground);
        let (cursor_r, cursor_g, cursor_b) = color_bytes(cursor);
        let selection_background = self.selection_background.unwrap_or(self.background);
        let (selection_r, selection_g, selection_b) = color_bytes(selection_background);

        let mut builder = ColorPalette::builder()
            .background(background_r, background_g, background_b)
            .background_alpha(self.background.a)
            .foreground(foreground_r, foreground_g, foreground_b)
            .cursor(cursor_r, cursor_g, cursor_b)
            .selection_background(selection_r, selection_g, selection_b);

        if let Some(color) = self.selection_foreground {
            let (r, g, b) = color_bytes(color);
            builder = builder.selection_foreground(r, g, b);
        }
        if let Some(color) = self.cursor_text {
            let (r, g, b) = color_bytes(color);
            builder = builder.cursor_text(r, g, b);
        }

        builder = builder
            .search(
                color_bytes(self.search_foreground),
                color_bytes(self.search_background),
            )
            .focused_search(
                color_bytes(self.focused_search_foreground),
                color_bytes(self.focused_search_background),
            )
            .hint_start(
                color_bytes(self.hint_start_foreground),
                color_bytes(self.hint_start_background),
            )
            .hint_end(
                color_bytes(self.hint_end_foreground),
                color_bytes(self.hint_end_background),
            );

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

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct IndexedColor {
    pub index: u8,
    pub color: Rgba,
}

pub(crate) fn color_hex(color: Rgba) -> String {
    let r = (color.r.clamp(0.0, 1.0) * 255.0).round() as u8;
    let g = (color.g.clamp(0.0, 1.0) * 255.0).round() as u8;
    let b = (color.b.clamp(0.0, 1.0) * 255.0).round() as u8;
    let a = (color.a.clamp(0.0, 1.0) * 255.0).round() as u8;
    if a == u8::MAX {
        format!("#{r:02x}{g:02x}{b:02x}")
    } else {
        format!("#{r:02x}{g:02x}{b:02x}{a:02x}")
    }
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
