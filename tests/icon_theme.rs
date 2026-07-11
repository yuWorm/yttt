use std::{fs, path::Path};

use gpui::AssetSource;
use tempfile::tempdir;
use yttt::{
    config::paths::AppConfigPaths,
    ui::{
        app::assets::app_assets,
        theme::icons::{IconVisual, available_icon_theme_names, load_icon_theme},
    },
};

const THEME_JSON: &str = r#"
{
  "name": "Fixture family",
  "themes": [{
    "name": "Fixture dark",
    "directory_icons": {
      "collapsed": "icons/folder.svg",
      "expanded": "icons/folder-open.svg"
    },
    "named_directory_icons": {
      "src": {
        "collapsed": "icons/src.svg",
        "expanded": "icons/src-open.svg"
      }
    },
    "chevron_icons": {
      "collapsed": "icons/chevron-right.svg",
      "expanded": "icons/chevron-down.svg"
    },
    "file_stems": { "Cargo.toml": "toml" },
    "file_suffixes": { "rs": "rust", "config.js": "javascript" },
    "file_icons": {
      "default": { "path": "icons/default.svg" },
      "rust": { "path": "icons/rust.svg" },
      "toml": { "path": "icons/toml.svg" },
      "javascript": { "path": "icons/javascript.svg" }
    }
  }]
}
"#;

#[test]
fn zed_compatible_icon_theme_resolves_icons_and_loads_svg_assets() {
    let temp = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path());
    let package = paths.icon_themes_dir().join("fixture-theme");
    fs::create_dir_all(package.join("icon_themes")).unwrap();
    fs::create_dir_all(package.join("icons")).unwrap();
    fs::write(package.join("icon_themes/fixture.json"), THEME_JSON).unwrap();
    for name in [
        "default",
        "rust",
        "toml",
        "javascript",
        "folder",
        "folder-open",
        "src",
        "src-open",
        "chevron-right",
        "chevron-down",
    ] {
        fs::write(
            package.join(format!("icons/{name}.svg")),
            format!("<svg id=\"{name}\" />"),
        )
        .unwrap();
    }

    assert_eq!(
        available_icon_theme_names(&paths).unwrap(),
        vec!["Fixture dark"]
    );
    assert!(matches!(
        load_icon_theme(&paths, None)
            .unwrap()
            .resolve_file(Path::new("src/lib.rs")),
        IconVisual::Component(_)
    ));
    let theme = load_icon_theme(&paths, Some("Fixture dark")).unwrap();

    assert_asset_path(
        theme.resolve_file(Path::new("src/lib.rs")),
        "fixture-theme/icons/rust.svg",
    );
    assert_asset_path(
        theme.resolve_file(Path::new("Cargo.toml")),
        "fixture-theme/icons/toml.svg",
    );
    assert_asset_path(
        theme.resolve_file(Path::new("eslint.config.js")),
        "fixture-theme/icons/javascript.svg",
    );
    assert_asset_path(
        theme.resolve_directory(Path::new("src"), false),
        "fixture-theme/icons/src.svg",
    );
    assert_asset_path(
        theme.resolve_directory(Path::new("docs"), true),
        "fixture-theme/icons/folder-open.svg",
    );
    let rust_icon = theme.resolve_file(Path::new("src/lib.rs"));
    assert_asset_path(
        theme.resolve_chevron(true),
        "fixture-theme/icons/chevron-down.svg",
    );

    let IconVisual::Asset(rust_asset) = rust_icon else {
        panic!("Rust file should resolve to an external SVG asset");
    };
    let asset = app_assets(&paths).load(&rust_asset).unwrap();
    assert_eq!(asset.as_deref(), Some(b"<svg id=\"rust\" />".as_slice()));
}

#[test]
fn icon_theme_rejects_asset_paths_outside_its_package() {
    let temp = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path());
    let package = paths.icon_themes_dir().join("fixture-theme");
    fs::create_dir_all(package.join("icon_themes")).unwrap();
    fs::write(
        package.join("icon_themes/fixture.json"),
        THEME_JSON.replace("icons/rust.svg", "../../outside.svg"),
    )
    .unwrap();
    fs::write(paths.themes_dir().join("outside.svg"), "<svg />").unwrap();

    let theme = load_icon_theme(&paths, Some("Fixture dark")).unwrap();
    assert!(matches!(
        theme.resolve_file(Path::new("src/lib.rs")),
        IconVisual::Component(_)
    ));
    let error = app_assets(&paths)
        .load("yttt-icon://fixture-theme/../../outside.svg")
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("escapes the configured icon theme root")
    );
}

fn assert_asset_path(visual: IconVisual, expected_suffix: &str) {
    let IconVisual::Asset(path) = visual else {
        panic!("expected an external icon asset");
    };
    assert_eq!(path, format!("yttt-icon://{expected_suffix}"));
}
