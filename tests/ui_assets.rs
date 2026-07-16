use gpui::AssetSource;
use tempfile::tempdir;
use yttt::config::paths::AppConfigPaths;

#[test]
fn app_assets_include_gpui_component_icons_used_by_ui() {
    let temp = tempdir().unwrap();
    let config_paths = AppConfigPaths::from_config_dir(temp.path());
    let assets = yttt::ui::app::assets::app_assets(&config_paths);

    for path in yttt::ui::app::assets::REQUIRED_COMPONENT_ICON_ASSET_PATHS {
        let asset = assets
            .load(path)
            .unwrap_or_else(|err| panic!("load {path}: {err}"));

        assert!(asset.is_some(), "missing component icon asset: {path}");
    }
}

#[test]
fn app_assets_include_builtin_file_icons() {
    let temp = tempdir().unwrap();
    let config_paths = AppConfigPaths::from_config_dir(temp.path());
    let assets = yttt::ui::app::assets::app_assets(&config_paths);

    for path in yttt::ui::app::assets::BUILTIN_FILE_ICON_ASSET_PATHS {
        let asset = assets
            .load(path)
            .unwrap_or_else(|err| panic!("load {path}: {err}"));

        assert!(asset.is_some(), "missing built-in file icon asset: {path}");
    }
}
