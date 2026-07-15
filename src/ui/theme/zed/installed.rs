use std::{
    collections::HashSet,
    env, fs,
    path::{Path, PathBuf},
};

use serde::Deserialize;
use thiserror::Error;

use super::{
    ImportedZedTheme, ZedExtensionManifest, ZedThemeImportError, convert_zed_theme_extension,
    import_zed_theme_extension, slugify,
};
use crate::config::paths::AppConfigPaths;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ZedThemeDetection {
    pub roots: Vec<PathBuf>,
    pub extensions: Vec<DetectedZedExtension>,
    pub warnings: Vec<ZedThemeDetectionWarning>,
}

impl ZedThemeDetection {
    pub fn ui_theme_count(&self) -> usize {
        self.extensions
            .iter()
            .map(|extension| extension.ui_theme_names.len())
            .sum()
    }

    pub fn icon_theme_count(&self) -> usize {
        self.extensions
            .iter()
            .map(|extension| extension.icon_theme_names.len())
            .sum()
    }

    pub fn is_empty(&self) -> bool {
        self.ui_theme_count() == 0 && self.icon_theme_count() == 0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DetectedZedExtension {
    pub path: PathBuf,
    pub id: String,
    pub name: String,
    pub version: String,
    pub ui_theme_names: Vec<String>,
    pub icon_theme_names: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ZedThemeDetectionWarning {
    pub path: PathBuf,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportedZedIconTheme {
    pub extension_name: String,
    pub theme_names: Vec<String>,
    pub path: PathBuf,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ImportedZedThemes {
    pub ui_themes: Vec<ImportedZedTheme>,
    pub icon_themes: Vec<ImportedZedIconTheme>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ZedThemeImportConflictPolicy {
    #[default]
    SkipExisting,
    OverwriteExisting,
}

#[derive(Debug, Error)]
pub enum ZedThemeImportSummaryError {
    #[error("failed to import UI themes from Zed extension {extension:?}: {source}")]
    UiThemes {
        extension: String,
        source: ZedThemeImportError,
    },
    #[error("failed to import icon themes from Zed extension {extension:?}: {source}")]
    IconThemes {
        extension: String,
        source: ZedIconThemeImportError,
    },
    #[error("failed to install staged Zed themes: {source}")]
    Install { source: std::io::Error },
}

#[derive(Debug, Error)]
pub enum ZedIconThemeImportError {
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
    #[error("Zed extension {extension:?} does not contain icon themes")]
    NoIconThemes { extension: String },
    #[error("invalid Zed icon theme in {path}: {message}")]
    InvalidIconTheme { path: PathBuf, message: String },
    #[error("failed to create icon theme output directory {path}: {source}")]
    CreateOutputDirectory {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("icon theme package already exists: {path}")]
    OutputExists { path: PathBuf },
    #[error("icon theme output directory {path} must not be inside Zed extension {extension_root}")]
    OutputInsideExtension {
        path: PathBuf,
        extension_root: PathBuf,
    },
    #[error("failed to copy Zed icon theme from {source_path} to {destination}: {source}")]
    CopyPackage {
        source_path: PathBuf,
        destination: PathBuf,
        source: std::io::Error,
    },
    #[error("Zed icon theme package contains unsupported symbolic link {path}")]
    SymbolicLink { path: PathBuf },
}

pub fn detect_installed_zed_themes() -> ZedThemeDetection {
    let mut detection = ZedThemeDetection::default();
    let mut seen_roots = HashSet::new();
    for root in installed_extension_roots() {
        if !root.is_dir() {
            continue;
        }
        let canonical_root = root.canonicalize().unwrap_or(root);
        if !seen_roots.insert(canonical_root.clone()) {
            continue;
        }
        merge_detection(&mut detection, detect_zed_themes_in(canonical_root));
    }
    detection
        .extensions
        .sort_by(|left, right| left.name.cmp(&right.name).then(left.id.cmp(&right.id)));
    detection
}

pub fn detect_zed_theme_extension(extension_dir: impl AsRef<Path>) -> ZedThemeDetection {
    let requested = extension_dir.as_ref();
    let extension_path = match requested.canonicalize() {
        Ok(path) => path,
        Err(error) => {
            return ZedThemeDetection {
                warnings: vec![ZedThemeDetectionWarning {
                    path: requested.to_path_buf(),
                    message: error.to_string(),
                }],
                ..Default::default()
            };
        }
    };
    let Some(parent) = extension_path.parent() else {
        return ZedThemeDetection {
            warnings: vec![ZedThemeDetectionWarning {
                path: extension_path,
                message: "Zed extension directory has no parent".to_string(),
            }],
            ..Default::default()
        };
    };
    let mut detection = detect_zed_themes_in(parent);
    detection
        .extensions
        .retain(|extension| extension.path == extension_path);
    detection.warnings.retain(|warning| {
        warning.path == parent
            || warning.path == extension_path
            || warning.path.starts_with(&extension_path)
    });
    detection
}

pub fn detect_zed_themes_in(installed_extensions_dir: impl AsRef<Path>) -> ZedThemeDetection {
    let root = installed_extensions_dir.as_ref().to_path_buf();
    let mut detection = ZedThemeDetection {
        roots: vec![root.clone()],
        ..Default::default()
    };
    let entries = match fs::read_dir(&root) {
        Ok(entries) => entries,
        Err(error) => {
            detection.warnings.push(ZedThemeDetectionWarning {
                path: root,
                message: error.to_string(),
            });
            return detection;
        }
    };
    let mut extension_dirs = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            entry
                .file_type()
                .ok()
                .filter(|file_type| file_type.is_dir())
                .map(|_| entry.path())
        })
        .collect::<Vec<_>>();
    extension_dirs.sort();

    for extension_dir in extension_dirs {
        let manifest_path = extension_dir.join("extension.toml");
        let manifest_source = match fs::read_to_string(&manifest_path) {
            Ok(source) => source,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                detection.warnings.push(ZedThemeDetectionWarning {
                    path: manifest_path,
                    message: error.to_string(),
                });
                continue;
            }
        };
        let manifest: ZedExtensionManifest = match toml::from_str(&manifest_source) {
            Ok(manifest) => manifest,
            Err(error) => {
                detection.warnings.push(ZedThemeDetectionWarning {
                    path: manifest_path,
                    message: error.to_string(),
                });
                continue;
            }
        };
        if manifest.themes.is_empty() && manifest.icon_themes.is_empty() {
            continue;
        }

        let ui_theme_names = if manifest.themes.is_empty() {
            Vec::new()
        } else {
            match convert_zed_theme_extension(&extension_dir) {
                Ok(themes) => themes
                    .into_iter()
                    .map(|converted| converted.theme.name)
                    .collect(),
                Err(error) => {
                    detection.warnings.push(ZedThemeDetectionWarning {
                        path: extension_dir.clone(),
                        message: error.to_string(),
                    });
                    Vec::new()
                }
            }
        };
        let icon_theme_names = if manifest.icon_themes.is_empty() {
            Vec::new()
        } else {
            match icon_theme_names(&extension_dir, &manifest.icon_themes) {
                Ok(names) => names,
                Err(message) => {
                    detection.warnings.push(ZedThemeDetectionWarning {
                        path: extension_dir.clone(),
                        message,
                    });
                    Vec::new()
                }
            }
        };
        if ui_theme_names.is_empty() && icon_theme_names.is_empty() {
            continue;
        }
        detection.extensions.push(DetectedZedExtension {
            path: extension_dir,
            id: manifest.id,
            name: manifest.name,
            version: manifest.version,
            ui_theme_names,
            icon_theme_names,
        });
    }

    detection
}

pub fn import_detected_zed_themes(
    detection: &ZedThemeDetection,
    config_paths: &AppConfigPaths,
) -> Result<ImportedZedThemes, ZedThemeImportSummaryError> {
    import_detected_zed_themes_to(
        detection,
        config_paths.themes_dir(),
        config_paths.icon_themes_dir(),
    )
}

pub fn import_detected_zed_themes_to(
    detection: &ZedThemeDetection,
    ui_theme_output_dir: impl AsRef<Path>,
    icon_theme_output_dir: impl AsRef<Path>,
) -> Result<ImportedZedThemes, ZedThemeImportSummaryError> {
    let ui_theme_output_dir = ui_theme_output_dir.as_ref();
    let icon_theme_output_dir = icon_theme_output_dir.as_ref();
    let mut imported = ImportedZedThemes::default();
    let mut created_ui_files = Vec::new();
    let mut created_icon_packages = Vec::new();

    for extension in &detection.extensions {
        if !extension.ui_theme_names.is_empty() {
            match import_zed_theme_extension(&extension.path, ui_theme_output_dir) {
                Ok(themes) => {
                    created_ui_files.extend(themes.iter().map(|theme| theme.path.clone()));
                    imported.ui_themes.extend(themes);
                }
                Err(source) => {
                    rollback_imports(&created_ui_files, &created_icon_packages);
                    return Err(ZedThemeImportSummaryError::UiThemes {
                        extension: extension.name.clone(),
                        source,
                    });
                }
            }
        }
        if !extension.icon_theme_names.is_empty() {
            match import_zed_icon_theme_extension(&extension.path, icon_theme_output_dir) {
                Ok(icon_theme) => {
                    created_icon_packages.push(icon_theme.path.clone());
                    imported.icon_themes.push(icon_theme);
                }
                Err(source) => {
                    rollback_imports(&created_ui_files, &created_icon_packages);
                    return Err(ZedThemeImportSummaryError::IconThemes {
                        extension: extension.name.clone(),
                        source,
                    });
                }
            }
        }
    }

    Ok(imported)
}

pub fn import_detected_zed_themes_with_policy(
    detection: &ZedThemeDetection,
    config_paths: &AppConfigPaths,
    conflict_policy: ZedThemeImportConflictPolicy,
) -> Result<ImportedZedThemes, ZedThemeImportSummaryError> {
    import_detected_zed_themes_to_with_policy(
        detection,
        config_paths.themes_dir(),
        config_paths.icon_themes_dir(),
        conflict_policy,
    )
}

pub fn import_detected_zed_themes_to_with_policy(
    detection: &ZedThemeDetection,
    ui_theme_output_dir: impl AsRef<Path>,
    icon_theme_output_dir: impl AsRef<Path>,
    conflict_policy: ZedThemeImportConflictPolicy,
) -> Result<ImportedZedThemes, ZedThemeImportSummaryError> {
    let ui_theme_output_dir = ui_theme_output_dir.as_ref();
    let icon_theme_output_dir = icon_theme_output_dir.as_ref();
    let staging_name = format!(".zed-theme-import-{}", uuid::Uuid::new_v4());
    let ui_staging_dir = ui_theme_output_dir.join(&staging_name);
    let icon_staging_dir = icon_theme_output_dir.join(&staging_name);
    let staged = match import_detected_zed_themes_to(detection, &ui_staging_dir, &icon_staging_dir)
    {
        Ok(staged) => staged,
        Err(error) => {
            cleanup_staging_dirs(&ui_staging_dir, &icon_staging_dir);
            return Err(error);
        }
    };

    let mut applied = Vec::new();
    let mut imported = ImportedZedThemes::default();
    let commit_result = (|| -> std::io::Result<()> {
        for theme in staged.ui_themes {
            let destination = staged_import_destination(ui_theme_output_dir, &theme.path)?;
            if install_staged_import(
                &theme.path,
                &destination,
                &ui_staging_dir,
                conflict_policy,
                &mut applied,
            )? {
                imported.ui_themes.push(ImportedZedTheme {
                    theme_name: theme.theme_name,
                    path: destination,
                });
            }
        }
        for package in staged.icon_themes {
            let destination = staged_import_destination(icon_theme_output_dir, &package.path)?;
            if install_staged_import(
                &package.path,
                &destination,
                &icon_staging_dir,
                conflict_policy,
                &mut applied,
            )? {
                imported.icon_themes.push(ImportedZedIconTheme {
                    extension_name: package.extension_name,
                    theme_names: package.theme_names,
                    path: destination,
                });
            }
        }
        Ok(())
    })();

    if let Err(source) = commit_result {
        rollback_applied_imports(&applied);
        cleanup_staging_dirs(&ui_staging_dir, &icon_staging_dir);
        return Err(ZedThemeImportSummaryError::Install { source });
    }

    cleanup_staging_dirs(&ui_staging_dir, &icon_staging_dir);
    Ok(imported)
}

fn staged_import_destination(output_dir: &Path, staged_path: &Path) -> std::io::Result<PathBuf> {
    let file_name = staged_path.file_name().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!(
                "staged import path has no file name: {}",
                staged_path.display()
            ),
        )
    })?;
    Ok(output_dir.join(file_name))
}

fn install_staged_import(
    staged_path: &Path,
    destination: &Path,
    backup_root: &Path,
    conflict_policy: ZedThemeImportConflictPolicy,
    applied: &mut Vec<AppliedZedImport>,
) -> std::io::Result<bool> {
    if destination.exists() && conflict_policy == ZedThemeImportConflictPolicy::SkipExisting {
        return Ok(false);
    }

    let backup = if destination.exists() {
        let backup = backup_root.join(format!(".backup-{}", applied.len()));
        fs::rename(destination, &backup)?;
        Some(backup)
    } else {
        None
    };

    if let Err(error) = fs::rename(staged_path, destination) {
        if let Some(backup) = &backup {
            let _ = fs::rename(backup, destination);
        }
        return Err(error);
    }
    applied.push(AppliedZedImport {
        destination: destination.to_path_buf(),
        backup,
    });
    Ok(true)
}

fn rollback_applied_imports(applied: &[AppliedZedImport]) {
    for import in applied.iter().rev() {
        remove_import_path(&import.destination);
        if let Some(backup) = &import.backup {
            let _ = fs::rename(backup, &import.destination);
        }
    }
}

fn cleanup_staging_dirs(ui_staging_dir: &Path, icon_staging_dir: &Path) {
    let _ = fs::remove_dir_all(ui_staging_dir);
    let _ = fs::remove_dir_all(icon_staging_dir);
}

fn remove_import_path(path: &Path) {
    if path.is_dir() {
        let _ = fs::remove_dir_all(path);
    } else {
        let _ = fs::remove_file(path);
    }
}

struct AppliedZedImport {
    destination: PathBuf,
    backup: Option<PathBuf>,
}

pub fn import_zed_icon_theme_extension(
    extension_dir: impl AsRef<Path>,
    output_dir: impl AsRef<Path>,
) -> Result<ImportedZedIconTheme, ZedIconThemeImportError> {
    let requested_root = extension_dir.as_ref();
    let extension_root = requested_root.canonicalize().map_err(|source| {
        ZedIconThemeImportError::ResolveExtensionDirectory {
            path: requested_root.to_path_buf(),
            source,
        }
    })?;
    let manifest_path = extension_root.join("extension.toml");
    let manifest_source = fs::read_to_string(&manifest_path).map_err(|source| {
        ZedIconThemeImportError::ReadManifest {
            path: manifest_path.clone(),
            source,
        }
    })?;
    let manifest: ZedExtensionManifest = toml::from_str(&manifest_source).map_err(|source| {
        ZedIconThemeImportError::ParseManifest {
            path: manifest_path,
            source,
        }
    })?;
    if manifest.icon_themes.is_empty() {
        return Err(ZedIconThemeImportError::NoIconThemes {
            extension: manifest.name,
        });
    }
    let theme_names =
        icon_theme_names(&extension_root, &manifest.icon_themes).map_err(|message| {
            ZedIconThemeImportError::InvalidIconTheme {
                path: extension_root.clone(),
                message,
            }
        })?;

    let output_dir = output_dir.as_ref();
    fs::create_dir_all(output_dir).map_err(|source| {
        ZedIconThemeImportError::CreateOutputDirectory {
            path: output_dir.to_path_buf(),
            source,
        }
    })?;
    let output_root = output_dir.canonicalize().map_err(|source| {
        ZedIconThemeImportError::CreateOutputDirectory {
            path: output_dir.to_path_buf(),
            source,
        }
    })?;
    if output_root.starts_with(&extension_root) {
        return Err(ZedIconThemeImportError::OutputInsideExtension {
            path: output_root,
            extension_root,
        });
    }

    let destination = output_root.join(slugify(&manifest.id));
    if destination.exists() {
        return Err(ZedIconThemeImportError::OutputExists { path: destination });
    }
    let staging = output_root.join(format!(
        ".{}.importing-{}",
        slugify(&manifest.id),
        uuid::Uuid::new_v4()
    ));
    let copy_result = copy_directory(&extension_root, &staging).and_then(|()| {
        fs::rename(&staging, &destination).map_err(|source| ZedIconThemeImportError::CopyPackage {
            source_path: extension_root.clone(),
            destination: destination.clone(),
            source,
        })
    });
    if let Err(error) = copy_result {
        let _ = fs::remove_dir_all(&staging);
        return Err(error);
    }

    Ok(ImportedZedIconTheme {
        extension_name: manifest.name,
        theme_names,
        path: destination,
    })
}

pub fn zed_icon_theme_output_path(extension_id: &str, output_dir: impl AsRef<Path>) -> PathBuf {
    output_dir.as_ref().join(slugify(extension_id))
}

fn installed_extension_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    #[cfg(target_os = "macos")]
    if let Some(home) = env::var_os("HOME").map(PathBuf::from) {
        roots.push(
            home.join("Library")
                .join("Application Support")
                .join("Zed")
                .join("extensions")
                .join("installed"),
        );
        roots.push(
            home.join("Library")
                .join("Application Support")
                .join("Zed Preview")
                .join("extensions")
                .join("installed"),
        );
    }
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    {
        if let Some(data_home) = env::var_os("XDG_DATA_HOME").map(PathBuf::from) {
            roots.push(data_home.join("zed").join("extensions").join("installed"));
            roots.push(
                data_home
                    .join("zed-preview")
                    .join("extensions")
                    .join("installed"),
            );
        } else if let Some(home) = env::var_os("HOME").map(PathBuf::from) {
            roots.push(
                home.join(".local")
                    .join("share")
                    .join("zed")
                    .join("extensions")
                    .join("installed"),
            );
            roots.push(
                home.join(".local")
                    .join("share")
                    .join("zed-preview")
                    .join("extensions")
                    .join("installed"),
            );
        }
    }
    #[cfg(target_os = "windows")]
    if let Some(local_app_data) = env::var_os("LOCALAPPDATA").map(PathBuf::from) {
        roots.push(
            local_app_data
                .join("Zed")
                .join("extensions")
                .join("installed"),
        );
        roots.push(
            local_app_data
                .join("Zed Preview")
                .join("extensions")
                .join("installed"),
        );
    }
    roots
}

fn merge_detection(target: &mut ZedThemeDetection, mut source: ZedThemeDetection) {
    target.roots.append(&mut source.roots);
    target.extensions.append(&mut source.extensions);
    target.warnings.append(&mut source.warnings);
}

fn icon_theme_names(
    extension_root: &Path,
    icon_theme_files: &[String],
) -> Result<Vec<String>, String> {
    let canonical_root = extension_root
        .canonicalize()
        .map_err(|error| format!("failed to resolve extension directory: {error}"))?;
    let mut names = Vec::new();
    for relative_path in icon_theme_files {
        let unresolved = canonical_root.join(relative_path);
        let path = unresolved
            .canonicalize()
            .map_err(|error| format!("failed to resolve {}: {error}", unresolved.display()))?;
        if !path.starts_with(&canonical_root) {
            return Err(format!(
                "icon theme path {} escapes extension directory {}",
                path.display(),
                canonical_root.display()
            ));
        }
        let source = fs::read_to_string(&path)
            .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
        let family: ZedIconThemeFamily = serde_json::from_str(&source)
            .map_err(|error| format!("failed to parse {}: {error}", path.display()))?;
        if family.themes.is_empty() && !family.name.trim().is_empty() {
            names.push(family.name);
        } else {
            names.extend(
                family
                    .themes
                    .into_iter()
                    .map(|theme| theme.name)
                    .filter(|name| !name.trim().is_empty()),
            );
        }
    }
    names.sort();
    names.dedup();
    Ok(names)
}

fn copy_directory(
    source_root: &Path,
    destination_root: &Path,
) -> Result<(), ZedIconThemeImportError> {
    fs::create_dir(destination_root).map_err(|source| ZedIconThemeImportError::CopyPackage {
        source_path: source_root.to_path_buf(),
        destination: destination_root.to_path_buf(),
        source,
    })?;
    let entries =
        fs::read_dir(source_root).map_err(|source| ZedIconThemeImportError::CopyPackage {
            source_path: source_root.to_path_buf(),
            destination: destination_root.to_path_buf(),
            source,
        })?;
    let mut entries = entries.filter_map(Result::ok).collect::<Vec<_>>();
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let source_path = entry.path();
        let destination = destination_root.join(entry.file_name());
        let file_type =
            entry
                .file_type()
                .map_err(|source| ZedIconThemeImportError::CopyPackage {
                    source_path: source_path.clone(),
                    destination: destination.clone(),
                    source,
                })?;
        if file_type.is_symlink() {
            return Err(ZedIconThemeImportError::SymbolicLink { path: source_path });
        }
        if file_type.is_dir() {
            copy_directory(&source_path, &destination)?;
        } else if file_type.is_file() {
            fs::copy(&source_path, &destination).map_err(|source| {
                ZedIconThemeImportError::CopyPackage {
                    source_path,
                    destination,
                    source,
                }
            })?;
        }
    }
    Ok(())
}

fn rollback_imports(ui_files: &[PathBuf], icon_packages: &[PathBuf]) {
    for path in ui_files {
        let _ = fs::remove_file(path);
    }
    for path in icon_packages {
        let _ = fs::remove_dir_all(path);
    }
}

#[derive(Debug, Deserialize)]
struct ZedIconThemeFamily {
    name: String,
    #[serde(default)]
    themes: Vec<ZedIconTheme>,
}

#[derive(Debug, Deserialize)]
struct ZedIconTheme {
    name: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{config::theme::load_theme_store, ui::theme::icons::available_icon_theme_names};

    #[test]
    fn detects_and_imports_compatible_ui_and_icon_themes() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let installed_root = temp.path().join("installed");
        let extension_dir = installed_root.join("test-pack");
        write_combined_extension(&extension_dir);

        let detection = detect_zed_themes_in(&installed_root);

        assert!(detection.warnings.is_empty());
        assert_eq!(detection.ui_theme_count(), 1);
        assert_eq!(detection.icon_theme_count(), 1);
        assert_eq!(detection.extensions.len(), 1);
        assert_eq!(detection.extensions[0].name, "Test Theme Pack");
        assert_eq!(detection.extensions[0].ui_theme_names, ["Test UI"]);
        assert_eq!(
            detection.extensions[0].icon_theme_names,
            ["Test File Icons"]
        );

        let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
        let imported =
            import_detected_zed_themes(&detection, &paths).expect("detected themes import");

        assert_eq!(imported.ui_themes.len(), 1);
        assert_eq!(imported.icon_themes.len(), 1);
        assert!(paths.themes_dir().join("test-pack-test-ui.toml").is_file());
        assert!(
            paths
                .icon_themes_dir()
                .join("test-pack/icons/rust.svg")
                .is_file()
        );
        let loaded = load_theme_store(&paths).expect("theme store loads");
        assert!(loaded.store.theme("Test UI").is_some());
        assert_eq!(
            available_icon_theme_names(&paths).expect("icon themes load"),
            ["Test File Icons"]
        );
    }

    #[test]
    fn conflict_policy_skips_or_overwrites_existing_themes() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let installed_root = temp.path().join("installed");
        write_combined_extension(&installed_root.join("test-pack"));
        let detection = detect_zed_themes_in(&installed_root);
        let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
        let first = import_detected_zed_themes(&detection, &paths).expect("first import succeeds");
        let ui_path = first.ui_themes[0].path.clone();
        let icon_path = first.icon_themes[0].path.join("icons/rust.svg");
        fs::write(&ui_path, "custom ui theme").expect("customize imported UI theme");
        fs::write(&icon_path, "custom icon").expect("customize imported icon");

        let skipped = import_detected_zed_themes_with_policy(
            &detection,
            &paths,
            ZedThemeImportConflictPolicy::SkipExisting,
        )
        .expect("skipping existing themes succeeds");

        assert!(skipped.ui_themes.is_empty());
        assert!(skipped.icon_themes.is_empty());
        assert_eq!(
            fs::read_to_string(&ui_path).expect("read skipped UI theme"),
            "custom ui theme"
        );
        assert_eq!(
            fs::read_to_string(&icon_path).expect("read skipped icon"),
            "custom icon"
        );

        let overwritten = import_detected_zed_themes_with_policy(
            &detection,
            &paths,
            ZedThemeImportConflictPolicy::OverwriteExisting,
        )
        .expect("overwriting existing themes succeeds");

        assert_eq!(overwritten.ui_themes.len(), 1);
        assert_eq!(overwritten.icon_themes.len(), 1);
        assert_ne!(
            fs::read_to_string(&ui_path).expect("read overwritten UI theme"),
            "custom ui theme"
        );
        assert_eq!(
            fs::read_to_string(&icon_path).expect("read overwritten icon"),
            "<svg xmlns=\"http://www.w3.org/2000/svg\"></svg>"
        );
    }

    #[test]
    fn icon_import_refuses_to_replace_an_existing_package() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let extension_dir = temp.path().join("test-pack");
        write_combined_extension(&extension_dir);
        let output_dir = temp.path().join("icons");
        let first = import_zed_icon_theme_extension(&extension_dir, &output_dir)
            .expect("first import succeeds");

        let error = import_zed_icon_theme_extension(&extension_dir, &output_dir)
            .expect_err("second import must not overwrite the package");

        assert!(matches!(
            error,
            ZedIconThemeImportError::OutputExists { path } if path == first.path
        ));
        assert!(first.path.join("icons/rust.svg").is_file());
    }

    fn write_combined_extension(extension_dir: &Path) {
        fs::create_dir_all(extension_dir.join("themes")).expect("UI themes directory");
        fs::create_dir_all(extension_dir.join("icon_themes")).expect("icon themes directory");
        fs::create_dir_all(extension_dir.join("icons")).expect("icons directory");
        fs::write(
            extension_dir.join("extension.toml"),
            r#"
id = "test-pack"
name = "Test Theme Pack"
version = "1.0.0"
description = "Combined theme pack"
repository = "https://github.com/example/test-pack"
authors = ["Ada"]
themes = ["themes/ui.json"]
icon_themes = ["icon_themes/icons.json"]
"#,
        )
        .expect("extension manifest");
        fs::write(
            extension_dir.join("themes/ui.json"),
            r##"{
  "name": "Test UI Family",
  "author": "Ada",
  "themes": [
    {
      "name": "Test UI",
      "appearance": "dark",
      "style": {
        "background": "#101010",
        "editor.foreground": "#eeeeee"
      }
    }
  ]
}"##,
        )
        .expect("UI theme");
        fs::write(
            extension_dir.join("icon_themes/icons.json"),
            r#"{
  "name": "Test Icon Family",
  "themes": [
    {
      "name": "Test File Icons",
      "file_suffixes": {
        "rs": "rust"
      },
      "file_icons": {
        "rust": {
          "path": "./icons/rust.svg"
        }
      }
    }
  ]
}"#,
        )
        .expect("icon theme");
        fs::write(
            extension_dir.join("icons/rust.svg"),
            "<svg xmlns=\"http://www.w3.org/2000/svg\"></svg>",
        )
        .expect("icon asset");
    }
}
