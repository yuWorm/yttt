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

#[test]
#[cfg(target_os = "macos")]
fn prepare_macos_app_runtime_disables_automatic_window_tabbing() {
    use cocoa::{
        appkit::NSWindow,
        base::{NO, id, nil},
    };

    unsafe {
        <id as NSWindow>::setAllowsAutomaticWindowTabbing_(nil, cocoa::base::YES);
    }

    yttt::ui::macos::prepare_macos_app_runtime();

    let allows_tabbing = unsafe { <id as NSWindow>::allowsAutomaticWindowTabbing(nil) };
    assert_eq!(allows_tabbing, NO);
}
