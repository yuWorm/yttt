use gpui::AssetSource;

#[test]
fn app_assets_include_gpui_component_icons_used_by_ui() {
    for path in yttt::ui::assets::REQUIRED_COMPONENT_ICON_ASSET_PATHS {
        let asset = yttt::ui::assets::app_assets()
            .load(path)
            .unwrap_or_else(|err| panic!("load {path}: {err}"));

        assert!(asset.is_some(), "missing component icon asset: {path}");
    }
}
