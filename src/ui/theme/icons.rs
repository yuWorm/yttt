use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context as _, Result, anyhow};
use gpui::{AnyElement, ImageSource, IntoElement, Resource, Rgba, SharedString, Styled as _, img};
use gpui_component::{Icon, IconName};
use serde::Deserialize;

use crate::{config::paths::AppConfigPaths, ui::app::assets::external_icon_asset_path};

#[derive(Clone)]
pub enum IconVisual {
    Asset(SharedString),
    Component(IconName),
}

pub fn icon_for_visual(visual: IconVisual, fallback_color: impl Into<Rgba>) -> AnyElement {
    let fallback_color = fallback_color.into();
    match visual {
        IconVisual::Asset(path) => img(ImageSource::Resource(Resource::Embedded(path)))
            .size_3()
            .into_any_element(),
        IconVisual::Component(icon) => Icon::new(icon)
            .size_3()
            .text_color(fallback_color)
            .into_any_element(),
    }
}

#[derive(Clone, Default)]
pub struct IconTheme {
    root: Option<PathBuf>,
    assets_root: Option<PathBuf>,
    directory_icons: DirectoryIcons,
    named_directory_icons: HashMap<String, DirectoryIcons>,
    chevron_icons: ChevronIcons,
    file_stems: HashMap<String, String>,
    file_suffixes: HashMap<String, String>,
    file_icons: HashMap<String, String>,
}

#[derive(Clone, Default)]
struct DirectoryIcons {
    collapsed: Option<String>,
    expanded: Option<String>,
}

#[derive(Clone, Default)]
struct ChevronIcons {
    collapsed: Option<String>,
    expanded: Option<String>,
}

impl IconTheme {
    pub fn resolve_file(&self, path: &Path) -> IconVisual {
        let icon = path
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| self.resolve_file_name(name))
            .or_else(|| self.resolve_icon_key("default"));

        icon.unwrap_or(IconVisual::Component(IconName::File))
    }

    pub fn resolve_directory(&self, path: &Path, expanded: bool) -> IconVisual {
        let icon = path
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| self.named_directory_icons.get(name))
            .and_then(|icons| self.resolve_directory_icon(icons, expanded))
            .or_else(|| self.resolve_directory_icon(&self.directory_icons, expanded));

        icon.unwrap_or_else(|| {
            IconVisual::Component(if expanded {
                IconName::FolderOpen
            } else {
                IconName::FolderClosed
            })
        })
    }

    pub fn resolve_chevron(&self, expanded: bool) -> IconVisual {
        let icon = if expanded {
            self.chevron_icons.expanded.as_deref()
        } else {
            self.chevron_icons.collapsed.as_deref()
        }
        .and_then(|path| self.resolve_asset_path(path));

        icon.map(IconVisual::Asset).unwrap_or_else(|| {
            IconVisual::Component(if expanded {
                IconName::ChevronDown
            } else {
                IconName::ChevronRight
            })
        })
    }

    fn resolve_file_name(&self, name: &str) -> Option<IconVisual> {
        if let Some(icon) = self.resolve_lookup(name) {
            return Some(icon);
        }

        let mut suffix = name;
        while let Some((_, next_suffix)) = suffix.split_once('.') {
            if let Some(icon) = self.resolve_lookup(next_suffix) {
                return Some(icon);
            }
            suffix = next_suffix;
        }

        let extension = Path::new(name)
            .extension()
            .and_then(|extension| extension.to_str());
        extension.and_then(|extension| self.resolve_lookup(extension))
    }

    fn resolve_lookup(&self, key: &str) -> Option<IconVisual> {
        self.file_stems
            .get(key)
            .or_else(|| self.file_suffixes.get(key))
            .and_then(|icon_key| self.resolve_icon_key(icon_key))
    }

    fn resolve_icon_key(&self, key: &str) -> Option<IconVisual> {
        self.file_icons
            .get(key)
            .and_then(|path| self.resolve_asset_path(path))
            .map(IconVisual::Asset)
    }

    fn resolve_directory_icon(&self, icons: &DirectoryIcons, expanded: bool) -> Option<IconVisual> {
        let path = if expanded {
            icons.expanded.as_deref()
        } else {
            icons.collapsed.as_deref()
        }?;
        self.resolve_asset_path(path).map(IconVisual::Asset)
    }

    fn resolve_asset_path(&self, path: &str) -> Option<SharedString> {
        let root = self.root.as_ref()?;
        let assets_root = self.assets_root.as_ref()?;
        let resolved = root.join(path).canonicalize().ok()?;
        let relative_path = resolved.strip_prefix(assets_root).ok()?;
        resolved
            .starts_with(root)
            .then(|| external_icon_asset_path(relative_path))
    }
}

pub fn load_icon_theme(
    config_paths: &AppConfigPaths,
    requested_theme: Option<&str>,
) -> Result<IconTheme> {
    let Some(requested_theme) = requested_theme else {
        return Ok(IconTheme::default());
    };
    let (assets_root, candidates) = load_icon_theme_candidates(config_paths)?;
    let Some(candidate) = candidates.into_iter().find(|candidate| {
        candidate.package_name == requested_theme
            || candidate.family_name == requested_theme
            || candidate.theme.name == requested_theme
    }) else {
        return Err(anyhow!("icon theme {requested_theme:?} was not found"));
    };

    let IconThemeCandidate { root, theme, .. } = candidate;
    Ok(IconTheme {
        root: Some(root),
        assets_root: Some(assets_root),
        directory_icons: theme.directory_icons.into(),
        named_directory_icons: theme
            .named_directory_icons
            .into_iter()
            .map(|(name, icons)| (name, icons.into()))
            .collect(),
        chevron_icons: theme.chevron_icons.into(),
        file_stems: theme.file_stems,
        file_suffixes: theme.file_suffixes,
        file_icons: theme
            .file_icons
            .into_iter()
            .map(|(name, icon)| (name, icon.path))
            .collect(),
    })
}

pub fn available_icon_theme_names(config_paths: &AppConfigPaths) -> Result<Vec<String>> {
    let (_, candidates) = load_icon_theme_candidates(config_paths)?;
    let mut names = candidates
        .into_iter()
        .map(|candidate| candidate.theme.name)
        .collect::<Vec<_>>();
    names.sort();
    names.dedup();
    Ok(names)
}

fn load_icon_theme_candidates(
    config_paths: &AppConfigPaths,
) -> Result<(PathBuf, Vec<IconThemeCandidate>)> {
    let packages_dir = config_paths.icon_themes_dir();
    fs::create_dir_all(&packages_dir).with_context(|| {
        format!(
            "failed to create icon theme directory {}",
            packages_dir.display()
        )
    })?;
    let assets_root = packages_dir.canonicalize().with_context(|| {
        format!(
            "failed to resolve icon theme directory {}",
            packages_dir.display()
        )
    })?;

    let mut packages = fs::read_dir(&assets_root)
        .with_context(|| {
            format!(
                "failed to read icon theme directory {}",
                assets_root.display()
            )
        })?
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().is_dir())
        .collect::<Vec<_>>();
    packages.sort_by_key(|entry| entry.file_name());

    let mut candidates = Vec::new();
    for package in packages {
        let package_root = package.path().canonicalize().with_context(|| {
            format!(
                "failed to resolve icon theme package {}",
                package.path().display()
            )
        })?;
        let package_name = package.file_name().to_string_lossy().into_owned();
        let theme_dir = package_root.join("icon_themes");
        if !theme_dir.is_dir() {
            continue;
        }

        let mut theme_files = fs::read_dir(&theme_dir)
            .with_context(|| {
                format!(
                    "failed to read icon theme directory {}",
                    theme_dir.display()
                )
            })?
            .filter_map(|entry| entry.ok())
            .filter(|entry| {
                entry
                    .path()
                    .extension()
                    .is_some_and(|extension| extension == "json")
            })
            .collect::<Vec<_>>();
        theme_files.sort_by_key(|entry| entry.file_name());

        for theme_file in theme_files {
            let contents = fs::read(theme_file.path()).with_context(|| {
                format!("failed to read icon theme {}", theme_file.path().display())
            })?;
            let family: IconThemeFamilyFile =
                serde_json::from_slice(&contents).with_context(|| {
                    format!("failed to parse icon theme {}", theme_file.path().display())
                })?;
            for theme in family.themes {
                candidates.push(IconThemeCandidate {
                    package_name: package_name.clone(),
                    family_name: family.name.clone(),
                    root: package_root.clone(),
                    theme,
                });
            }
        }
    }

    Ok((assets_root, candidates))
}

struct IconThemeCandidate {
    package_name: String,
    family_name: String,
    root: PathBuf,
    theme: IconThemeFile,
}

#[derive(Deserialize)]
struct IconThemeFamilyFile {
    name: String,
    themes: Vec<IconThemeFile>,
}

#[derive(Deserialize)]
struct IconThemeFile {
    name: String,
    #[serde(default)]
    directory_icons: DirectoryIconsFile,
    #[serde(default)]
    named_directory_icons: HashMap<String, DirectoryIconsFile>,
    #[serde(default)]
    chevron_icons: ChevronIconsFile,
    #[serde(default)]
    file_stems: HashMap<String, String>,
    #[serde(default)]
    file_suffixes: HashMap<String, String>,
    #[serde(default)]
    file_icons: HashMap<String, IconDefinitionFile>,
}

#[derive(Deserialize, Default)]
struct DirectoryIconsFile {
    collapsed: Option<String>,
    expanded: Option<String>,
}

impl From<DirectoryIconsFile> for DirectoryIcons {
    fn from(value: DirectoryIconsFile) -> Self {
        Self {
            collapsed: value.collapsed,
            expanded: value.expanded,
        }
    }
}

#[derive(Deserialize, Default)]
struct ChevronIconsFile {
    collapsed: Option<String>,
    expanded: Option<String>,
}

impl From<ChevronIconsFile> for ChevronIcons {
    fn from(value: ChevronIconsFile) -> Self {
        Self {
            collapsed: value.collapsed,
            expanded: value.expanded,
        }
    }
}

#[derive(Deserialize)]
struct IconDefinitionFile {
    path: String,
}
