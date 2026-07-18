use std::{
    borrow::Cow,
    fs,
    path::{Path, PathBuf},
};

use anyhow::anyhow;
use gpui::{AssetSource, Result, SharedString};

use crate::config::paths::AppConfigPaths;

pub const REQUIRED_COMPONENT_ICON_ASSET_PATHS: &[&str] = &["icons/search.svg"];
pub(crate) const EXTERNAL_ICON_ASSET_PREFIX: &str = "yttt-icon://";
pub const BUILTIN_APP_ICON_ASSET_PATH: &str = "app-icon/yttt.png";
pub const BUILTIN_FILE_ICON_ASSET_PATHS: &[&str] = &[
    "icons/file-csharp.svg",
    "icons/file-powershell.svg",
    "icons/file-windows-project.svg",
    "icons/file-xml.svg",
];

fn builtin_asset(path: &str) -> Option<&'static [u8]> {
    match path {
        BUILTIN_APP_ICON_ASSET_PATH => {
            Some(include_bytes!("../../../assets/app-icon/png/256.png").as_slice())
        }
        "icons/file-csharp.svg" => {
            Some(include_bytes!("../../../assets/icons/file-csharp.svg").as_slice())
        }
        "icons/file-powershell.svg" => {
            Some(include_bytes!("../../../assets/icons/file-powershell.svg").as_slice())
        }
        "icons/file-windows-project.svg" => {
            Some(include_bytes!("../../../assets/icons/file-windows-project.svg").as_slice())
        }
        "icons/file-xml.svg" => {
            Some(include_bytes!("../../../assets/icons/file-xml.svg").as_slice())
        }
        _ => None,
    }
}

pub struct YtttAssets {
    icon_themes_root: PathBuf,
}

impl AssetSource for YtttAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if let Some(relative_path) = path.strip_prefix(EXTERNAL_ICON_ASSET_PREFIX) {
            let root = self
                .icon_themes_root
                .canonicalize()
                .unwrap_or_else(|_| self.icon_themes_root.clone());
            let resolved = root
                .join(Path::new(relative_path))
                .canonicalize()
                .map_err(|error| {
                    anyhow!("failed to resolve external icon {relative_path:?}: {error}")
                })?;
            if !resolved.starts_with(&root) {
                return Err(anyhow!(
                    "external icon path escapes the configured icon theme root"
                ));
            }
            return Ok(Some(Cow::Owned(fs::read(resolved)?)));
        }

        if let Some(asset) = builtin_asset(path) {
            return Ok(Some(Cow::Borrowed(asset)));
        }

        gpui_component_assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut assets = gpui_component_assets::Assets.list(path)?;
        assets.extend(
            BUILTIN_FILE_ICON_ASSET_PATHS
                .iter()
                .filter(|asset_path| asset_path.starts_with(path))
                .map(|asset_path| (*asset_path).into()),
        );
        if BUILTIN_APP_ICON_ASSET_PATH.starts_with(path) {
            assets.push(BUILTIN_APP_ICON_ASSET_PATH.into());
        }
        Ok(assets)
    }
}

pub fn app_assets(config_paths: &AppConfigPaths) -> YtttAssets {
    YtttAssets {
        icon_themes_root: config_paths.icon_themes_dir(),
    }
}

pub(crate) fn external_icon_asset_path(relative_path: &Path) -> SharedString {
    format!(
        "{EXTERNAL_ICON_ASSET_PREFIX}{}",
        relative_path.to_string_lossy()
    )
    .into()
}
