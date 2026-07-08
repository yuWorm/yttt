#[test]
#[cfg(target_os = "macos")]
fn fallback_app_icon_path_points_to_existing_icns() {
    let path = yttt::ui::macos::fallback_app_icon_path();

    assert!(path.ends_with(".icns"));
    assert!(
        std::path::Path::new(path).is_file(),
        "fallback app icon should exist at {path}"
    );
}
