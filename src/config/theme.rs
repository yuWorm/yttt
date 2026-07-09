use std::{collections::HashMap, fs, path::PathBuf};

use gpui::{Rgba, rgb};
use gpui_component::ThemeMode;

use crate::{
    config::paths::AppConfigPaths,
    ui::theme::{AnsiColors, AppTheme},
};

#[derive(Clone, Debug, PartialEq)]
pub struct ThemeStore {
    themes: HashMap<String, AppTheme>,
}

impl ThemeStore {
    pub fn builtin() -> Self {
        let mut themes = HashMap::new();
        let theme = AppTheme::builtin_dark();
        themes.insert(theme.name.clone(), theme);
        Self { themes }
    }

    pub fn theme(&self, name: &str) -> Option<&AppTheme> {
        self.themes.get(name)
    }

    pub fn theme_names(&self) -> Vec<String> {
        let mut names = self.themes.keys().cloned().collect::<Vec<_>>();
        names.sort();
        names
    }

    fn insert(&mut self, theme: AppTheme) {
        self.themes.insert(theme.name.clone(), theme);
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct LoadedThemeStore {
    pub store: ThemeStore,
    pub warnings: Vec<ThemeLoadWarning>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ThemeLoadWarning {
    ReadDir { path: PathBuf, message: String },
    ReadFile { path: PathBuf, message: String },
    ParseFile { path: PathBuf, message: String },
    InvalidColor { theme: String, field: String },
}

#[derive(Debug, thiserror::Error)]
pub enum ThemeLoadError {
    #[error("failed to create theme directory {path}: {source}")]
    CreateThemeDirectory {
        path: PathBuf,
        source: std::io::Error,
    },
}

pub fn load_theme_store(paths: &AppConfigPaths) -> Result<LoadedThemeStore, ThemeLoadError> {
    let mut store = ThemeStore::builtin();
    let mut warnings = Vec::new();
    let themes_dir = paths.themes_dir();

    fs::create_dir_all(&themes_dir).map_err(|source| ThemeLoadError::CreateThemeDirectory {
        path: themes_dir.clone(),
        source,
    })?;

    let entries = match fs::read_dir(&themes_dir) {
        Ok(entries) => entries,
        Err(error) => {
            warnings.push(ThemeLoadWarning::ReadDir {
                path: themes_dir,
                message: error.to_string(),
            });
            return Ok(LoadedThemeStore { store, warnings });
        }
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("toml") {
            continue;
        }
        let source = match fs::read_to_string(&path) {
            Ok(source) => source,
            Err(error) => {
                warnings.push(ThemeLoadWarning::ReadFile {
                    path,
                    message: error.to_string(),
                });
                continue;
            }
        };
        let file = match toml::from_str::<ThemeFile>(&source) {
            Ok(file) => file,
            Err(error) => {
                warnings.push(ThemeLoadWarning::ParseFile {
                    path,
                    message: error.to_string(),
                });
                continue;
            }
        };

        if let Some(theme) = theme_from_file(file, &mut warnings) {
            store.insert(theme);
        }
    }

    Ok(LoadedThemeStore { store, warnings })
}

#[derive(Clone, Debug, Default, serde::Deserialize)]
#[serde(default)]
struct ThemeFile {
    name: String,
    mode: String,
    ui: UiThemeFile,
    terminal: TerminalThemeFile,
}

#[derive(Clone, Debug, Default, serde::Deserialize)]
#[serde(default)]
struct UiThemeFile {
    background: Option<String>,
    surface: Option<String>,
    surface_elevated: Option<String>,
    titlebar: Option<String>,
    sidebar: Option<String>,
    tabbar: Option<String>,
    terminal_background: Option<String>,
    border: Option<String>,
    border_strong: Option<String>,
    split_line: Option<String>,
    split_line_active: Option<String>,
    text: Option<String>,
    text_muted: Option<String>,
    text_subtle: Option<String>,
    accent: Option<String>,
    active_surface: Option<String>,
    hover_surface: Option<String>,
    danger: Option<String>,
    success: Option<String>,
    warning: Option<String>,
    focus_ring: Option<String>,
    focused_pane_border: Option<String>,
}

#[derive(Clone, Debug, Default, serde::Deserialize)]
#[serde(default)]
struct TerminalThemeFile {
    colors: TerminalColorsFile,
}

#[derive(Clone, Debug, Default, serde::Deserialize)]
#[serde(default)]
struct TerminalColorsFile {
    primary: TerminalPrimaryColors,
    cursor: TerminalCursorColors,
    selection: TerminalSelectionColors,
    normal: AnsiColorsFile,
    bright: AnsiColorsFile,
}

#[derive(Clone, Debug, Default, serde::Deserialize)]
#[serde(default)]
struct TerminalPrimaryColors {
    background: Option<String>,
    foreground: Option<String>,
}

#[derive(Clone, Debug, Default, serde::Deserialize)]
#[serde(default)]
struct TerminalCursorColors {
    cursor: Option<String>,
}

#[derive(Clone, Debug, Default, serde::Deserialize)]
#[serde(default)]
struct TerminalSelectionColors {
    background: Option<String>,
}

#[derive(Clone, Debug, Default, serde::Deserialize)]
#[serde(default)]
struct AnsiColorsFile {
    black: Option<String>,
    red: Option<String>,
    green: Option<String>,
    yellow: Option<String>,
    blue: Option<String>,
    magenta: Option<String>,
    cyan: Option<String>,
    white: Option<String>,
}

fn theme_from_file(file: ThemeFile, warnings: &mut Vec<ThemeLoadWarning>) -> Option<AppTheme> {
    if file.name.trim().is_empty() {
        return None;
    }

    let fallback = AppTheme::builtin_dark();
    let mut ui = fallback.ui;
    let theme_name = file.name;

    apply_color(
        &mut ui.app_background,
        file.ui.background,
        &theme_name,
        "ui.background",
        warnings,
    );
    apply_color(
        &mut ui.surface,
        file.ui.surface,
        &theme_name,
        "ui.surface",
        warnings,
    );
    apply_color(
        &mut ui.surface_elevated,
        file.ui.surface_elevated,
        &theme_name,
        "ui.surface_elevated",
        warnings,
    );
    apply_color(
        &mut ui.titlebar_background,
        file.ui.titlebar,
        &theme_name,
        "ui.titlebar",
        warnings,
    );
    apply_color(
        &mut ui.sidebar_background,
        file.ui.sidebar,
        &theme_name,
        "ui.sidebar",
        warnings,
    );
    apply_color(
        &mut ui.tabbar_background,
        file.ui.tabbar,
        &theme_name,
        "ui.tabbar",
        warnings,
    );
    apply_color(
        &mut ui.terminal_background,
        file.ui.terminal_background,
        &theme_name,
        "ui.terminal_background",
        warnings,
    );
    apply_color(
        &mut ui.border,
        file.ui.border,
        &theme_name,
        "ui.border",
        warnings,
    );
    apply_color(
        &mut ui.border_strong,
        file.ui.border_strong,
        &theme_name,
        "ui.border_strong",
        warnings,
    );
    apply_color(
        &mut ui.split_line,
        file.ui.split_line,
        &theme_name,
        "ui.split_line",
        warnings,
    );
    apply_color(
        &mut ui.split_line_active,
        file.ui.split_line_active,
        &theme_name,
        "ui.split_line_active",
        warnings,
    );
    apply_color(&mut ui.text, file.ui.text, &theme_name, "ui.text", warnings);
    apply_color(
        &mut ui.text_muted,
        file.ui.text_muted,
        &theme_name,
        "ui.text_muted",
        warnings,
    );
    apply_color(
        &mut ui.text_subtle,
        file.ui.text_subtle,
        &theme_name,
        "ui.text_subtle",
        warnings,
    );
    apply_color(
        &mut ui.accent,
        file.ui.accent,
        &theme_name,
        "ui.accent",
        warnings,
    );
    apply_color(
        &mut ui.active_surface,
        file.ui.active_surface,
        &theme_name,
        "ui.active_surface",
        warnings,
    );
    apply_color(
        &mut ui.hover_surface,
        file.ui.hover_surface,
        &theme_name,
        "ui.hover_surface",
        warnings,
    );
    apply_color(
        &mut ui.danger,
        file.ui.danger,
        &theme_name,
        "ui.danger",
        warnings,
    );
    apply_color(
        &mut ui.success,
        file.ui.success,
        &theme_name,
        "ui.success",
        warnings,
    );
    apply_color(
        &mut ui.warning,
        file.ui.warning,
        &theme_name,
        "ui.warning",
        warnings,
    );
    apply_color(
        &mut ui.focus_ring,
        file.ui.focus_ring,
        &theme_name,
        "ui.focus_ring",
        warnings,
    );
    apply_color(
        &mut ui.focused_pane_border,
        file.ui.focused_pane_border,
        &theme_name,
        "ui.focused_pane_border",
        warnings,
    );

    let mut terminal = fallback.terminal;
    apply_color(
        &mut terminal.background,
        file.terminal.colors.primary.background,
        &theme_name,
        "terminal.colors.primary.background",
        warnings,
    );
    apply_color(
        &mut terminal.foreground,
        file.terminal.colors.primary.foreground,
        &theme_name,
        "terminal.colors.primary.foreground",
        warnings,
    );
    apply_optional_color(
        &mut terminal.cursor,
        file.terminal.colors.cursor.cursor,
        &theme_name,
        "terminal.colors.cursor.cursor",
        warnings,
    );
    apply_optional_color(
        &mut terminal.selection_background,
        file.terminal.colors.selection.background,
        &theme_name,
        "terminal.colors.selection.background",
        warnings,
    );
    apply_ansi_colors(
        &mut terminal.normal,
        file.terminal.colors.normal,
        &theme_name,
        "terminal.colors.normal",
        warnings,
    );
    apply_ansi_colors(
        &mut terminal.bright,
        file.terminal.colors.bright,
        &theme_name,
        "terminal.colors.bright",
        warnings,
    );

    Some(AppTheme {
        name: theme_name,
        mode: parse_theme_mode(&file.mode),
        ui,
        terminal,
    })
}

fn parse_theme_mode(mode: &str) -> ThemeMode {
    match mode {
        "light" => ThemeMode::Light,
        _ => ThemeMode::Dark,
    }
}

fn apply_color(
    target: &mut Rgba,
    value: Option<String>,
    theme: &str,
    field: &str,
    warnings: &mut Vec<ThemeLoadWarning>,
) {
    let Some(value) = value else {
        return;
    };
    match parse_hex_color(&value) {
        Some(color) => *target = color,
        None => warnings.push(ThemeLoadWarning::InvalidColor {
            theme: theme.to_string(),
            field: field.to_string(),
        }),
    }
}

fn apply_optional_color(
    target: &mut Option<Rgba>,
    value: Option<String>,
    theme: &str,
    field: &str,
    warnings: &mut Vec<ThemeLoadWarning>,
) {
    let Some(value) = value else {
        return;
    };
    match parse_hex_color(&value) {
        Some(color) => *target = Some(color),
        None => warnings.push(ThemeLoadWarning::InvalidColor {
            theme: theme.to_string(),
            field: field.to_string(),
        }),
    }
}

fn apply_ansi_colors(
    target: &mut AnsiColors,
    colors: AnsiColorsFile,
    theme: &str,
    prefix: &str,
    warnings: &mut Vec<ThemeLoadWarning>,
) {
    apply_color(
        &mut target.black,
        colors.black,
        theme,
        &format!("{prefix}.black"),
        warnings,
    );
    apply_color(
        &mut target.red,
        colors.red,
        theme,
        &format!("{prefix}.red"),
        warnings,
    );
    apply_color(
        &mut target.green,
        colors.green,
        theme,
        &format!("{prefix}.green"),
        warnings,
    );
    apply_color(
        &mut target.yellow,
        colors.yellow,
        theme,
        &format!("{prefix}.yellow"),
        warnings,
    );
    apply_color(
        &mut target.blue,
        colors.blue,
        theme,
        &format!("{prefix}.blue"),
        warnings,
    );
    apply_color(
        &mut target.magenta,
        colors.magenta,
        theme,
        &format!("{prefix}.magenta"),
        warnings,
    );
    apply_color(
        &mut target.cyan,
        colors.cyan,
        theme,
        &format!("{prefix}.cyan"),
        warnings,
    );
    apply_color(
        &mut target.white,
        colors.white,
        theme,
        &format!("{prefix}.white"),
        warnings,
    );
}

fn parse_hex_color(value: &str) -> Option<Rgba> {
    let value = value.trim().strip_prefix('#').unwrap_or(value.trim());
    if value.len() != 6 {
        return None;
    }
    u32::from_str_radix(value, 16).ok().map(rgb)
}
