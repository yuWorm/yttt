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

        gpui_component_assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        gpui_component_assets::Assets.list(path)
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
