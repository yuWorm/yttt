mod installed;

pub use installed::{
    DetectedZedExtension, ImportedZedIconTheme, ImportedZedThemes, ZedIconThemeImportError,
    ZedThemeDetection, ZedThemeDetectionWarning, ZedThemeImportSummaryError,
    detect_installed_zed_themes, detect_zed_theme_extension, detect_zed_themes_in,
    import_detected_zed_themes, import_detected_zed_themes_to, import_zed_icon_theme_extension,
};

use std::{
    collections::{HashMap, HashSet},
    fs::{self, OpenOptions},
    io::Write as _,
    path::{Path, PathBuf},
};

use gpui::{Rgba, rgb, rgba};
use gpui_component::ThemeMode;
use serde::Deserialize;
use thiserror::Error;

use super::{AppTheme, ThemeMetadata, ThemeSourceMetadata};
use crate::config::theme::serialize_theme_file;

#[derive(Clone, Debug, PartialEq)]
pub struct ConvertedZedTheme {
    pub theme: AppTheme,
    pub suggested_file_name: String,
}

impl ConvertedZedTheme {
    pub fn to_toml(&self) -> Result<String, ZedThemeImportError> {
        serialize_theme_file(&self.theme).map_err(|source| ZedThemeImportError::SerializeTheme {
            theme: self.theme.name.clone(),
            source,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportedZedTheme {
    pub theme_name: String,
    pub path: PathBuf,
}

#[derive(Debug, Error)]
pub enum ZedThemeImportError {
    #[error("failed to resolve Zed extension directory {path}: {source}")]
    ResolveExtensionDirectory {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to read Zed extension manifest {path}: {source}")]
    ReadManifest {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to parse Zed extension manifest {path}: {source}")]
    ParseManifest {
        path: PathBuf,
        source: toml::de::Error,
    },
    #[error("Zed extension manifest {path} does not list any themes")]
    NoThemes { path: PathBuf },
    #[error("failed to resolve Zed theme file {path}: {source}")]
    ResolveThemeFile {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("Zed theme path {path} escapes extension directory {extension_root}")]
    ThemeOutsideExtension {
        path: PathBuf,
        extension_root: PathBuf,
    },
    #[error("failed to read Zed theme file {path}: {source}")]
    ReadThemeFile {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to parse Zed theme file {path}: {source}")]
    ParseThemeFile {
        path: PathBuf,
        source: serde_json::Error,
    },
    #[error("Zed theme {theme:?} has invalid color {value:?} in {field}")]
    InvalidColor {
        theme: String,
        field: String,
        value: String,
    },
    #[error("multiple converted themes would use output file {file_name}")]
    DuplicateOutputFile { file_name: String },
    #[error("failed to serialize converted theme {theme:?}: {source}")]
    SerializeTheme {
        theme: String,
        source: toml::ser::Error,
    },
    #[error("failed to create theme output directory {path}: {source}")]
    CreateOutputDirectory {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("theme output file already exists: {path}")]
    OutputExists { path: PathBuf },
    #[error("failed to write converted theme {path}: {source}")]
    WriteTheme {
        path: PathBuf,
        source: std::io::Error,
    },
}

pub fn convert_zed_theme_extension(
    extension_dir: impl AsRef<Path>,
) -> Result<Vec<ConvertedZedTheme>, ZedThemeImportError> {
    let requested_root = extension_dir.as_ref();
    let extension_root = requested_root.canonicalize().map_err(|source| {
        ZedThemeImportError::ResolveExtensionDirectory {
            path: requested_root.to_path_buf(),
            source,
        }
    })?;
    let manifest_path = extension_root.join("extension.toml");
    let manifest_source =
        fs::read_to_string(&manifest_path).map_err(|source| ZedThemeImportError::ReadManifest {
            path: manifest_path.clone(),
            source,
        })?;
    let manifest: ZedExtensionManifest =
        toml::from_str(&manifest_source).map_err(|source| ZedThemeImportError::ParseManifest {
            path: manifest_path.clone(),
            source,
        })?;
    if manifest.themes.is_empty() {
        return Err(ZedThemeImportError::NoThemes {
            path: manifest_path,
        });
    }

    let mut converted = Vec::new();
    let mut output_names = HashSet::new();
    for relative_theme_path in &manifest.themes {
        let unresolved_theme_path = extension_root.join(relative_theme_path);
        let theme_path = unresolved_theme_path.canonicalize().map_err(|source| {
            ZedThemeImportError::ResolveThemeFile {
                path: unresolved_theme_path.clone(),
                source,
            }
        })?;
        if !theme_path.starts_with(&extension_root) {
            return Err(ZedThemeImportError::ThemeOutsideExtension {
                path: theme_path,
                extension_root,
            });
        }
        let source = fs::read_to_string(&theme_path).map_err(|source| {
            ZedThemeImportError::ReadThemeFile {
                path: theme_path.clone(),
                source,
            }
        })?;
        let family: ZedThemeFamily = serde_json::from_str(&source).map_err(|source| {
            ZedThemeImportError::ParseThemeFile {
                path: theme_path,
                source,
            }
        })?;

        for definition in family.themes {
            let metadata = metadata_from_zed(
                &manifest,
                relative_theme_path,
                &family.name,
                family.author.as_deref(),
            );
            let theme = convert_theme(definition, metadata)?;
            let package_slug = slugify(&manifest.id);
            let theme_slug = slugify(&theme.name);
            let stem = if package_slug == theme_slug {
                package_slug
            } else {
                format!("{package_slug}-{theme_slug}")
            };
            let suggested_file_name = format!("{stem}.toml");
            if !output_names.insert(suggested_file_name.clone()) {
                return Err(ZedThemeImportError::DuplicateOutputFile {
                    file_name: suggested_file_name,
                });
            }
            converted.push(ConvertedZedTheme {
                theme,
                suggested_file_name,
            });
        }
    }

    if converted.is_empty() {
        return Err(ZedThemeImportError::NoThemes {
            path: manifest_path,
        });
    }
    Ok(converted)
}

pub fn import_zed_theme_extension(
    extension_dir: impl AsRef<Path>,
    output_dir: impl AsRef<Path>,
) -> Result<Vec<ImportedZedTheme>, ZedThemeImportError> {
    let converted = convert_zed_theme_extension(extension_dir)?;
    let output_dir = output_dir.as_ref();
    fs::create_dir_all(output_dir).map_err(|source| {
        ZedThemeImportError::CreateOutputDirectory {
            path: output_dir.to_path_buf(),
            source,
        }
    })?;

    let mut staged = Vec::with_capacity(converted.len());
    for converted_theme in converted {
        let path = output_dir.join(&converted_theme.suggested_file_name);
        if path.exists() {
            return Err(ZedThemeImportError::OutputExists { path });
        }
        let source = converted_theme.to_toml()?;
        staged.push((converted_theme.theme.name, path, source));
    }

    let mut written_paths = Vec::with_capacity(staged.len());
    let mut imported = Vec::with_capacity(staged.len());
    for (theme_name, path, source) in staged {
        let write_result = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .and_then(|mut file| file.write_all(source.as_bytes()));
        if let Err(source) = write_result {
            for written_path in &written_paths {
                let _ = fs::remove_file(written_path);
            }
            let _ = fs::remove_file(&path);
            return Err(ZedThemeImportError::WriteTheme { path, source });
        }
        written_paths.push(path.clone());
        imported.push(ImportedZedTheme { theme_name, path });
    }

    Ok(imported)
}

fn metadata_from_zed(
    manifest: &ZedExtensionManifest,
    theme_file: &str,
    family_name: &str,
    family_author: Option<&str>,
) -> ThemeMetadata {
    let mut authors = manifest.authors.clone();
    if let Some(author) = family_author.filter(|author| !author.trim().is_empty())
        && !authors.iter().any(|existing| existing == author)
    {
        authors.push(author.to_string());
    }

    ThemeMetadata {
        authors,
        description: manifest.description.clone(),
        repository: manifest.repository.clone(),
        converted_from: Some("Zed theme".to_string()),
        source: Some(ThemeSourceMetadata {
            format: "zed".to_string(),
            extension_id: Some(manifest.id.clone()),
            extension_name: Some(manifest.name.clone()),
            extension_version: Some(manifest.version.clone()),
            theme_file: Some(theme_file.to_string()),
            family_name: Some(family_name.to_string()),
            family_author: family_author.map(ToString::to_string),
        }),
    }
}

fn convert_theme(
    definition: ZedThemeDefinition,
    metadata: ThemeMetadata,
) -> Result<AppTheme, ZedThemeImportError> {
    let fallback = AppTheme::one_dark();
    let mut ui = fallback.ui;
    let mut editor = fallback.editor;
    let mut terminal = fallback.terminal;
    let style = definition.style;
    let theme_name = definition.name;

    ui.app_background = resolve_color(
        &theme_name,
        &[
            ("background", style.background.as_deref()),
            ("surface.background", style.surface_background.as_deref()),
            ("editor.background", style.editor_background.as_deref()),
        ],
        ui.app_background,
    )?;
    ui.surface = resolve_color(
        &theme_name,
        &[
            ("surface.background", style.surface_background.as_deref()),
            ("background", style.background.as_deref()),
        ],
        ui.app_background,
    )?;
    ui.surface_elevated = resolve_color(
        &theme_name,
        &[
            (
                "elevated_surface.background",
                style.elevated_surface_background.as_deref(),
            ),
            (
                "status_bar.background",
                style.status_bar_background.as_deref(),
            ),
        ],
        ui.surface,
    )?;
    ui.titlebar_background = resolve_color(
        &theme_name,
        &[
            (
                "title_bar.background",
                style.title_bar_background.as_deref(),
            ),
            ("toolbar.background", style.toolbar_background.as_deref()),
        ],
        ui.app_background,
    )?;
    ui.sidebar_background = resolve_color(
        &theme_name,
        &[
            ("panel.background", style.panel_background.as_deref()),
            ("surface.background", style.surface_background.as_deref()),
        ],
        ui.surface,
    )?;
    ui.tabbar_background = resolve_color(
        &theme_name,
        &[
            ("tab_bar.background", style.tab_bar_background.as_deref()),
            (
                "tab.inactive_background",
                style.tab_inactive_background.as_deref(),
            ),
        ],
        ui.surface_elevated,
    )?;
    ui.terminal_background = resolve_color(
        &theme_name,
        &[
            ("terminal.background", style.terminal_background.as_deref()),
            ("editor.background", style.editor_background.as_deref()),
        ],
        ui.app_background,
    )?;
    ui.border = resolve_color(
        &theme_name,
        &[
            ("border", style.border.as_deref()),
            ("border.variant", style.border_variant.as_deref()),
        ],
        ui.border,
    )?;
    ui.border_strong = resolve_color(
        &theme_name,
        &[
            (
                "scrollbar.thumb.border",
                style.scrollbar_thumb_border.as_deref(),
            ),
            ("border.selected", style.border_selected.as_deref()),
        ],
        ui.border,
    )?;
    ui.split_line = resolve_color(
        &theme_name,
        &[
            ("border.variant", style.border_variant.as_deref()),
            ("border", style.border.as_deref()),
        ],
        ui.border,
    )?;
    ui.split_line_active = resolve_color(
        &theme_name,
        &[
            ("border.focused", style.border_focused.as_deref()),
            (
                "scrollbar.thumb.hover_background",
                style.scrollbar_thumb_hover_background.as_deref(),
            ),
        ],
        ui.border_strong,
    )?;
    ui.text = resolve_color(
        &theme_name,
        &[
            ("text", style.text.as_deref()),
            ("editor.foreground", style.editor_foreground.as_deref()),
        ],
        ui.text,
    )?;
    ui.text_muted = resolve_color(
        &theme_name,
        &[
            ("text.muted", style.text_muted.as_deref()),
            ("hint", style.hint.as_deref()),
            ("editor.line_number", style.editor_line_number.as_deref()),
        ],
        ui.text_muted,
    )?;
    ui.text_subtle = resolve_color(
        &theme_name,
        &[
            ("text.placeholder", style.text_placeholder.as_deref()),
            ("ignored", style.ignored.as_deref()),
            ("editor.line_number", style.editor_line_number.as_deref()),
        ],
        ui.text_subtle,
    )?;
    ui.accent = resolve_color(
        &theme_name,
        &[
            ("text.accent", style.text_accent.as_deref()),
            ("syntax.function.color", syntax_value(&style, "function")),
            ("players[0].cursor", player_cursor(&style)),
        ],
        ui.accent,
    )?;
    ui.active_surface = resolve_color(
        &theme_name,
        &[
            ("element.selected", style.element_selected.as_deref()),
            (
                "ghost_element.selected",
                style.ghost_element_selected.as_deref(),
            ),
            (
                "tab.active_background",
                style.tab_active_background.as_deref(),
            ),
        ],
        ui.active_surface,
    )?;
    ui.hover_surface = resolve_color(
        &theme_name,
        &[
            ("element.hover", style.element_hover.as_deref()),
            ("ghost_element.hover", style.ghost_element_hover.as_deref()),
        ],
        ui.active_surface,
    )?;
    ui.danger = resolve_color(&theme_name, &[("error", style.error.as_deref())], ui.danger)?;
    ui.success = resolve_color(
        &theme_name,
        &[
            ("success", style.success.as_deref()),
            ("created", style.created.as_deref()),
        ],
        ui.success,
    )?;
    ui.warning = resolve_color(
        &theme_name,
        &[
            ("warning", style.warning.as_deref()),
            ("modified", style.modified.as_deref()),
        ],
        ui.warning,
    )?;
    ui.focus_ring = resolve_color(
        &theme_name,
        &[
            ("border.focused", style.border_focused.as_deref()),
            ("border.selected", style.border_selected.as_deref()),
        ],
        ui.border,
    )?;
    ui.selection = resolve_color(
        &theme_name,
        &[("players[0].selection", player_selection(&style))],
        ui.selection,
    )?;
    ui.focused_pane_border = resolve_color(
        &theme_name,
        &[
            (
                "panel.focused_border",
                style.panel_focused_border.as_deref(),
            ),
            ("border.focused", style.border_focused.as_deref()),
        ],
        ui.focus_ring,
    )?;

    editor.background = resolve_color(
        &theme_name,
        &[
            ("editor.background", style.editor_background.as_deref()),
            ("background", style.background.as_deref()),
        ],
        ui.app_background,
    )?;
    editor.foreground = resolve_color(
        &theme_name,
        &[
            ("editor.foreground", style.editor_foreground.as_deref()),
            ("text", style.text.as_deref()),
        ],
        ui.text,
    )?;
    editor.active_line = resolve_color(
        &theme_name,
        &[(
            "editor.active_line.background",
            style.editor_active_line_background.as_deref(),
        )],
        ui.active_surface,
    )?;
    editor.line_number = resolve_color(
        &theme_name,
        &[("editor.line_number", style.editor_line_number.as_deref())],
        ui.text_subtle,
    )?;
    editor.active_line_number = resolve_color(
        &theme_name,
        &[(
            "editor.active_line_number",
            style.editor_active_line_number.as_deref(),
        )],
        editor.foreground,
    )?;
    editor.syntax.boolean =
        resolve_syntax_color(&theme_name, &style, &["boolean"], editor.syntax.boolean)?;
    editor.syntax.comment =
        resolve_syntax_color(&theme_name, &style, &["comment"], editor.syntax.comment)?;
    editor.syntax.comment_doc = resolve_syntax_color(
        &theme_name,
        &style,
        &["comment.doc", "comment"],
        editor.syntax.comment,
    )?;
    editor.syntax.constant = resolve_syntax_color(
        &theme_name,
        &style,
        &["constant", "attribute"],
        editor.syntax.constant,
    )?;
    editor.syntax.constructor = resolve_syntax_color(
        &theme_name,
        &style,
        &["constructor", "tag"],
        editor.syntax.constructor,
    )?;
    editor.syntax.function =
        resolve_syntax_color(&theme_name, &style, &["function"], editor.syntax.function)?;
    editor.syntax.keyword = resolve_syntax_color(
        &theme_name,
        &style,
        &["keyword", "keyword.operator"],
        editor.syntax.keyword,
    )?;
    editor.syntax.number =
        resolve_syntax_color(&theme_name, &style, &["number"], editor.syntax.number)?;
    editor.syntax.operator = resolve_syntax_color(
        &theme_name,
        &style,
        &["operator", "keyword.operator"],
        editor.syntax.operator,
    )?;
    editor.syntax.property =
        resolve_syntax_color(&theme_name, &style, &["property"], editor.syntax.property)?;
    editor.syntax.punctuation = resolve_syntax_color(
        &theme_name,
        &style,
        &["punctuation"],
        editor.syntax.punctuation,
    )?;
    editor.syntax.string = resolve_syntax_color(
        &theme_name,
        &style,
        &["string", "text.literal"],
        editor.syntax.string,
    )?;
    editor.syntax.string_escape = resolve_syntax_color(
        &theme_name,
        &style,
        &["string.escape", "string.special"],
        editor.syntax.string_escape,
    )?;
    editor.syntax.type_ =
        resolve_syntax_color(&theme_name, &style, &["type", "enum"], editor.syntax.type_)?;
    editor.syntax.variable =
        resolve_syntax_color(&theme_name, &style, &["variable"], editor.syntax.variable)?;
    editor.syntax.variable_special = resolve_syntax_color(
        &theme_name,
        &style,
        &["variable.special", "variant"],
        editor.syntax.variable_special,
    )?;

    terminal.background = resolve_color(
        &theme_name,
        &[
            ("terminal.background", style.terminal_background.as_deref()),
            ("editor.background", style.editor_background.as_deref()),
        ],
        editor.background,
    )?;
    terminal.foreground = resolve_color(
        &theme_name,
        &[
            ("terminal.foreground", style.terminal_foreground.as_deref()),
            ("editor.foreground", style.editor_foreground.as_deref()),
        ],
        editor.foreground,
    )?;
    terminal.cursor = Some(resolve_color(
        &theme_name,
        &[("terminal.cursor", style.terminal_cursor.as_deref())],
        terminal.foreground,
    )?);
    let terminal_selection = resolve_color(
        &theme_name,
        &[("players[0].selection", player_selection(&style))],
        terminal.selection_background.unwrap_or(terminal.background),
    )?;
    terminal.selection_background = Some(composite_over(terminal_selection, terminal.background));
    terminal.normal.black = resolve_color(
        &theme_name,
        &[("terminal.ansi.black", style.terminal_ansi_black.as_deref())],
        terminal.normal.black,
    )?;
    terminal.normal.red = resolve_color(
        &theme_name,
        &[("terminal.ansi.red", style.terminal_ansi_red.as_deref())],
        terminal.normal.red,
    )?;
    terminal.normal.green = resolve_color(
        &theme_name,
        &[("terminal.ansi.green", style.terminal_ansi_green.as_deref())],
        terminal.normal.green,
    )?;
    terminal.normal.yellow = resolve_color(
        &theme_name,
        &[(
            "terminal.ansi.yellow",
            style.terminal_ansi_yellow.as_deref(),
        )],
        terminal.normal.yellow,
    )?;
    terminal.normal.blue = resolve_color(
        &theme_name,
        &[("terminal.ansi.blue", style.terminal_ansi_blue.as_deref())],
        terminal.normal.blue,
    )?;
    terminal.normal.magenta = resolve_color(
        &theme_name,
        &[(
            "terminal.ansi.magenta",
            style.terminal_ansi_magenta.as_deref(),
        )],
        terminal.normal.magenta,
    )?;
    terminal.normal.cyan = resolve_color(
        &theme_name,
        &[("terminal.ansi.cyan", style.terminal_ansi_cyan.as_deref())],
        terminal.normal.cyan,
    )?;
    terminal.normal.white = resolve_color(
        &theme_name,
        &[("terminal.ansi.white", style.terminal_ansi_white.as_deref())],
        terminal.normal.white,
    )?;
    terminal.bright.black = resolve_color(
        &theme_name,
        &[(
            "terminal.ansi.bright_black",
            style.terminal_ansi_bright_black.as_deref(),
        )],
        terminal.bright.black,
    )?;
    terminal.bright.red = resolve_color(
        &theme_name,
        &[(
            "terminal.ansi.bright_red",
            style.terminal_ansi_bright_red.as_deref(),
        )],
        terminal.bright.red,
    )?;
    terminal.bright.green = resolve_color(
        &theme_name,
        &[(
            "terminal.ansi.bright_green",
            style.terminal_ansi_bright_green.as_deref(),
        )],
        terminal.bright.green,
    )?;
    terminal.bright.yellow = resolve_color(
        &theme_name,
        &[(
            "terminal.ansi.bright_yellow",
            style.terminal_ansi_bright_yellow.as_deref(),
        )],
        terminal.bright.yellow,
    )?;
    terminal.bright.blue = resolve_color(
        &theme_name,
        &[(
            "terminal.ansi.bright_blue",
            style.terminal_ansi_bright_blue.as_deref(),
        )],
        terminal.bright.blue,
    )?;
    terminal.bright.magenta = resolve_color(
        &theme_name,
        &[(
            "terminal.ansi.bright_magenta",
            style.terminal_ansi_bright_magenta.as_deref(),
        )],
        terminal.bright.magenta,
    )?;
    terminal.bright.cyan = resolve_color(
        &theme_name,
        &[(
            "terminal.ansi.bright_cyan",
            style.terminal_ansi_bright_cyan.as_deref(),
        )],
        terminal.bright.cyan,
    )?;
    terminal.bright.white = resolve_color(
        &theme_name,
        &[(
            "terminal.ansi.bright_white",
            style.terminal_ansi_bright_white.as_deref(),
        )],
        terminal.bright.white,
    )?;

    Ok(AppTheme {
        name: theme_name,
        mode: if definition.appearance.eq_ignore_ascii_case("light") {
            ThemeMode::Light
        } else {
            ThemeMode::Dark
        },
        metadata,
        ui,
        editor,
        terminal,
    })
}

fn resolve_color(
    theme: &str,
    candidates: &[(&str, Option<&str>)],
    fallback: Rgba,
) -> Result<Rgba, ZedThemeImportError> {
    for (field, value) in candidates {
        if let Some(value) = value {
            return parse_color(value).ok_or_else(|| ZedThemeImportError::InvalidColor {
                theme: theme.to_string(),
                field: (*field).to_string(),
                value: (*value).to_string(),
            });
        }
    }
    Ok(fallback)
}

fn resolve_syntax_color(
    theme: &str,
    style: &ZedStyle,
    keys: &[&str],
    fallback: Rgba,
) -> Result<Rgba, ZedThemeImportError> {
    for key in keys {
        if let Some(value) = syntax_value(style, key) {
            return parse_color(value).ok_or_else(|| ZedThemeImportError::InvalidColor {
                theme: theme.to_string(),
                field: format!("syntax.{key}.color"),
                value: value.to_string(),
            });
        }
    }
    Ok(fallback)
}

fn syntax_value<'a>(style: &'a ZedStyle, key: &str) -> Option<&'a str> {
    style
        .syntax
        .get(key)
        .and_then(|entry| entry.color.as_deref())
}

fn player_cursor(style: &ZedStyle) -> Option<&str> {
    style
        .players
        .first()
        .and_then(|player| player.cursor.as_deref())
}

fn player_selection(style: &ZedStyle) -> Option<&str> {
    style
        .players
        .first()
        .and_then(|player| player.selection.as_deref())
}

fn parse_color(value: &str) -> Option<Rgba> {
    let value = value.trim().strip_prefix('#').unwrap_or(value.trim());
    match value.len() {
        6 => u32::from_str_radix(value, 16).ok().map(rgb),
        8 => u32::from_str_radix(value, 16).ok().map(rgba),
        _ => None,
    }
}

fn composite_over(foreground: Rgba, background: Rgba) -> Rgba {
    let alpha = foreground.a;
    Rgba {
        r: foreground.r * alpha + background.r * (1.0 - alpha),
        g: foreground.g * alpha + background.g * (1.0 - alpha),
        b: foreground.b * alpha + background.b * (1.0 - alpha),
        a: 1.0,
    }
}

fn slugify(value: &str) -> String {
    let mut slug = String::new();
    let mut pending_separator = false;
    for character in value.chars() {
        if character.is_ascii_alphanumeric() {
            if pending_separator && !slug.is_empty() {
                slug.push('-');
            }
            slug.push(character.to_ascii_lowercase());
            pending_separator = false;
        } else {
            pending_separator = true;
        }
    }
    if slug.is_empty() {
        "theme".to_string()
    } else {
        slug
    }
}

#[derive(Debug, Deserialize)]
struct ZedExtensionManifest {
    id: String,
    name: String,
    version: String,
    description: Option<String>,
    repository: Option<String>,
    #[serde(default)]
    authors: Vec<String>,
    #[serde(default)]
    themes: Vec<String>,
    #[serde(default)]
    icon_themes: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct ZedThemeFamily {
    name: String,
    author: Option<String>,
    #[serde(default)]
    themes: Vec<ZedThemeDefinition>,
}

#[derive(Debug, Deserialize)]
struct ZedThemeDefinition {
    name: String,
    #[serde(default)]
    appearance: String,
    #[serde(default)]
    style: ZedStyle,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct ZedStyle {
    border: Option<String>,
    #[serde(rename = "border.variant")]
    border_variant: Option<String>,
    #[serde(rename = "border.focused")]
    border_focused: Option<String>,
    #[serde(rename = "border.selected")]
    border_selected: Option<String>,
    #[serde(rename = "elevated_surface.background")]
    elevated_surface_background: Option<String>,
    #[serde(rename = "surface.background")]
    surface_background: Option<String>,
    background: Option<String>,
    #[serde(rename = "element.hover")]
    element_hover: Option<String>,
    #[serde(rename = "element.selected")]
    element_selected: Option<String>,
    #[serde(rename = "ghost_element.hover")]
    ghost_element_hover: Option<String>,
    #[serde(rename = "ghost_element.selected")]
    ghost_element_selected: Option<String>,
    text: Option<String>,
    #[serde(rename = "text.muted")]
    text_muted: Option<String>,
    #[serde(rename = "text.placeholder")]
    text_placeholder: Option<String>,
    #[serde(rename = "text.accent")]
    text_accent: Option<String>,
    #[serde(rename = "status_bar.background")]
    status_bar_background: Option<String>,
    #[serde(rename = "title_bar.background")]
    title_bar_background: Option<String>,
    #[serde(rename = "toolbar.background")]
    toolbar_background: Option<String>,
    #[serde(rename = "tab_bar.background")]
    tab_bar_background: Option<String>,
    #[serde(rename = "tab.inactive_background")]
    tab_inactive_background: Option<String>,
    #[serde(rename = "tab.active_background")]
    tab_active_background: Option<String>,
    #[serde(rename = "panel.background")]
    panel_background: Option<String>,
    #[serde(rename = "panel.focused_border")]
    panel_focused_border: Option<String>,
    #[serde(rename = "scrollbar.thumb.border")]
    scrollbar_thumb_border: Option<String>,
    #[serde(rename = "scrollbar.thumb.hover_background")]
    scrollbar_thumb_hover_background: Option<String>,
    #[serde(rename = "editor.foreground")]
    editor_foreground: Option<String>,
    #[serde(rename = "editor.background")]
    editor_background: Option<String>,
    #[serde(rename = "editor.active_line.background")]
    editor_active_line_background: Option<String>,
    #[serde(rename = "editor.line_number")]
    editor_line_number: Option<String>,
    #[serde(rename = "editor.active_line_number")]
    editor_active_line_number: Option<String>,
    #[serde(rename = "terminal.background")]
    terminal_background: Option<String>,
    #[serde(rename = "terminal.foreground")]
    terminal_foreground: Option<String>,
    #[serde(rename = "terminal.cursor")]
    terminal_cursor: Option<String>,
    #[serde(rename = "terminal.ansi.black")]
    terminal_ansi_black: Option<String>,
    #[serde(rename = "terminal.ansi.bright_black")]
    terminal_ansi_bright_black: Option<String>,
    #[serde(rename = "terminal.ansi.red")]
    terminal_ansi_red: Option<String>,
    #[serde(rename = "terminal.ansi.bright_red")]
    terminal_ansi_bright_red: Option<String>,
    #[serde(rename = "terminal.ansi.green")]
    terminal_ansi_green: Option<String>,
    #[serde(rename = "terminal.ansi.bright_green")]
    terminal_ansi_bright_green: Option<String>,
    #[serde(rename = "terminal.ansi.yellow")]
    terminal_ansi_yellow: Option<String>,
    #[serde(rename = "terminal.ansi.bright_yellow")]
    terminal_ansi_bright_yellow: Option<String>,
    #[serde(rename = "terminal.ansi.blue")]
    terminal_ansi_blue: Option<String>,
    #[serde(rename = "terminal.ansi.bright_blue")]
    terminal_ansi_bright_blue: Option<String>,
    #[serde(rename = "terminal.ansi.magenta")]
    terminal_ansi_magenta: Option<String>,
    #[serde(rename = "terminal.ansi.bright_magenta")]
    terminal_ansi_bright_magenta: Option<String>,
    #[serde(rename = "terminal.ansi.cyan")]
    terminal_ansi_cyan: Option<String>,
    #[serde(rename = "terminal.ansi.bright_cyan")]
    terminal_ansi_bright_cyan: Option<String>,
    #[serde(rename = "terminal.ansi.white")]
    terminal_ansi_white: Option<String>,
    #[serde(rename = "terminal.ansi.bright_white")]
    terminal_ansi_bright_white: Option<String>,
    created: Option<String>,
    error: Option<String>,
    hint: Option<String>,
    ignored: Option<String>,
    modified: Option<String>,
    success: Option<String>,
    warning: Option<String>,
    players: Vec<ZedPlayer>,
    syntax: HashMap<String, ZedSyntaxStyle>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct ZedPlayer {
    cursor: Option<String>,
    selection: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct ZedSyntaxStyle {
    color: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config::{paths::AppConfigPaths, theme::load_theme_store},
        ui::theme::color_hex,
    };

    #[test]
    fn imports_theme_with_zed_metadata_and_loadable_colors() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let extension_dir = temp.path().join("extension");
        write_test_extension(&extension_dir);
        let config_dir = temp.path().join("config");
        let paths = AppConfigPaths::from_config_dir(&config_dir);

        let imported = import_zed_theme_extension(&extension_dir, paths.themes_dir())
            .expect("Zed theme imports");

        assert_eq!(imported.len(), 1);
        assert_eq!(imported[0].theme_name, "Test Theme");
        assert_eq!(
            imported[0].path.file_name().and_then(|name| name.to_str()),
            Some("test-theme.toml")
        );

        let loaded = load_theme_store(&paths).expect("theme store loads");
        assert!(loaded.warnings.is_empty());
        let theme = loaded.store.theme("Test Theme").expect("imported theme");
        assert_eq!(theme.mode, ThemeMode::Dark);
        assert_eq!(theme.metadata.authors, ["Ada", "Zed Family Author"]);
        assert_eq!(
            theme.metadata.description.as_deref(),
            Some("Theme package description")
        );
        assert_eq!(
            theme.metadata.repository.as_deref(),
            Some("https://github.com/example/test-theme")
        );
        assert_eq!(theme.metadata.converted_from.as_deref(), Some("Zed theme"));
        let source = theme.metadata.source.as_ref().expect("source metadata");
        assert_eq!(source.format, "zed");
        assert_eq!(source.extension_id.as_deref(), Some("test-theme"));
        assert_eq!(source.extension_version.as_deref(), Some("1.2.3"));
        assert_eq!(source.theme_file.as_deref(), Some("themes/test.json"));
        assert_eq!(source.family_author.as_deref(), Some("Zed Family Author"));
        assert_eq!(color_hex(theme.ui.selection), "#ffffff80");
        assert_eq!(
            theme
                .terminal
                .selection_background
                .map(color_hex)
                .as_deref(),
            Some("#888888")
        );
        assert_eq!(color_hex(theme.editor.syntax.function), "#123456");
        assert_eq!(color_hex(theme.terminal.normal.red), "#aa0000");
    }

    #[test]
    fn import_does_not_overwrite_an_existing_native_theme() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let extension_dir = temp.path().join("extension");
        write_test_extension(&extension_dir);
        let output_dir = temp.path().join("themes");
        fs::create_dir_all(&output_dir).expect("output directory");
        let existing_path = output_dir.join("test-theme.toml");
        fs::write(&existing_path, "user-owned").expect("existing theme");

        let error = import_zed_theme_extension(&extension_dir, &output_dir)
            .expect_err("existing theme must not be overwritten");

        assert!(matches!(
            error,
            ZedThemeImportError::OutputExists { path } if path == existing_path
        ));
        assert_eq!(
            fs::read_to_string(existing_path).expect("existing theme remains"),
            "user-owned"
        );
    }

    #[test]
    fn conversion_rejects_theme_paths_outside_the_extension() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let extension_dir = temp.path().join("extension");
        fs::create_dir_all(&extension_dir).expect("extension directory");
        fs::write(
            extension_dir.join("extension.toml"),
            r#"
id = "unsafe-theme"
name = "Unsafe Theme"
version = "1.0.0"
themes = ["../outside.json"]
"#,
        )
        .expect("extension manifest");
        fs::write(temp.path().join("outside.json"), "{}").expect("outside theme");

        let error = convert_zed_theme_extension(&extension_dir)
            .expect_err("theme path traversal must be rejected");

        assert!(matches!(
            error,
            ZedThemeImportError::ThemeOutsideExtension { .. }
        ));
    }

    fn write_test_extension(extension_dir: &Path) {
        let themes_dir = extension_dir.join("themes");
        fs::create_dir_all(&themes_dir).expect("theme directory");
        fs::write(
            extension_dir.join("extension.toml"),
            r#"
id = "test-theme"
name = "Test Theme Extension"
version = "1.2.3"
description = "Theme package description"
repository = "https://github.com/example/test-theme"
authors = ["Ada"]
themes = ["themes/test.json"]
"#,
        )
        .expect("extension manifest");
        fs::write(
            themes_dir.join("test.json"),
            r##"{
  "name": "Test Family",
  "author": "Zed Family Author",
  "themes": [
    {
      "name": "Test Theme",
      "appearance": "dark",
      "style": {
        "background": "#101010",
        "surface.background": "#111111",
        "editor.background": "#101010",
        "editor.foreground": "#eeeeee",
        "terminal.ansi.red": "#aa0000",
        "players": [
          {
            "cursor": "#abcdef",
            "selection": "#ffffff80"
          }
        ],
        "syntax": {
          "function": {
            "color": "#123456"
          }
        }
      }
    }
  ]
}"##,
        )
        .expect("Zed theme file");
    }
}
